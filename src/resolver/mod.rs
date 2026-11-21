//! Official-source registry and resolver dispatch.
//!
//! Catalogs are never hand-written: resolvers fetch official index pages and
//! turn links into typed entries. Parser functions are public and tested with
//! saved HTML fixtures; live fetching is confined to [`resolve_source`].

pub mod cbse_academic;
pub mod generic_index;
pub mod kseab;
pub mod ncert;
pub mod pue;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Client;
use rust_embed::RustEmbed;
use scraper::Html;
use serde::Deserialize;
use tokio::time::sleep;
use url::Url;

use crate::catalog::{Entry, MaterialType};
use crate::config::Language;
use crate::error::{ResolverError, ResolverResult};

/// Largest index page accepted from a portal.
const MAX_HTML_BYTES: usize = 6 * 1024 * 1024;

#[derive(RustEmbed)]
#[folder = "sources/"]
struct EmbeddedSources;

/// Whether a source has been manually confirmed to work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceStatus {
    /// Index and file URLs have been confirmed.
    Verified,
    /// Official source, but its index has not been confirmed yet.
    Unverified,
}

impl SourceStatus {
    /// Stable label for reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
        }
    }
}

/// Resolver implementation selected by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolverKind {
    /// `ncert.nic.in/textbook.php`.
    Ncert,
    /// `cbseacademic.nic.in` sample-paper pages.
    CbseAcademic,
    /// KSEAB/KSEEB SSLC question-paper pages.
    Kseab,
    /// Karnataka Pre-University textbook page.
    Pue,
    /// Generic same-host PDF index used for unverified sources.
    GenericIndex,
}

/// One curated official source.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceDefinition {
    /// Stable id used by `crawl --source`.
    pub id: String,
    /// Display name used as the catalog `source` label.
    pub name: String,
    /// Resolver implementation.
    pub resolver: ResolverKind,
    /// Board id written into generated catalogs.
    pub board: String,
    /// Index URL fetched by the resolver.
    pub base_url: String,
    /// Levels covered by the source.
    pub levels: Vec<String>,
    /// Inclusive publication/exam year range.
    pub year_range: [u16; 2],
    /// Verification status.
    pub status: SourceStatus,
    /// Optional keyword filter for `generic_index`.
    #[serde(default)]
    pub include_terms: Vec<String>,
    /// Maximum pages fetched by index-following resolvers.
    #[serde(default = "default_max_pages")]
    pub max_pages: usize,
}

impl SourceDefinition {
    /// Years from newest to oldest.
    pub fn years(&self) -> impl DoubleEndedIterator<Item = u16> {
        (self.year_range[0]..=self.year_range[1]).rev()
    }

    /// Parsed, validated base URL.
    pub fn base_url(&self) -> ResolverResult<Url> {
        let url = Url::parse(&self.base_url).map_err(|error| {
            ResolverError::Invalid(format!(
                "source {} has an invalid base_url: {error}",
                self.id
            ))
        })?;

        if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
            Ok(url)
        } else {
            Err(ResolverError::Invalid(format!(
                "source {} base_url must be absolute http(s)",
                self.id
            )))
        }
    }
}

fn default_max_pages() -> usize {
    1
}

/// Curated source registry.
#[derive(Debug, Clone)]
pub struct Registry {
    /// Sources in file order.
    pub sources: Vec<SourceDefinition>,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(rename = "source", alias = "sources", default)]
    sources: Vec<SourceDefinition>,
}

impl Registry {
    /// Load an explicit path, a local `sources/registry.toml`, or the embedded
    /// registry, in that order.
    pub fn load(path: Option<&Path>) -> ResolverResult<Self> {
        if let Some(path) = path {
            let contents = fs::read_to_string(path).map_err(|source| ResolverError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            return Self::from_toml(path.display().to_string(), &contents);
        }

        let local = PathBuf::from("sources/registry.toml");
        if local.is_file() {
            let contents = fs::read_to_string(&local).map_err(|source| ResolverError::Read {
                path: local.clone(),
                source,
            })?;
            return Self::from_toml(local.display().to_string(), &contents);
        }

        let embedded =
            EmbeddedSources::get("registry.toml").ok_or(ResolverError::EmbeddedMissing)?;
        let contents = std::str::from_utf8(embedded.data.as_ref()).map_err(|_| {
            ResolverError::Invalid("embedded registry is not valid UTF-8".to_owned())
        })?;
        Self::from_toml("embedded:registry.toml", contents)
    }

    /// Parse and validate a registry from TOML.
    pub fn from_toml(origin: impl Into<String>, contents: &str) -> ResolverResult<Self> {
        let origin = origin.into();
        let file: RegistryFile =
            toml::from_str(contents).map_err(|source| ResolverError::Toml {
                origin: origin.clone(),
                source,
            })?;

        let registry = Self {
            sources: file.sources,
        };
        registry.validate()?;
        Ok(registry)
    }

    /// Select sources for a crawl.
    ///
    /// With no explicit ids, verified sources are selected unless
    /// `include_unverified` is set. Explicitly named sources are always
    /// selected, even if unverified.
    pub fn select(
        &self,
        ids: &[String],
        include_unverified: bool,
    ) -> ResolverResult<Vec<SourceDefinition>> {
        if ids.is_empty() {
            return Ok(self
                .sources
                .iter()
                .filter(|source| include_unverified || source.status == SourceStatus::Verified)
                .cloned()
                .collect());
        }

        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            let source = self
                .sources
                .iter()
                .find(|source| source.id == *id)
                .ok_or_else(|| {
                    ResolverError::Invalid(format!(
                        "unknown source id `{id}`; available: {}",
                        self.sources
                            .iter()
                            .map(|source| source.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?;

            if seen.insert(source.id.clone()) {
                selected.push(source.clone());
            }
        }
        Ok(selected)
    }

    /// Find a source by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&SourceDefinition> {
        self.sources.iter().find(|source| source.id == id)
    }

    fn validate(&self) -> ResolverResult<()> {
        if self.sources.is_empty() {
            return Err(ResolverError::Invalid(
                "registry contains no sources".to_owned(),
            ));
        }

        let mut ids = HashSet::new();
        for source in &self.sources {
            if source.id.trim().is_empty() {
                return Err(ResolverError::Invalid(
                    "source id must not be empty".to_owned(),
                ));
            }

            if !ids.insert(source.id.clone()) {
                return Err(ResolverError::Invalid(format!(
                    "duplicate source id `{}`",
                    source.id
                )));
            }

            if !source.id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            }) {
                return Err(ResolverError::Invalid(format!(
                    "source id `{}` may only contain ASCII letters, digits, '-' and '_'",
                    source.id
                )));
            }

            if !source.board.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            }) {
                return Err(ResolverError::Invalid(format!(
                    "source `{}` has an invalid board id `{}`",
                    source.id, source.board
                )));
            }

            source.base_url()?;

            if source.levels.is_empty() || source.levels.iter().any(|level| level.trim().is_empty())
            {
                return Err(ResolverError::Invalid(format!(
                    "source `{}` must declare at least one level",
                    source.id
                )));
            }

            let [start, end] = source.year_range;
            if start > end || !(1900..=2100).contains(&start) || !(1900..=2100).contains(&end) {
                return Err(ResolverError::Invalid(format!(
                    "source `{}` has an invalid year range",
                    source.id
                )));
            }

            if source.max_pages == 0 || source.max_pages > 200 {
                return Err(ResolverError::Invalid(format!(
                    "source `{}` max_pages must be between 1 and 200",
                    source.id
                )));
            }
        }

        Ok(())
    }
}

/// Entries and diagnostics produced by one resolver run.
#[derive(Debug, Default)]
pub struct Resolved {
    /// Parsed entries.
    pub entries: Vec<Entry>,
    /// Human-readable warnings for the crawl report.
    pub notes: Vec<String>,
    /// Number of successfully fetched pages.
    pub pages_fetched: usize,
}

impl Resolved {
    /// Add a warning.
    pub fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }

    /// Merge another resolver result.
    pub fn absorb(&mut self, other: Self) {
        self.entries.extend(other.entries);
        self.notes.extend(other.notes);
        self.pages_fetched += other.pages_fetched;
    }

    /// Remove duplicate URLs, keeping the first occurrence.
    pub fn dedupe(&mut self) {
        let mut seen = HashSet::new();
        self.entries.retain(|entry| seen.insert(entry.url.clone()));
    }
}

/// Dispatch to the resolver named by `source`.
pub async fn resolve_source(
    client: &Client,
    source: &SourceDefinition,
) -> ResolverResult<Resolved> {
    match source.resolver {
        ResolverKind::Ncert => ncert::resolve(client, source).await,
        ResolverKind::CbseAcademic => cbse_academic::resolve(client, source).await,
        ResolverKind::Kseab => kseab::resolve(client, source).await,
        ResolverKind::Pue => pue::resolve(client, source).await,
        ResolverKind::GenericIndex => generic_index::resolve(client, source).await,
    }
}

/// Human-readable board name for generated catalogs.
#[must_use]
pub fn board_name(id: &str) -> String {
    match id {
        "kseeb" => "Karnataka State Board".to_owned(),
        "pue" => "Karnataka Pre-University".to_owned(),
        "cbse" => "CBSE".to_owned(),
        "ncert" => "NCERT".to_owned(),
        "nios" => "NIOS".to_owned(),
        "diksha" => "DIKSHA".to_owned(),
        "swayam" => "SWAYAM".to_owned(),
        "nptel" => "NPTEL".to_owned(),
        other => {
            let mut characters = other.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => "Unknown".to_owned(),
            }
        }
    }
}

/// Fetch a page, returning `None` for 404/410 (used for probing years).
///
/// One retry is attempted after a network failure or transient 5xx response:
/// several government portals are slow and intermittently close connections.
pub(crate) async fn fetch_optional_html(
    client: &Client,
    url: &Url,
) -> ResolverResult<Option<String>> {
    match fetch_optional_html_once(client, url).await {
        Ok(result) => Ok(result),
        Err(error) if is_retryable(&error) => {
            sleep(Duration::from_secs(1)).await;
            fetch_optional_html_once(client, url).await
        }
        Err(error) => Err(error),
    }
}

fn is_retryable(error: &ResolverError) -> bool {
    match error {
        ResolverError::Http { .. } => true,
        ResolverError::Status { status, .. } => *status == 429 || (500..=599).contains(status),
        _ => false,
    }
}

async fn fetch_optional_html_once(client: &Client, url: &Url) -> ResolverResult<Option<String>> {
    let response = client
        .get(url.as_str())
        .send()
        .await
        .map_err(|error| ResolverError::Http {
            url: url.to_string(),
            message: crate::http::error_chain(&error),
        })?;

    if matches!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
    ) {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(ResolverError::Status {
            url: url.to_string(),
            status: response.status().as_u16(),
        });
    }

    if let Some(length) = response.content_length() {
        if length > MAX_HTML_BYTES as u64 {
            return Err(ResolverError::Parse {
                url: url.to_string(),
                message: format!("page is too large ({length} bytes)"),
            });
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| ResolverError::Http {
            url: url.to_string(),
            message: crate::http::error_chain(&error),
        })?;

    if bytes.len() > MAX_HTML_BYTES {
        return Err(ResolverError::Parse {
            url: url.to_string(),
            message: format!("page is too large ({} bytes)", bytes.len()),
        });
    }

    String::from_utf8(bytes.to_vec())
        .map(Some)
        .map_err(|_| ResolverError::Parse {
            url: url.to_string(),
            message: "page is not valid UTF-8".to_owned(),
        })
}

/// Fetch a required page.
pub(crate) async fn fetch_html(client: &Client, url: &Url) -> ResolverResult<String> {
    match fetch_optional_html(client, url).await? {
        Some(contents) => Ok(contents),
        None => Err(ResolverError::Status {
            url: url.to_string(),
            status: 404,
        }),
    }
}

/// Parse an HTML document.
pub(crate) fn parse_document(html: &str) -> Html {
    Html::parse_document(html)
}

/// Collapse whitespace and non-breaking spaces.
pub(crate) fn clean_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut pending_space = false;

    for character in value.chars() {
        if character.is_whitespace() || character == '\u{a0}' {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }

    output.trim().to_owned()
}

/// Join a href to a page and keep only absolute http(s) URLs.
pub(crate) fn resolve_url(base: &Url, href: &str) -> Option<Url> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
    {
        return None;
    }

    let url = base.join(href).ok()?;
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
        Some(url)
    } else {
        None
    }
}

/// Whether a URL path ends in `.pdf`.
pub(crate) fn is_pdf_url(url: &Url) -> bool {
    url.path().to_ascii_lowercase().ends_with(".pdf")
}

/// First 1900-2100 year in a string.
pub(crate) fn extract_year(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate().take(bytes.len().saturating_sub(3)) {
        if !byte.is_ascii_digit() {
            continue;
        }

        let Some(slice) = text.get(index..index + 4) else {
            continue;
        };

        if slice.bytes().all(|byte| byte.is_ascii_digit()) {
            if let Ok(year) = slice.parse::<u16>() {
                if (1900..=2100).contains(&year) {
                    return Some(year);
                }
            }
        }
    }
    None
}

/// Infer a language from English-language labels and URLs.
pub(crate) fn language_from_text(text: &str) -> Language {
    let lower = text.to_lowercase();

    if lower.contains("kannada")
        || lower.contains("ಕನ್ನಡ")
        || lower.contains(" kan")
        || lower.contains("km ")
    {
        Language::Kn
    } else if lower.contains("urdu") || lower.contains(" um ") {
        Language::Ur
    } else if lower.contains("hindi") {
        Language::Hi
    } else {
        Language::En
    }
}

/// Infer a stream from a PUC subject name.
pub(crate) fn stream_for_subject(subject: &str) -> Option<String> {
    let subject = subject.to_lowercase();
    let science = [
        "physics",
        "chemistry",
        "mathematics",
        "maths",
        "biology",
        "botany",
        "zoology",
        "science",
        "electronics",
        "statistics",
    ];
    let commerce = [
        "accountancy",
        "business studies",
        "economics",
        "economic development",
        "entrepreneurship",
        "accounting",
    ];
    let arts = [
        "history",
        "political science",
        "sociology",
        "geography",
        "psychology",
        "home science",
        "music",
    ];

    if science.iter().any(|needle| subject.contains(needle)) {
        Some("Science".to_owned())
    } else if commerce.iter().any(|needle| subject.contains(needle)) {
        Some("Commerce".to_owned())
    } else if arts.iter().any(|needle| subject.contains(needle)) {
        Some("Arts".to_owned())
    } else {
        None
    }
}

/// Infer a material type from link text.
pub(crate) fn infer_material_type(text: &str) -> MaterialType {
    let text = text.to_lowercase();

    if text.contains("syllab") {
        MaterialType::Syllabus
    } else if text.contains("marking")
        || text.contains("answer key")
        || text.contains("model answer")
    {
        MaterialType::MarkingScheme
    } else if text.contains("sample") {
        MaterialType::SamplePaper
    } else if text.contains("model") {
        MaterialType::ModelPaper
    } else if text.contains("previous")
        || text.contains("question paper")
        || text.contains("question-paper")
        || text.contains(" qp")
    {
        MaterialType::PreviousYearPaper
    } else {
        MaterialType::Textbook
    }
}

/// Infer a level from link text.
pub(crate) fn infer_level(text: &str) -> Option<String> {
    let lower = text.to_lowercase();

    if lower.contains("sslc") {
        return Some("SSLC".to_owned());
    }
    if lower.contains("puc 2") || lower.contains("ii puc") || lower.contains("2nd puc") {
        return Some("PUC 2".to_owned());
    }
    if lower.contains("puc 1") || lower.contains("i puc") || lower.contains("1st puc") {
        return Some("PUC 1".to_owned());
    }

    for (index, _) in lower.match_indices("class ") {
        let digits: String = lower[index + "class ".len()..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        if let Ok(level) = digits.parse::<u8>() {
            if (1..=12).contains(&level) {
                return Some(level.to_string());
            }
        }
    }

    None
}

/// Split CamelCase / underscore identifiers into words.
pub(crate) fn humanize_identifier(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_was_lower = false;

    for character in value.chars() {
        if character == '_' || character == '-' {
            output.push(' ');
            previous_was_lower = false;
            continue;
        }

        if character.is_uppercase() && previous_was_lower {
            output.push(' ');
        }
        output.push(character);
        previous_was_lower = character.is_lowercase();
    }

    clean_text(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = r#"
[[source]]
id = "cbse-sqp"
name = "CBSE"
resolver = "cbse_academic"
board = "cbse"
base_url = "https://cbseacademic.nic.in/"
levels = ["10", "12"]
year_range = [2016, 2026]
status = "verified"

[[source]]
id = "ktbs"
name = "KTBS"
resolver = "generic_index"
board = "kseeb"
base_url = "https://ktbs.kar.nic.in/"
levels = ["1", "10"]
year_range = [2016, 2026]
status = "unverified"
include_terms = ["textbook"]
"#;

    #[test]
    fn parses_and_selects_registry_sources() {
        let registry = Registry::from_toml("test", REGISTRY).unwrap();
        assert_eq!(registry.sources.len(), 2);

        let verified = registry.select(&[], false).unwrap();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].id, "cbse-sqp");

        let all = registry.select(&[], true).unwrap();
        assert_eq!(all.len(), 2);

        let explicit = registry.select(&["ktbs".to_owned()], false).unwrap();
        assert_eq!(explicit.len(), 1);
        assert_eq!(explicit[0].id, "ktbs");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let duplicate = REGISTRY.replace("\"ktbs\"", "\"cbse-sqp\"");
        assert!(Registry::from_toml("test", &duplicate).is_err());
    }

    #[test]
    fn rejects_bad_ranges_and_urls() {
        let bad_range = REGISTRY.replace("[2016, 2026]", "[2026, 2016]");
        assert!(Registry::from_toml("test", &bad_range).is_err());

        let bad_url =
            REGISTRY.replace("https://cbseacademic.nic.in/", "ftp://cbseacademic.nic.in/");
        assert!(Registry::from_toml("test", &bad_url).is_err());
    }

    #[test]
    fn text_helpers_normalise_and_classify() {
        assert_eq!(clean_text("  a \n  b\u{a0}c  "), "a b c");
        assert_eq!(extract_year("SSLC 2025 EXAM-1"), Some(2025));
        assert_eq!(extract_year("no year"), None);
        assert_eq!(language_from_text("Kannada Medium"), Language::Kn);
        assert_eq!(language_from_text("English Medium"), Language::En);
        assert_eq!(language_from_text("Hindi"), Language::Hi);
        assert_eq!(stream_for_subject("Physics"), Some("Science".to_owned()));
        assert_eq!(
            stream_for_subject("Business Studies"),
            Some("Commerce".to_owned())
        );
        assert_eq!(stream_for_subject("History"), Some("Arts".to_owned()));
        assert_eq!(stream_for_subject("Kannada"), None);
    }

    #[test]
    fn infers_types_and_levels() {
        assert_eq!(
            infer_material_type("Sample Question Paper"),
            MaterialType::SamplePaper
        );
        assert_eq!(
            infer_material_type("Marking Scheme"),
            MaterialType::MarkingScheme
        );
        assert_eq!(infer_material_type("Syllabus"), MaterialType::Syllabus);
        assert_eq!(infer_level("Class 10 Science"), Some("10".to_owned()));
        assert_eq!(infer_level("II PUC Physics"), Some("PUC 2".to_owned()));
        assert_eq!(infer_level("SSLC papers"), Some("SSLC".to_owned()));
    }

    #[test]
    fn humanizes_camel_case_filenames() {
        assert_eq!(humanize_identifier("BusinessStudies"), "Business Studies");
        assert_eq!(
            humanize_identifier("CarnaticMusicMelodicInstrument"),
            "Carnatic Music Melodic Instrument"
        );
    }
}
