//! Resolver for Karnataka Pre-University textbook PDFs.

use std::collections::HashSet;

use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use super::{
    clean_text, fetch_html, is_pdf_url, language_from_text, parse_document, resolve_url,
    stream_for_subject, Resolved, SourceDefinition,
};
use crate::catalog::{Entry, MaterialType};
use crate::error::ResolverResult;

/// Fetch and parse the PUE textbook index.
pub async fn resolve(client: &Client, source: &SourceDefinition) -> ResolverResult<Resolved> {
    let base = source.base_url()?;
    let html = fetch_html(client, &base).await?;
    let mut resolved = parse_index(&html, &base, source);
    resolved.pages_fetched = 1;

    if resolved.entries.is_empty() {
        resolved.note("no textbook PDF links were found on the PUE index page");
    }

    Ok(resolved)
}

/// Parse the PUE textbook table: column 1 is PUC 1, column 2 is PUC 2.
#[must_use]
pub fn parse_index(html: &str, page_url: &Url, source: &SourceDefinition) -> Resolved {
    let document: Html = parse_document(html);
    let row_selector = Selector::parse("table tr").expect("static row selector");
    let cell_selector = Selector::parse("td").expect("static cell selector");
    let anchor_selector = Selector::parse("a[href]").expect("static anchor selector");

    let mut resolved = Resolved::default();
    let mut seen = HashSet::new();

    for row in document.select(&row_selector) {
        let cells: Vec<_> = row.select(&cell_selector).collect();
        if cells.len() < 3 {
            continue;
        }

        for (column, level) in [(1_usize, "PUC 1"), (2_usize, "PUC 2")] {
            let Some(cell) = cells.get(column) else {
                continue;
            };

            let anchors: Vec<_> = cell
                .select(&anchor_selector)
                .filter(|anchor| {
                    anchor
                        .value()
                        .attr("href")
                        .is_some_and(|href| href.to_ascii_lowercase().contains(".pdf"))
                })
                .collect();

            if anchors.is_empty() {
                continue;
            }

            let cell_text = clean_text(&cell.text().collect::<String>());

            for (index, anchor) in anchors.iter().enumerate() {
                let Some(href) = anchor.value().attr("href") else {
                    continue;
                };
                let Some(url) = resolve_url(page_url, href) else {
                    continue;
                };
                if !is_pdf_url(&url) || !seen.insert(url.to_string()) {
                    continue;
                }

                let anchor_text = clean_text(&anchor.text().collect::<String>());
                let context = if cell_text.is_empty() {
                    anchor_text.clone()
                } else {
                    cell_text.clone()
                };

                let subject = infer_subject(&context);
                let language = language_from_text(&format!("{context} {url}"));
                let stream = stream_for_subject(&subject);
                let kind = infer_book_kind(&format!("{context} {anchor_text}"));
                let part = part_marker(&anchor_text)
                    .or_else(|| part_marker(&vector_encoded_url(&url)))
                    .or_else(|| (anchors.len() > 1).then(|| format!("Part {}", index + 1)));

                let mut title = format!("{level} {subject} {kind}");
                if let Some(part) = part {
                    title.push_str(" - ");
                    title.push_str(&part);
                }

                resolved.entries.push(Entry {
                    level: level.to_owned(),
                    stream,
                    subject,
                    kind: MaterialType::Textbook,
                    title,
                    language,
                    year: None,
                    url: url.to_string(),
                    source: source.name.clone(),
                    page: Some(page_url.to_string()),
                    size_bytes: None,
                });
            }
        }
    }

    resolved
}

fn infer_subject(raw: &str) -> String {
    let replaced = raw.replace(['_', '/', '&', '-'], " ");
    let mut words = Vec::new();

    for word in replaced.split_whitespace() {
        let lower = word
            .trim_matches(|character: char| !character.is_ascii_alphanumeric())
            .to_lowercase();

        if lower.is_empty()
            || lower.len() == 1
            || lower.chars().all(|character| character.is_ascii_digit())
            || matches!(
                lower.as_str(),
                "tb" | "wb"
                    | "em"
                    | "km"
                    | "bm"
                    | "textbook"
                    | "workbook"
                    | "book"
                    | "part"
                    | "vol"
                    | "volume"
                    | "new"
                    | "final"
                    | "pdf"
                    | "file"
                    | "edit"
                    | "optional"
                    | "puc"
            )
        {
            continue;
        }

        if (lower.starts_with("part") || lower.starts_with("vol"))
            && lower[4..]
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            continue;
        }

        words.push(title_case(word));
    }

    if words.is_empty() {
        "General".to_owned()
    } else {
        words.join(" ")
    }
}

fn title_case(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first
            .to_uppercase()
            .chain(characters.flat_map(|character| character.to_lowercase()))
            .collect(),
        None => String::new(),
    }
}

fn infer_book_kind(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("workbook") || lower.contains(" wb") || lower.contains("work book") {
        "Workbook"
    } else {
        "Textbook"
    }
}

fn part_marker(text: &str) -> Option<String> {
    let lower = text.to_lowercase();

    for keyword in ["part", "vol"] {
        if let Some(position) = lower.find(keyword) {
            let rest = text[position + keyword.len()..].trim_start_matches([' ', '-', '_', '.']);
            let token: String = rest
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric())
                .collect();
            if !token.is_empty() {
                let label = if keyword == "part" { "Part" } else { "Volume" };
                return Some(format!("{label} {token}"));
            }
        }
    }

    None
}

fn vector_encoded_url(url: &Url) -> String {
    url.path().replace("%20", " ").replace("%2F", "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{Registry, SourceDefinition};

    const FIXTURE: &str = include_str!("../../tests/fixtures/resolvers/pue_textbooks.html");
    const REGISTRY: &str = include_str!("../../sources/registry.toml");

    fn source() -> SourceDefinition {
        Registry::from_toml("registry", REGISTRY)
            .unwrap()
            .find("pue-textbooks")
            .unwrap()
            .clone()
    }

    #[test]
    fn parses_both_levels_and_subjects() {
        let page = Url::parse("https://pue.karnataka.gov.in/new-page/TEXT%20BOOKS/kn").unwrap();
        let resolved = parse_index(FIXTURE, &page, &source());

        assert!(!resolved.entries.is_empty());
        assert!(resolved
            .entries
            .iter()
            .all(|entry| entry.kind == MaterialType::Textbook));
        assert!(resolved.entries.iter().any(|entry| entry.level == "PUC 1"));
        assert!(resolved.entries.iter().any(|entry| entry.level == "PUC 2"));
        assert!(resolved
            .entries
            .iter()
            .any(|entry| entry.subject == "Physics"));
        assert!(resolved
            .entries
            .iter()
            .any(|entry| entry.subject == "Business Studies"));
        assert!(resolved
            .entries
            .iter()
            .any(|entry| entry.stream.as_deref() == Some("Science")));
        assert!(resolved
            .entries
            .iter()
            .any(|entry| entry.stream.as_deref() == Some("Commerce")));
        assert!(resolved
            .entries
            .iter()
            .any(|entry| entry.language == crate::config::Language::Kn));
        assert!(resolved
            .entries
            .iter()
            .all(|entry| entry.url.starts_with("https://pue.karnataka.gov.in/")));
    }

    #[test]
    fn deduplicates_urls() {
        let page = Url::parse("https://pue.karnataka.gov.in/new-page/TEXT%20BOOKS/kn").unwrap();
        let resolved = parse_index(FIXTURE, &page, &source());
        let mut urls = HashSet::new();
        assert!(resolved
            .entries
            .iter()
            .all(|entry| urls.insert(entry.url.clone())));
    }
}
