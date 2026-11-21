//! Typed catalog model.
//!
//! A catalog file describes exactly one board:
//!
//! ```toml
//! [board]
//! id = "kseeb"
//! name = "Karnataka State Board"
//!
//! [[entries]]
//! level = "SSLC"
//! subject = "Mathematics"
//! type = "previous_year_paper"
//! title = "SSLC Mathematics Previous Year Paper 2025"
//! language = "kn"
//! year = 2025
//! url = "https://..."
//! source = "KSEAB"
//! page = "https://..."
//! ```
//!
//! `stream`, `size_bytes` and `page` are optional. `year` is optional because
//! textbooks often have no meaningful exam year. An entry with an empty `url`
//! is ignored by the loader.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::Language;
use crate::error::CatalogError;

/// Kind of study material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialType {
    /// Textbook or textbook chapter.
    Textbook,
    /// Model question paper.
    ModelPaper,
    /// Previous year / question paper.
    PreviousYearPaper,
    /// Sample question paper.
    SamplePaper,
    /// Marking scheme or answer key.
    MarkingScheme,
    /// Syllabus.
    Syllabus,
}

impl MaterialType {
    /// Every supported material type, in display order.
    pub const ALL: [Self; 6] = [
        Self::Textbook,
        Self::ModelPaper,
        Self::PreviousYearPaper,
        Self::SamplePaper,
        Self::MarkingScheme,
        Self::Syllabus,
    ];

    /// Stable identifier used in catalogs, filters and folder templates.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Textbook => "textbook",
            Self::ModelPaper => "model_paper",
            Self::PreviousYearPaper => "previous_year_paper",
            Self::SamplePaper => "sample_paper",
            Self::MarkingScheme => "marking_scheme",
            Self::Syllabus => "syllabus",
        }
    }

    /// Human-readable label for the TUI and plain output.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Textbook => "Textbook",
            Self::ModelPaper => "Model Papers",
            Self::PreviousYearPaper => "Previous Year Papers",
            Self::SamplePaper => "Sample Papers",
            Self::MarkingScheme => "Marking Scheme",
            Self::Syllabus => "Syllabus",
        }
    }
}

impl fmt::Display for MaterialType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MaterialType {
    type Err = CatalogError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalised = value
            .trim()
            .to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .replace('-', "_");

        let kind = match normalised.as_str() {
            "textbook" | "textbooks" | "book" | "books" => Some(Self::Textbook),
            "model_paper" | "model_papers" | "model" | "models" => Some(Self::ModelPaper),
            "previous_year_paper"
            | "previous_year_papers"
            | "previous_year"
            | "previous_years"
            | "question_paper"
            | "question_papers"
            | "pyq"
            | "pyqp" => Some(Self::PreviousYearPaper),
            "sample_paper" | "sample_papers" | "sample" | "samples" => Some(Self::SamplePaper),
            "marking_scheme" | "marking_schemes" | "marking" => Some(Self::MarkingScheme),
            "syllabus" | "syllabi" | "syllabuses" => Some(Self::Syllabus),
            _ => None,
        };

        kind.ok_or_else(|| CatalogError::UnknownMaterialType(value.trim().to_owned()))
    }
}

/// A board (or board family) that owns a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardInfo {
    /// Stable lowercase identifier used by filters and folder templates.
    pub id: String,
    /// Display name shown in the TUI.
    pub name: String,
}

/// One downloadable (or placeholder) resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Level: `SSLC`, `PUC 1`, `PUC 2`, or `1`-`12`.
    pub level: String,
    /// Stream for pre-university material (`Science`, `Commerce`, `Arts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// Subject name.
    pub subject: String,
    /// Material type, serialised as `type`.
    #[serde(rename = "type")]
    pub kind: MaterialType,
    /// Human-readable title.
    pub title: String,
    /// Language of the resource.
    #[serde(default)]
    pub language: Language,
    /// Publication or exam year, when meaningful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// Direct PDF URL. Empty entries are skipped by the loader.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Official source label (`KSEAB`, `NCERT`, ...).
    pub source: String,
    /// Official page the file was found on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Known file size in bytes, used for progress and skip checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// On-disk representation of one board catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardCatalog {
    /// Board metadata.
    pub board: BoardInfo,
    /// Entries belonging to the board.
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// Merged, validated in-memory catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub(crate) boards: Vec<BoardCatalog>,
}

impl Catalog {
    /// All boards in deterministic id order.
    #[must_use]
    pub fn boards(&self) -> &[BoardCatalog] {
        &self.boards
    }

    /// Iterate over every entry together with its board metadata.
    pub fn entries(&self) -> impl Iterator<Item = CatalogEntry<'_>> {
        self.boards.iter().flat_map(|board| {
            board.entries.iter().map(move |entry| CatalogEntry {
                board: &board.board,
                entry,
            })
        })
    }

    /// Total number of entries across all boards.
    #[must_use]
    pub fn len(&self) -> usize {
        self.boards.iter().map(|board| board.entries.len()).sum()
    }

    /// Whether the catalog contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Find a board by id, case-insensitively.
    #[must_use]
    pub fn find_board(&self, id: &str) -> Option<&BoardCatalog> {
        self.boards
            .iter()
            .find(|board| board.board.id.eq_ignore_ascii_case(id.trim()))
    }

    pub(crate) fn sort(&mut self) {
        self.boards
            .sort_by(|left, right| left.board.id.cmp(&right.board.id));

        for board in &mut self.boards {
            board.entries.sort_by(|left, right| {
                canonical_level(&left.level)
                    .cmp(&canonical_level(&right.level))
                    .then_with(|| {
                        left.stream
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .cmp(&right.stream.as_deref().unwrap_or("").to_lowercase())
                    })
                    .then_with(|| {
                        left.subject
                            .to_lowercase()
                            .cmp(&right.subject.to_lowercase())
                    })
                    .then_with(|| left.kind.cmp(&right.kind))
                    .then_with(|| right.year.unwrap_or(0).cmp(&left.year.unwrap_or(0)))
                    .then_with(|| left.language.code().cmp(right.language.code()))
                    .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
                    .then_with(|| left.url.cmp(&right.url))
            });
        }
    }
}

/// Borrowed view of an entry plus its board.
#[derive(Debug, Clone, Copy)]
pub struct CatalogEntry<'a> {
    /// Board that owns the entry.
    pub board: &'a BoardInfo,
    /// The entry itself.
    pub entry: &'a Entry,
}

impl<'a> CatalogEntry<'a> {
    /// Stable board id.
    #[must_use]
    pub fn board_id(&self) -> &'a str {
        &self.board.id
    }

    /// Flattened, serialisable view used by `pu-get list --json` and the TUI.
    #[must_use]
    pub fn flat(self) -> FlatEntry<'a> {
        FlatEntry {
            board: &self.board.id,
            board_name: &self.board.name,
            entry: self.entry,
        }
    }
}

/// JSON-friendly entry with board metadata inlined.
#[derive(Debug, Clone, Serialize)]
pub struct FlatEntry<'a> {
    /// Board id.
    pub board: &'a str,
    /// Board display name.
    pub board_name: &'a str,
    /// Entry fields (`type`, `year`, ...) flattened alongside `board`.
    #[serde(flatten)]
    pub entry: &'a Entry,
}

/// Canonical level key used for filtering, sorting and de-duplication.
///
/// Karnataka levels stay distinct from CBSE class numbers (`sslc`, `puc1`,
/// `puc2` versus `10`, `11`, `12`), but common spellings are folded together.
/// Board-aware aliases (for example `11` -> `PUC 1` for the PUE board) are
/// handled by [`crate::catalog::filter::Filter`].
#[must_use]
pub fn canonical_level(value: &str) -> String {
    let compact: String = value
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();

    match compact.as_str() {
        "sslc" => "sslc".to_owned(),
        "puc1" | "pu1" | "1puc" | "ipuc" | "puci" | "pui" | "firstpuc" | "1stpuc" | "firstyear"
        | "1styear" => "puc1".to_owned(),
        "puc2" | "pu2" | "2puc" | "iipuc" | "pucii" | "puii" | "secondpuc" | "2ndpuc"
        | "secondyear" | "2ndyear" => "puc2".to_owned(),
        "class10" | "10" | "10th" | "tenth" | "x" => "10".to_owned(),
        "class11" | "11" | "11th" | "eleventh" | "xi" => "11".to_owned(),
        "class12" | "12" | "12th" | "twelfth" | "xii" => "12".to_owned(),
        other if other.starts_with("class") => {
            other.strip_prefix("class").unwrap_or(other).to_owned()
        }
        other if other.starts_with("standard") => {
            other.strip_prefix("standard").unwrap_or(other).to_owned()
        }
        other if other.starts_with("std") => other.strip_prefix("std").unwrap_or(other).to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> Entry {
        Entry {
            level: "SSLC".to_owned(),
            stream: None,
            subject: "Mathematics".to_owned(),
            kind: MaterialType::PreviousYearPaper,
            title: "SSLC Mathematics Previous Year Paper 2025".to_owned(),
            language: Language::Kn,
            year: Some(2025),
            url: "https://example.com/kseeb/maths-2025.pdf".to_owned(),
            source: "KSEAB".to_owned(),
            page: None,
            size_bytes: None,
        }
    }

    #[test]
    fn material_types_round_trip() {
        for kind in MaterialType::ALL {
            assert_eq!(kind.as_str().parse::<MaterialType>().unwrap(), kind);
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }

    #[test]
    fn material_type_aliases_are_accepted() {
        assert_eq!(
            "Previous Year Papers".parse::<MaterialType>().unwrap(),
            MaterialType::PreviousYearPaper
        );
        assert_eq!(
            "model-paper".parse::<MaterialType>().unwrap(),
            MaterialType::ModelPaper
        );
        assert_eq!(
            "marking_schemes".parse::<MaterialType>().unwrap(),
            MaterialType::MarkingScheme
        );
        assert!("guidebook".parse::<MaterialType>().is_err());
    }

    #[test]
    fn levels_are_canonicalised() {
        for alias in ["Class 10", "10th", "X", "SSLC"] {
            assert_eq!(
                canonical_level(alias),
                if alias == "SSLC" { "sslc" } else { "10" }
            );
        }
        for alias in ["PUC 1", "I PUC", "11th", "Class 11", "XI"] {
            assert!(
                matches!(canonical_level(alias).as_str(), "puc1" | "11"),
                "alias {alias} produced {}",
                canonical_level(alias)
            );
        }
        assert_eq!(canonical_level("PUC II"), "puc2");
        assert_eq!(canonical_level("Class 7"), "7");
    }

    #[test]
    fn flat_entry_serialises_board_and_type() {
        let board = BoardInfo {
            id: "kseeb".to_owned(),
            name: "Karnataka State Board".to_owned(),
        };
        let entry = sample_entry();
        let item = CatalogEntry {
            board: &board,
            entry: &entry,
        };

        let json = serde_json::to_value(item.flat()).unwrap();
        assert_eq!(json["board"], "kseeb");
        assert_eq!(json["board_name"], "Karnataka State Board");
        assert_eq!(json["type"], "previous_year_paper");
        assert_eq!(json["year"], 2025);
        assert_eq!(json["language"], "kn");
        assert_eq!(json["url"], "https://example.com/kseeb/maths-2025.pdf");
    }
}
