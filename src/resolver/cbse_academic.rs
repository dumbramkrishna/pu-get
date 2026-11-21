//! Resolver for CBSE sample question papers and marking schemes.

use std::collections::HashSet;
use std::time::Duration;

use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use super::{
    clean_text, fetch_optional_html, humanize_identifier, is_pdf_url, parse_document, resolve_url,
    Resolved, SourceDefinition,
};
use crate::catalog::{Entry, MaterialType};
use crate::config::Language;
use crate::error::{ResolverError, ResolverResult};

/// Probe sessions from newest to oldest for both Class XII and Class X.
pub async fn resolve(client: &Client, source: &SourceDefinition) -> ResolverResult<Resolved> {
    let base = source.base_url()?;
    let mut resolved = Resolved::default();
    let mut errors = 0_usize;

    for year in source.years() {
        for (level, prefix) in [("12", "SQP_CLASSXII"), ("10", "SQP_CLASSX")] {
            if !source.levels.iter().any(|declared| declared == level) {
                continue;
            }

            let next = (year + 1) % 100;
            let page = base
                .join(&format!("{prefix}_{year}-{next:02}.html"))
                .map_err(|error| ResolverError::Parse {
                    url: base.to_string(),
                    message: format!("could not build CBSE page URL: {error}"),
                })?;

            match fetch_optional_html(client, &page).await {
                Ok(Some(html)) => {
                    resolved.pages_fetched += 1;
                    let entries = parse_page(&html, &page, level, year, source);
                    if entries.is_empty() {
                        resolved.note(format!("{page}: no SQP/MS links found"));
                    }
                    resolved.entries.extend(entries);
                }
                Ok(None) => {}
                Err(error) => {
                    errors += 1;
                    resolved.note(format!("{page}: {error}"));
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    if resolved.pages_fetched == 0 && errors > 0 {
        return Err(ResolverError::Parse {
            url: base.to_string(),
            message: format!("all {errors} CBSE sample-paper page requests failed"),
        });
    }

    if resolved.entries.is_empty() && resolved.pages_fetched == 0 {
        resolved.note("no CBSE sample-paper pages were found in the configured year range");
    }

    resolved.dedupe();
    Ok(resolved)
}

/// Parse one `SQP_CLASS*_<session>.html` page.
#[must_use]
pub fn parse_page(
    html: &str,
    page_url: &Url,
    level: &str,
    session_year: u16,
    source: &SourceDefinition,
) -> Vec<Entry> {
    let document: Html = parse_document(html);
    let anchor_selector = Selector::parse("a[href]").expect("static anchor selector");
    let mut entries = Vec::new();
    let mut seen = HashSet::new();

    for anchor in document.select(&anchor_selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(url) = resolve_url(page_url, href) else {
            continue;
        };
        if !is_pdf_url(&url) || !seen.insert(url.to_string()) {
            continue;
        }

        let filename = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or_default();
        let stem = filename
            .strip_suffix(".pdf")
            .or_else(|| filename.strip_suffix(".PDF"))
            .unwrap_or(filename);
        let lower = stem.to_ascii_lowercase();

        let (kind, marker, label) = if lower.contains("-sqp") {
            (MaterialType::SamplePaper, "-sqp", "Sample Question Paper")
        } else if lower.contains("-ms") {
            (MaterialType::MarkingScheme, "-ms", "Marking Scheme")
        } else {
            continue;
        };

        let Some(marker_position) = lower.find(marker) else {
            continue;
        };
        let subject = humanize_identifier(&stem[..marker_position]);
        if subject.is_empty() {
            continue;
        }

        let suffix = &lower[marker_position + marker.len()..];
        let language = if suffix.contains("_hi") {
            Language::Hi
        } else if suffix.contains("_ur") {
            Language::Ur
        } else {
            Language::En
        };

        let next = (session_year + 1) % 100;
        let language_suffix = match language {
            Language::En => "",
            Language::Hi => " (Hindi)",
            Language::Ur => " (Urdu)",
            Language::Kn => " (Kannada)",
        };
        let title = format!(
            "CBSE Class {level} {subject} {label} {session_year}-{next:02}{language_suffix}"
        );

        entries.push(Entry {
            level: level.to_owned(),
            stream: None,
            subject,
            kind,
            title: clean_text(&title),
            language,
            year: Some(session_year),
            url: url.to_string(),
            source: source.name.clone(),
            page: Some(page_url.to_string()),
            size_bytes: None,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{Registry, SourceDefinition};

    const FIXTURE: &str = include_str!("../../tests/fixtures/resolvers/cbse_class12.html");
    const REGISTRY: &str = include_str!("../../sources/registry.toml");

    fn source() -> SourceDefinition {
        Registry::from_toml("registry", REGISTRY)
            .unwrap()
            .find("cbse-sqp")
            .unwrap()
            .clone()
    }

    #[test]
    fn parses_sample_papers_and_marking_schemes() {
        let page = Url::parse("https://cbseacademic.nic.in/SQP_CLASSXII_2026-27.html").unwrap();
        let entries = parse_page(FIXTURE, &page, "12", 2026, &source());

        assert_eq!(entries.len(), 6);
        assert!(
            entries
                .iter()
                .any(|entry| entry.subject == "Accountancy"
                    && entry.kind == MaterialType::SamplePaper)
        );
        assert!(entries.iter().any(|entry| entry.subject == "Biology"
            && entry.kind == MaterialType::MarkingScheme
            && entry.language == Language::Hi));
        assert!(entries
            .iter()
            .any(|entry| entry.subject == "Business Studies"));
        assert!(entries
            .iter()
            .all(|entry| entry.level == "12" && entry.year == Some(2026)));
        assert!(entries
            .iter()
            .all(|entry| entry.url.starts_with("https://cbseacademic.nic.in/")));
    }

    #[test]
    fn ignores_non_sqp_pdfs() {
        let page = Url::parse("https://cbseacademic.nic.in/SQP_CLASSXII_2026-27.html").unwrap();
        let entries = parse_page(FIXTURE, &page, "12", 2026, &source());
        assert!(entries
            .iter()
            .all(|entry| !entry.url.ends_with("error.pdf")));
    }
}
