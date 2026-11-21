//! Loading, validating and merging catalogs.
//!
//! Sources are layered:
//!
//! 1. The `sources/generated/*.toml` files embedded in the binary at build
//!    time (produced by `pu-get crawl`).
//! 2. Any `.toml`/`.json` files in the user catalog directory
//!    (`<config-dir>/pu-get/catalogs`), loaded in filename order. A later file
//!    overrides an earlier entry when it has the same URL.
//!
//! Empty URLs are skipped, so a resolver may emit placeholders that
//! `pu-get crawl` later fills in or drops.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::catalog::model::{BoardCatalog, Catalog, Entry};
use crate::config::AppPaths;
use crate::error::{CatalogError, CatalogResult};

#[derive(RustEmbed)]
#[folder = "sources/generated/"]
struct EmbeddedCatalogs;

/// Accepted JSON shapes for `pu-get update` payloads.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonPayload {
    /// A single board object.
    One(BoardCatalog),
    /// An object with a `boards` array.
    Wrapped { boards: Vec<BoardCatalog> },
    /// A bare array of boards.
    Many(Vec<BoardCatalog>),
}

impl Catalog {
    /// Load embedded catalogs and merge user overrides from `paths`.
    pub fn load(paths: &AppPaths) -> CatalogResult<Self> {
        let mut catalog = Self::load_embedded()?;
        let user = Self::from_dir(&paths.catalog_dir)?;
        catalog.merge(user);
        Ok(catalog)
    }

    /// Load the catalogs compiled into the binary.
    pub fn load_embedded() -> CatalogResult<Self> {
        let mut names: Vec<String> = EmbeddedCatalogs::iter()
            .map(|path| path.into_owned())
            .collect();
        names.sort();

        let mut catalog = Self::default();
        for name in names {
            if Path::new(&name).extension().and_then(OsStr::to_str) != Some("toml") {
                continue;
            }

            let Some(file) = EmbeddedCatalogs::get(&name) else {
                return Err(CatalogError::MissingEmbedded(name));
            };
            let origin = format!("embedded:{name}");
            let contents =
                std::str::from_utf8(file.data.as_ref()).map_err(|_| CatalogError::Utf8 {
                    origin: origin.clone(),
                })?;
            let board = parse_toml(&origin, contents)?;
            catalog.add_board(board, &origin)?;
        }

        catalog.sort();
        Ok(catalog)
    }

    /// Parse a single-board TOML catalog.
    pub fn from_toml(origin: impl Into<String>, contents: &str) -> CatalogResult<Self> {
        let origin = origin.into();
        let board = parse_toml(&origin, contents)?;

        let mut catalog = Self::default();
        catalog.add_board(board, &origin)?;
        catalog.sort();
        Ok(catalog)
    }

    /// Parse a JSON catalog: one board, `{ "boards": [...] }`, or an array.
    pub fn from_json(origin: impl Into<String>, contents: &str) -> CatalogResult<Self> {
        let origin = origin.into();
        let payload: JsonPayload =
            serde_json::from_str(contents).map_err(|source| CatalogError::Json {
                origin: origin.clone(),
                source,
            })?;

        let boards = match payload {
            JsonPayload::One(board) => vec![board],
            JsonPayload::Wrapped { boards } | JsonPayload::Many(boards) => boards,
        };

        let mut catalog = Self::default();
        for board in boards {
            catalog.add_board(board, &origin)?;
        }
        catalog.sort();
        Ok(catalog)
    }

    /// Read and parse one `.toml` or `.json` catalog file.
    pub fn from_file(path: &Path) -> CatalogResult<Self> {
        let origin = path.display().to_string();
        let data = fs::read(path).map_err(|source| CatalogError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let contents = String::from_utf8(data).map_err(|_| CatalogError::Utf8 {
            origin: origin.clone(),
        })?;

        match path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("toml") => Self::from_toml(origin, &contents),
            Some("json") => Self::from_json(origin, &contents),
            _ => Err(CatalogError::UnsupportedFormat {
                path: path.to_path_buf(),
            }),
        }
    }

    /// Load every catalog file in a directory, in filename order. A missing
    /// directory is not an error: it simply contributes no entries.
    pub fn from_dir(path: &Path) -> CatalogResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let read_dir = fs::read_dir(path).map_err(|source| CatalogError::ReadDir {
            path: path.to_path_buf(),
            source,
        })?;

        let mut files = Vec::new();
        for item in read_dir {
            let item = item.map_err(|source| CatalogError::ReadDir {
                path: path.to_path_buf(),
                source,
            })?;
            let file_type = item.file_type().map_err(|source| CatalogError::ReadDir {
                path: path.to_path_buf(),
                source,
            })?;
            if !file_type.is_file() {
                continue;
            }

            let file_path = item.path();
            let extension = file_path
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase);
            if matches!(extension.as_deref(), Some("toml" | "json")) {
                files.push(file_path);
            }
        }

        files.sort();

        let mut catalog = Self::default();
        for file in files {
            catalog.merge(Self::from_file(&file)?);
        }
        Ok(catalog)
    }

    /// Validate and merge a set of in-memory boards. The crawler uses this
    /// before serialising generated catalogs.
    pub fn from_boards(boards: Vec<BoardCatalog>) -> CatalogResult<Self> {
        let mut catalog = Self::default();
        for board in boards {
            catalog.add_board(board, "<memory>")?;
        }
        catalog.sort();
        Ok(catalog)
    }

    /// Merge another catalog into this one. Entries with the same URL are
    /// replaced by the incoming entry; new boards are appended.
    pub fn merge(&mut self, other: Self) {
        for board in other.boards {
            self.merge_board(board);
        }
        self.sort();
    }

    fn add_board(&mut self, mut board: BoardCatalog, origin: &str) -> CatalogResult<()> {
        validate_board(&mut board, origin)?;
        self.merge_board(board);
        Ok(())
    }

    fn merge_board(&mut self, board: BoardCatalog) {
        if let Some(existing) = self
            .boards
            .iter_mut()
            .find(|existing| existing.board.id == board.board.id)
        {
            if !board.board.name.is_empty() {
                existing.board.name = board.board.name;
            }
            for entry in board.entries {
                upsert_entry(&mut existing.entries, entry);
            }
        } else {
            self.boards.push(board);
        }
    }
}

fn parse_toml(origin: &str, contents: &str) -> CatalogResult<BoardCatalog> {
    toml::from_str(contents).map_err(|source| CatalogError::Toml {
        origin: origin.to_owned(),
        source,
    })
}

fn validate_board(board: &mut BoardCatalog, origin: &str) -> CatalogResult<()> {
    board.board.id = board.board.id.trim().to_ascii_lowercase();
    board.board.name = board.board.name.trim().to_owned();

    if board.board.id.is_empty() {
        return Err(CatalogError::EmptyBoardId {
            origin: origin.to_owned(),
        });
    }

    if !board
        .board
        .id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(CatalogError::InvalidBoardId {
            origin: origin.to_owned(),
            id: board.board.id.clone(),
        });
    }

    if board.board.name.is_empty() {
        return Err(CatalogError::EmptyBoardName {
            origin: origin.to_owned(),
        });
    }

    let entries = std::mem::take(&mut board.entries);
    let mut kept = Vec::with_capacity(entries.len());

    for (index, mut entry) in entries.into_iter().enumerate() {
        normalise_entry(&mut entry);

        // Resolvers may emit placeholders before validation; empty URLs are
        // explicitly skipped rather than treated as malformed.
        if entry.url.is_empty() {
            continue;
        }

        validate_entry(&entry, origin, index)?;
        upsert_entry(&mut kept, entry);
    }

    board.entries = kept;
    Ok(())
}

fn normalise_entry(entry: &mut Entry) {
    entry.level = entry.level.trim().to_owned();
    entry.subject = entry.subject.trim().to_owned();
    entry.title = entry.title.trim().to_owned();
    entry.source = entry.source.trim().to_owned();
    entry.url = entry.url.trim().to_owned();
    entry.stream = clean_optional(entry.stream.take());
    entry.page = clean_optional(entry.page.take());
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn validate_entry(entry: &Entry, origin: &str, index: usize) -> CatalogResult<()> {
    for (field, value) in [
        ("level", entry.level.as_str()),
        ("subject", entry.subject.as_str()),
        ("title", entry.title.as_str()),
        ("source", entry.source.as_str()),
    ] {
        if value.is_empty() {
            return Err(CatalogError::MissingField {
                origin: origin.to_owned(),
                index,
                field,
            });
        }
    }

    if let Some(year) = entry.year {
        if !(1900..=2100).contains(&year) {
            return Err(CatalogError::InvalidYear {
                origin: origin.to_owned(),
                index,
                year,
            });
        }
    }

    if !is_http_url(&entry.url) {
        return Err(CatalogError::InvalidField {
            origin: origin.to_owned(),
            index,
            field: "url",
            value: entry.url.clone(),
        });
    }

    if let Some(page) = &entry.page {
        if !is_http_url(page) {
            return Err(CatalogError::InvalidField {
                origin: origin.to_owned(),
                index,
                field: "page",
                value: page.clone(),
            });
        }
    }

    Ok(())
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn upsert_entry(entries: &mut Vec<Entry>, entry: Entry) {
    if let Some(existing) = entries
        .iter_mut()
        .find(|candidate| candidate.url == entry.url)
    {
        *existing = entry;
    } else {
        entries.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VALID: &str = r#"
[board]
id = "  KSEEB  "
name = "  Karnataka State Board  "

[[entries]]
level = " SSLC "
subject = " Mathematics "
type = "previous_year_paper"
title = " Mathematics Paper 2025 "
language = "kn"
year = 2025
url = " https://example.com/kseeb/maths-2025.pdf "
source = " KSEAB "

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "textbook"
title = "Placeholder awaiting crawl"
url = ""
source = "KTBS"
"#;

    #[test]
    fn trims_values_and_skips_empty_urls() {
        let catalog = Catalog::from_toml("test", VALID).unwrap();
        assert_eq!(catalog.boards().len(), 1);

        let board = &catalog.boards()[0];
        assert_eq!(board.board.id, "kseeb");
        assert_eq!(board.board.name, "Karnataka State Board");
        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].title, "Mathematics Paper 2025");
        assert_eq!(
            board.entries[0].url,
            "https://example.com/kseeb/maths-2025.pdf"
        );
    }

    #[test]
    fn later_entries_with_the_same_url_win() {
        let text = r#"
[board]
id = "kseeb"
name = "Karnataka State Board"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "Old title"
year = 2025
url = "https://example.com/paper.pdf"
source = "KSEAB"

[[entries]]
level = "SSLC"
subject = "Mathematics"
type = "previous_year_paper"
title = "Corrected title"
year = 2025
url = "https://example.com/paper.pdf"
source = "KSEAB"
"#;

        let catalog = Catalog::from_toml("test", text).unwrap();
        let board = &catalog.boards()[0];
        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].title, "Corrected title");
    }

    #[test]
    fn rejects_malformed_entries() {
        let bad_url = VALID.replace("https://example.com/kseeb/maths-2025.pdf", "not-a-url");
        assert!(Catalog::from_toml("bad-url", &bad_url).is_err());

        let empty_title = VALID.replace("Mathematics Paper 2025", " ");
        assert!(Catalog::from_toml("empty-title", &empty_title).is_err());

        let bad_year = VALID.replace("year = 2025", "year = 1800");
        assert!(Catalog::from_toml("bad-year", &bad_year).is_err());
    }

    #[test]
    fn rejects_invalid_board_ids() {
        let text = r#"
[board]
id = "../escape"
name = "Bad"
"#;
        assert!(Catalog::from_toml("bad-board", text).is_err());
    }

    #[test]
    fn parses_json_arrays_and_wrapped_objects() {
        let board = r#"{
            "board": { "id": "cbse", "name": "CBSE" },
            "entries": [{
                "level": "12",
                "subject": "Chemistry",
                "type": "sample_paper",
                "title": "Chemistry Sample Paper",
                "language": "en",
                "year": 2026,
                "url": "https://example.com/cbse/chemistry.pdf",
                "source": "CBSE"
            }]
        }"#;

        let array = format!("[{board}]");
        assert_eq!(Catalog::from_json("array", &array).unwrap().len(), 1);

        let wrapped = format!(r#"{{ "boards": [{board}] }}"#);
        assert_eq!(Catalog::from_json("wrapped", &wrapped).unwrap().len(), 1);

        assert_eq!(Catalog::from_json("one", board).unwrap().len(), 1);
    }

    #[test]
    fn unsupported_extensions_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.txt");
        fs::write(&path, VALID).unwrap();
        assert!(matches!(
            Catalog::from_file(&path),
            Err(CatalogError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn missing_catalog_directory_is_empty() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = Catalog::from_dir(&directory.path().join("missing")).unwrap();
        assert!(catalog.is_empty());
    }
}
