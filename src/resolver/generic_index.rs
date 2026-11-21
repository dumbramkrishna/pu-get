//! Generic same-host PDF index resolver for unverified official sources.
//!
//! It only emits direct `.pdf` links and follows a bounded number of internal
//! pages whose text matches the source's `include_terms`. PDFs are the only
//! resources ever emitted, so the resolver is safe for portals that mix
//! circulars, images and documents.

use std::collections::HashSet;
use std::time::Duration;

use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use super::{
    clean_text, extract_year, fetch_html, fetch_optional_html, humanize_identifier, infer_level,
    infer_material_type, is_pdf_url, language_from_text, parse_document, resolve_url, Resolved,
    SourceDefinition,
};
use crate::catalog::Entry;
use crate::error::ResolverResult;

/// Result of parsing one generic index page.
#[derive(Debug, Default)]
pub struct ParsedPage {
    /// Direct PDF entries.
    pub entries: Vec<Entry>,
    /// Internal pages that match the source's include terms.
    pub follow: Vec<Url>,
}

/// Fetch the base page and up to `max_pages - 1` matching internal pages.
pub async fn resolve(client: &Client, source: &SourceDefinition) -> ResolverResult<Resolved> {
    let base = source.base_url()?;
    let html = fetch_html(client, &base).await?;

    let mut resolved = Resolved {
        pages_fetched: 1,
        ..Resolved::default()
    };

    let parsed = parse_page(&html, &base, source);
    resolved.entries.extend(parsed.entries);

    let mut seen_pages = HashSet::new();
    seen_pages.insert(base.to_string());

    for page in parsed
        .follow
        .into_iter()
        .take(source.max_pages.saturating_sub(1))
    {
        if !seen_pages.insert(page.to_string()) {
            continue;
        }

        match fetch_optional_html(client, &page).await {
            Ok(Some(html)) => {
                resolved.pages_fetched += 1;
                resolved
                    .entries
                    .extend(parse_page(&html, &page, source).entries);
            }
            Ok(None) => {}
            Err(error) => resolved.note(format!("{page}: {error}")),
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    if resolved.entries.is_empty() {
        resolved.note("no direct PDF links were found on the generic source pages");
    }

    resolved.dedupe();
    Ok(resolved)
}

/// Parse PDF links and matching internal follow links from one page.
#[must_use]
pub fn parse_page(html: &str, page_url: &Url, source: &SourceDefinition) -> ParsedPage {
    let document: Html = parse_document(html);
    let anchor_selector = Selector::parse("a[href]").expect("static anchor selector");

    let mut parsed = ParsedPage::default();
    let mut seen_entries = HashSet::new();
    let mut seen_follow = HashSet::new();

    for anchor in document.select(&anchor_selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(url) = resolve_url(page_url, href) else {
            continue;
        };

        let text = clean_text(&anchor.text().collect::<String>());

        if is_pdf_url(&url) {
            if !seen_entries.insert(url.to_string()) {
                continue;
            }

            let filename = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or_default();
            let fallback_title = humanize_identifier(
                filename
                    .strip_suffix(".pdf")
                    .or_else(|| filename.strip_suffix(".PDF"))
                    .unwrap_or(filename),
            );
            let title = if text.is_empty() {
                fallback_title
            } else {
                text.clone()
            };
            if title.is_empty() {
                continue;
            }

            let subject = infer_subject(&title);
            let level = infer_level(&title)
                .or_else(|| source.levels.first().cloned())
                .unwrap_or_else(|| "General".to_owned());
            let year = extract_year(&title)
                .filter(|year| (source.year_range[0]..=source.year_range[1]).contains(year));
            let kind = infer_material_type(&title);
            let language = language_from_text(&format!("{title} {url}"));

            parsed.entries.push(Entry {
                level,
                stream: None,
                subject,
                kind,
                title,
                language,
                year,
                url: url.to_string(),
                source: source.name.clone(),
                page: Some(page_url.to_string()),
                size_bytes: None,
            });
        } else if url.host_str() == page_url.host_str() && seen_follow.insert(url.to_string()) {
            let haystack = format!("{} {}", text.to_lowercase(), url.as_str().to_lowercase());
            if source
                .include_terms
                .iter()
                .any(|term| haystack.contains(&term.to_lowercase()))
            {
                parsed.follow.push(url);
            }
        }
    }

    parsed
}

fn infer_subject(text: &str) -> String {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|word| {
            let lower = word
                .trim_matches(|character: char| !character.is_ascii_alphanumeric())
                .to_lowercase();
            !lower.is_empty()
                && !matches!(
                    lower.as_str(),
                    "class"
                        | "model"
                        | "question"
                        | "paper"
                        | "papers"
                        | "sample"
                        | "previous"
                        | "year"
                        | "syllabus"
                        | "marking"
                        | "scheme"
                        | "textbook"
                        | "text"
                        | "book"
                        | "pdf"
                        | "sslc"
                        | "puc"
                        | "karnataka"
                        | "i"
                        | "ii"
                )
                && !lower.starts_with("20")
                && !lower.chars().all(|character| character.is_ascii_digit())
        })
        .collect();

    if words.is_empty() {
        "General".to_owned()
    } else {
        words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::MaterialType;
    use crate::resolver::Registry;

    const FIXTURE: &str = include_str!("../../tests/fixtures/resolvers/generic_index.html");

    fn source() -> SourceDefinition {
        let registry = Registry::from_toml(
            "registry",
            r#"
[[source]]
id = "generic"
name = "Generic Official"
resolver = "generic_index"
board = "other"
base_url = "https://example.com/"
levels = ["10"]
year_range = [2016, 2026]
status = "unverified"
include_terms = ["model papers"]
max_pages = 3
"#,
        )
        .unwrap();
        registry.find("generic").unwrap().clone()
    }

    #[test]
    fn parses_pdf_entries_and_follow_links() {
        let page = Url::parse("https://example.com/").unwrap();
        let parsed = parse_page(FIXTURE, &page, &source());

        assert_eq!(parsed.entries.len(), 3);
        assert_eq!(parsed.follow.len(), 1);
        assert_eq!(
            parsed.follow[0].as_str(),
            "https://example.com/more-model-papers.html"
        );

        let syllabus = parsed
            .entries
            .iter()
            .find(|entry| entry.kind == MaterialType::Syllabus)
            .expect("syllabus entry");
        assert_eq!(syllabus.level, "10");
        assert_eq!(syllabus.subject, "Science");
        assert_eq!(syllabus.year, Some(2025));

        let model = parsed
            .entries
            .iter()
            .find(|entry| entry.kind == MaterialType::ModelPaper)
            .expect("model paper");
        assert_eq!(model.subject, "General");
    }
}
