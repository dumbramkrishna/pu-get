//! Resolver for NCERT textbook chapters.
//!
//! `ncert.nic.in/textbook.php` is JavaScript-driven. The index embeds book
//! codes, classes, subjects, titles and chapter counts; we parse those values
//! and construct the chapter PDF URLs the page itself uses
//! (`textbook/pdf/<code><chapter>.pdf`). No codes are guessed.

use std::collections::{HashMap, HashSet};

use reqwest::Client;
use url::Url;

use super::{clean_text, fetch_html, Resolved, SourceDefinition};
use crate::catalog::{Entry, MaterialType};
use crate::config::Language;
use crate::error::ResolverResult;

/// Fetch and parse the NCERT textbook index.
pub async fn resolve(client: &Client, source: &SourceDefinition) -> ResolverResult<Resolved> {
    let base = source.base_url()?;
    let html = fetch_html(client, &base).await?;
    let mut resolved = parse_index(&html, &base, source);
    resolved.pages_fetched = 1;

    if resolved.entries.is_empty() {
        resolved.note("no textbook codes were found on the NCERT index page");
    }

    Ok(resolved)
}

/// Parse the book/chapter assignments from the NCERT index page.
#[must_use]
pub fn parse_index(html: &str, page_url: &Url, source: &SourceDefinition) -> Resolved {
    let mut resolved = Resolved::default();
    let mut seen = HashSet::new();
    let mut current_class: Option<u16> = None;
    let mut current_subject: Option<String> = None;
    let mut pending_titles: HashMap<usize, String> = HashMap::new();

    for raw_line in html.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some((class, subject)) = parse_condition(line) {
            current_class = (1..=12).contains(&class).then_some(class);
            current_subject = Some(subject);
            pending_titles.clear();
            continue;
        }

        if let Some((index, title)) = parse_title_assignment(line) {
            pending_titles.insert(index, title);
            continue;
        }

        let Some((index, code, last_chapter)) = parse_value_assignment(line) else {
            continue;
        };

        let Some(class) = current_class else {
            continue;
        };
        let Some(subject) = current_subject.as_deref() else {
            continue;
        };
        let Some(book_title) = pending_titles.remove(&index) else {
            continue;
        };

        for chapter in 1..=last_chapter {
            let filename = format!("{code}{chapter:02}.pdf");
            let Ok(url) = page_url.join(&format!("textbook/pdf/{filename}")) else {
                continue;
            };
            if !seen.insert(url.to_string()) {
                continue;
            }

            let language = language_for(&code, &book_title);
            let Some(language) = language else {
                continue;
            };

            resolved.entries.push(Entry {
                level: class.to_string(),
                stream: None,
                subject: clean_text(subject),
                kind: MaterialType::Textbook,
                title: clean_text(&format!(
                    "NCERT Class {class} {subject} - {book_title} - Chapter {chapter}"
                )),
                language,
                year: None,
                url: url.to_string(),
                source: source.name.clone(),
                page: Some(page_url.to_string()),
                size_bytes: None,
            });
        }
    }

    resolved.dedupe();
    resolved
}

fn parse_condition(line: &str) -> Option<(u16, String)> {
    if !line.contains("tclass.value==") || !line.contains("tsubject.options[sind].text==") {
        return None;
    }

    let class_marker = "tclass.value==";
    let class_start = line.find(class_marker)? + class_marker.len();
    let class_digits: String = line[class_start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    let class: u16 = class_digits.parse().ok()?;

    let subject_marker = "tsubject.options[sind].text==\"";
    let subject_start = line.find(subject_marker)? + subject_marker.len();
    let subject_rest = &line[subject_start..];
    let subject_end = subject_rest.find('"')?;
    let subject = clean_text(&subject_rest[..subject_end]);

    (!subject.is_empty()).then_some((class, subject))
}

fn parse_title_assignment(line: &str) -> Option<(usize, String)> {
    let marker = ".tbook.options[";
    if !line.contains(marker) || !line.contains(".text=\"") {
        return None;
    }

    let index_start = line.find(marker)? + marker.len();
    let index_end = line[index_start..].find(']')? + index_start;
    let index: usize = line[index_start..index_end].parse().ok()?;

    let text_marker = ".text=\"";
    let text_start = line.find(text_marker)? + text_marker.len();
    let text_rest = &line[text_start..];
    let text_end = text_rest.find('"')?;
    let title = clean_text(&text_rest[..text_end]);

    (!title.is_empty()).then_some((index, title))
}

fn parse_value_assignment(line: &str) -> Option<(usize, String, u16)> {
    let marker = ".tbook.options[";
    let value_marker = ".value=\"textbook.php?";
    if !line.contains(marker) || !line.contains(value_marker) {
        return None;
    }

    let index_start = line.find(marker)? + marker.len();
    let index_end = line[index_start..].find(']')? + index_start;
    let index: usize = line[index_start..index_end].parse().ok()?;

    let value_start = line.find(value_marker)? + value_marker.len();
    let value_rest = &line[value_start..];
    let value_end = value_rest.find('"')?;
    let query = &value_rest[..value_end];

    let (code, range) = query.split_once('=')?;
    let code = code.trim();
    if code.len() < 3
        || !code
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }

    let (_, last) = range.split_once('-')?;
    let last_chapter: u16 = last.trim().parse().ok()?;
    if last_chapter == 0 || last_chapter > 60 {
        return None;
    }

    Some((index, code.to_owned(), last_chapter))
}

fn language_for(code: &str, title: &str) -> Option<Language> {
    let lower_title = title.to_lowercase();
    if lower_title.contains("hindi") {
        return Some(Language::Hi);
    }
    if lower_title.contains("urdu") {
        return Some(Language::Ur);
    }
    if lower_title.contains("kannada") {
        return Some(Language::Kn);
    }

    match code.chars().nth(1) {
        Some('e') => Some(Language::En),
        Some('h') => Some(Language::Hi),
        Some('u') => Some(Language::Ur),
        // NCERT also publishes Marathi, Sindhi, Tamil, ... variants that the
        // catalog's language enum does not represent yet.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{Registry, SourceDefinition};

    const FIXTURE: &str = include_str!("../../tests/fixtures/resolvers/ncert_textbook.html");
    const REGISTRY: &str = include_str!("../../sources/registry.toml");

    fn source() -> SourceDefinition {
        Registry::from_toml("registry", REGISTRY)
            .unwrap()
            .find("ncert-textbooks")
            .unwrap()
            .clone()
    }

    fn page() -> Url {
        Url::parse("http://ncert.nic.in/textbook.php").unwrap()
    }

    #[test]
    fn parses_classes_subjects_and_chapters() {
        let resolved = parse_index(FIXTURE, &page(), &source());

        assert_eq!(resolved.entries.len(), 127);
        assert_eq!(
            resolved
                .entries
                .iter()
                .filter(|entry| entry.level == "12")
                .count(),
            42
        );
        assert_eq!(
            resolved
                .entries
                .iter()
                .filter(|entry| entry.level == "10" && entry.subject == "Mathematics")
                .count(),
            43
        );
        assert!(resolved.entries.iter().any(|entry| {
            entry.url == "http://ncert.nic.in/textbook/pdf/leph101.pdf"
                && entry.language == Language::En
                && entry.subject == "Physics"
        }));
        assert!(resolved.entries.iter().any(|entry| {
            entry.url == "http://ncert.nic.in/textbook/pdf/lhph101.pdf"
                && entry.language == Language::Hi
        }));
        assert!(resolved.entries.iter().any(|entry| {
            entry.url == "http://ncert.nic.in/textbook/pdf/jumh101.pdf"
                && entry.language == Language::Ur
        }));
    }

    #[test]
    fn ignores_commented_out_books() {
        let resolved = parse_index(FIXTURE, &page(), &source());
        assert!(resolved
            .entries
            .iter()
            .all(|entry| !entry.url.contains("leep6")));
    }

    #[test]
    fn every_entry_has_a_unique_url() {
        let resolved = parse_index(FIXTURE, &page(), &source());
        let mut urls = HashSet::new();
        assert!(resolved
            .entries
            .iter()
            .all(|entry| urls.insert(entry.url.clone())));
    }
}
