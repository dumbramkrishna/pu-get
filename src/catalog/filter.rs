//! Filtering, search and "latest only" selection for catalogs.
//!
//! Filtering is pure: [`Filter::apply`] borrows a [`Catalog`] and returns
//! borrowed [`CatalogEntry`] values in catalog order. The TUI and the
//! non-interactive `list`/`get` commands share exactly the same semantics.

use std::collections::HashMap;

use crate::catalog::model::{
    canonical_level, BoardInfo, Catalog, CatalogEntry, Entry, MaterialType,
};
use crate::config::Language;

/// User-selected catalog constraints.
///
/// Empty strings are treated as "no constraint", which makes it safe to feed
/// values straight from an interactive search box.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Board id or board display name.
    pub board: Option<String>,
    /// Level alias (`SSLC`, `PUC 2`, `Class 12`, `12`, ...).
    pub level: Option<String>,
    /// Stream (`Science`, `Commerce`, `Arts`).
    pub stream: Option<String>,
    /// Subject (case-insensitive substring).
    pub subject: Option<String>,
    /// Material type.
    pub kind: Option<MaterialType>,
    /// Exact year.
    pub year: Option<u16>,
    /// Language.
    pub language: Option<Language>,
    /// Free-text terms; every whitespace-separated term must match.
    pub search: Option<String>,
    /// Keep only the newest dated entry per board/level/stream/subject/type.
    pub latest_only: bool,
}

impl Filter {
    /// Whether a single entry satisfies every active constraint.
    #[must_use]
    pub fn matches(&self, board: &BoardInfo, entry: &Entry) -> bool {
        self.matches_board(board)
            && self.matches_level(board, entry)
            && self.matches_stream(entry)
            && self.matches_subject(entry)
            && match self.kind {
                Some(kind) => entry.kind == kind,
                None => true,
            }
            && match self.year {
                Some(year) => entry.year == Some(year),
                None => true,
            }
            && match self.language {
                Some(language) => entry.language == language,
                None => true,
            }
            && self.matches_search(board, entry)
    }

    /// Apply this filter to a catalog, optionally collapsing older editions.
    #[must_use]
    pub fn apply<'a>(&self, catalog: &'a Catalog) -> Vec<CatalogEntry<'a>> {
        let entries: Vec<CatalogEntry<'a>> = catalog
            .entries()
            .filter(|item| self.matches(item.board, item.entry))
            .collect();

        if self.latest_only {
            keep_newest(entries)
        } else {
            entries
        }
    }

    /// Whether any constraint is active. Useful for empty-state messaging.
    #[must_use]
    pub fn is_active(&self) -> bool {
        query(&self.board).is_some()
            || query(&self.level).is_some()
            || query(&self.stream).is_some()
            || query(&self.subject).is_some()
            || self.kind.is_some()
            || self.year.is_some()
            || self.language.is_some()
            || query(&self.search).is_some()
            || self.latest_only
    }

    fn matches_board(&self, board: &BoardInfo) -> bool {
        query(&self.board).is_none_or(|query| {
            board.id.eq_ignore_ascii_case(query) || contains_ignore_case(&board.name, query)
        })
    }

    fn matches_level(&self, board: &BoardInfo, entry: &Entry) -> bool {
        let Some(query) = query(&self.level) else {
            return true;
        };

        let query = canonical_level(query);
        let candidate = canonical_level(&entry.level);

        if query == candidate {
            return true;
        }

        if board.id.eq_ignore_ascii_case("kseeb") {
            return matches!(
                (query.as_str(), candidate.as_str()),
                ("10", "sslc") | ("sslc", "10")
            );
        }

        if board.id.eq_ignore_ascii_case("pue") {
            return matches!(
                (query.as_str(), candidate.as_str()),
                ("11", "puc1") | ("puc1", "11") | ("12", "puc2") | ("puc2", "12")
            );
        }

        false
    }

    fn matches_stream(&self, entry: &Entry) -> bool {
        query(&self.stream).is_none_or(|query| {
            // Entries without a stream (languages, common papers) belong to
            // every stream within a board/level that uses streams.
            entry
                .stream
                .as_deref()
                .is_none_or(|stream| contains_ignore_case(stream, query))
        })
    }

    fn matches_subject(&self, entry: &Entry) -> bool {
        query(&self.subject).is_none_or(|query| contains_ignore_case(&entry.subject, query))
    }

    fn matches_search(&self, board: &BoardInfo, entry: &Entry) -> bool {
        let Some(search) = query(&self.search) else {
            return true;
        };

        let haystack = [
            entry.title.as_str(),
            entry.subject.as_str(),
            entry.source.as_str(),
            entry.level.as_str(),
            entry.stream.as_deref().unwrap_or(""),
            board.name.as_str(),
        ]
        .join(" ")
        .to_lowercase();

        search
            .split_whitespace()
            .all(|term| haystack.contains(&term.to_lowercase()))
    }
}

/// Grouping key for latest-only selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LatestKey {
    board: String,
    level: String,
    stream: String,
    subject: String,
    kind: MaterialType,
    language: Language,
}

impl From<CatalogEntry<'_>> for LatestKey {
    fn from(item: CatalogEntry<'_>) -> Self {
        Self {
            board: item.board.id.clone(),
            level: canonical_level(&item.entry.level),
            stream: item
                .entry
                .stream
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_lowercase(),
            subject: item.entry.subject.trim().to_lowercase(),
            kind: item.entry.kind,
            language: item.entry.language,
        }
    }
}

fn keep_newest<'a>(entries: Vec<CatalogEntry<'a>>) -> Vec<CatalogEntry<'a>> {
    let mut newest: HashMap<LatestKey, u16> = HashMap::new();

    for item in &entries {
        let Some(year) = item.entry.year else {
            continue;
        };

        let key = LatestKey::from(*item);
        match newest.get_mut(&key) {
            Some(maximum) if year > *maximum => *maximum = year,
            Some(_) => {}
            None => {
                newest.insert(key, year);
            }
        }
    }

    entries
        .into_iter()
        .filter(|item| match item.entry.year {
            Some(year) => match newest.get(&LatestKey::from(*item)) {
                Some(maximum) => year >= *maximum,
                None => true,
            },
            None => true,
        })
        .collect()
}

fn query(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::model::Catalog;

    const FIXTURE: &str = r#"
[board]
id = "kseeb"
name = "Karnataka State Board"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "Mathematics Paper 2025"
language = "kn"
year = 2025
url = "https://example.com/kseeb/maths-2025.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "Mathematics Paper 2024"
language = "kn"
year = 2024
url = "https://example.com/kseeb/maths-2024.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Science"
type = "model_paper"
title = "Science Model Paper 2025"
language = "en"
year = 2025
url = "https://example.com/kseeb/science-model-2025.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "textbook"
title = "Mathematics Textbook"
language = "kn"
url = "https://example.com/kseeb/maths-textbook.pdf"
source = "KTBS"
"#;

    const PUE_FIXTURE: &str = r#"
[board]
id = "pue"
name = "Karnataka Pre-University"

[[entries]]
level = "PUC 1"
stream = "Science"
subject = "Physics"
type = "textbook"
title = "Physics Textbook"
language = "kn"
year = 2025
url = "https://example.com/pue/puc1-physics.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 2"
stream = "Science"
subject = "Physics"
type = "textbook"
title = "Physics Textbook 2"
language = "kn"
year = 2025
url = "https://example.com/pue/puc2-physics.pdf"
source = "PUE Karnataka"
"#;

    fn catalog() -> Catalog {
        Catalog::from_toml("filter-test", FIXTURE).unwrap()
    }

    #[test]
    fn latest_only_keeps_newest_per_series() {
        let filter = Filter {
            board: Some("kseeb".to_owned()),
            level: Some("Class 10".to_owned()),
            latest_only: true,
            ..Filter::default()
        };

        let catalog = catalog();
        let results = filter.apply(&catalog);
        let titles: Vec<&str> = results
            .iter()
            .map(|item| item.entry.title.as_str())
            .collect();

        assert_eq!(results.len(), 3);
        assert!(titles.contains(&"Mathematics Paper 2025"));
        assert!(!titles.contains(&"Mathematics Paper 2024"));
        assert!(titles.contains(&"Science Model Paper 2025"));
        assert!(titles.contains(&"Mathematics Textbook"));
    }

    #[test]
    fn year_and_language_constraints_combine() {
        let filter = Filter {
            year: Some(2024),
            language: Some(Language::Kn),
            ..Filter::default()
        };

        let catalog = catalog();
        let results = filter.apply(&catalog);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.year, Some(2024));
    }

    #[test]
    fn board_aware_level_aliases_match() {
        let catalog = Catalog::from_toml("pue-test", PUE_FIXTURE).unwrap();

        let level_11 = Filter {
            board: Some("pue".to_owned()),
            level: Some("11".to_owned()),
            ..Filter::default()
        };
        assert_eq!(level_11.apply(&catalog).len(), 1);

        let pu_ii = Filter {
            board: Some("pue".to_owned()),
            level: Some("PU II".to_owned()),
            ..Filter::default()
        };
        assert_eq!(pu_ii.apply(&catalog).len(), 1);
    }

    #[test]
    fn stream_filter_keeps_entries_common_to_all_streams() {
        let text = r#"
[board]
id = "pue"
name = "Karnataka Pre-University"

[[entries]]
level = "PUC 1"
stream = "Science"
subject = "Physics"
type = "textbook"
title = "Physics Textbook"
url = "https://example.com/pue/physics.pdf"
source = "PUE Karnataka"

[[entries]]
level = "PUC 1"
subject = "Kannada"
type = "textbook"
title = "Kannada Textbook"
url = "https://example.com/pue/kannada.pdf"
source = "PUE Karnataka"
"#;

        let catalog = Catalog::from_toml("pue-common", text).unwrap();
        let science = Filter {
            stream: Some("Science".to_owned()),
            ..Filter::default()
        };
        assert_eq!(science.apply(&catalog).len(), 2);
    }

    #[test]
    fn stream_and_subject_filters_work() {
        let catalog = Catalog::from_toml("pue-test", PUE_FIXTURE).unwrap();
        let filter = Filter {
            stream: Some("science".to_owned()),
            subject: Some("phys".to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.apply(&catalog).len(), 2);

        let commerce = Filter {
            stream: Some("Commerce".to_owned()),
            ..Filter::default()
        };
        assert!(commerce.apply(&catalog).is_empty());
    }

    #[test]
    fn search_requires_every_term() {
        let catalog = catalog();
        let filter = Filter {
            search: Some("mathematics 2025".to_owned()),
            ..Filter::default()
        };
        let results = filter.apply(&catalog);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.title, "Mathematics Paper 2025");
    }

    #[test]
    fn empty_filter_text_is_ignored() {
        let catalog = catalog();
        let filter = Filter {
            board: Some("  ".to_owned()),
            subject: Some(String::new()),
            ..Filter::default()
        };
        assert_eq!(filter.apply(&catalog).len(), 4);
    }
}
