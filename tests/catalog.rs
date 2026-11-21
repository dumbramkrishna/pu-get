//! Integration tests for catalog parsing, merging and filtering.
//!
//! All fixtures use the reserved `example.com` domain; they are synthetic and
//! never represent real download URLs.

use std::fs;
use std::path::{Path, PathBuf};

use pu_get::catalog::{Catalog, Filter, MaterialType};
use pu_get::config::{AppPaths, Language};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog")
}

fn fixture(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn fixture_catalog() -> Catalog {
    Catalog::from_dir(&fixture_dir()).unwrap()
}

fn find<'a>(catalog: &'a Catalog, title: &str) -> pu_get::catalog::CatalogEntry<'a> {
    catalog
        .entries()
        .find(|item| item.entry.title == title)
        .unwrap_or_else(|| panic!("entry `{title}` not found"))
}

#[test]
fn parses_all_fixture_boards_and_skips_empty_urls() {
    let catalog = fixture_catalog();

    assert_eq!(catalog.boards().len(), 4);
    assert_eq!(catalog.len(), 13);

    let kseeb = catalog.find_board("kseeb").unwrap();
    assert_eq!(kseeb.entries.len(), 3);
    assert!(
        kseeb
            .entries
            .iter()
            .all(|entry| entry.title != "SSLC Mathematics Textbook"),
        "entry with an empty url must be skipped"
    );
}

#[test]
fn embedded_catalogs_are_available() {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(
        directory.path().join("config.toml"),
        directory.path().join("cache"),
    );

    let catalog = Catalog::load(&paths).unwrap();
    for board in ["kseeb", "pue", "cbse", "ncert"] {
        assert!(catalog.find_board(board).is_some(), "missing board {board}");
    }
}

#[test]
fn metadata_survives_parsing() {
    let catalog = fixture_catalog();
    let entry = find(&catalog, "SSLC Mathematics Previous Year Paper 2025");

    assert_eq!(entry.board.id, "kseeb");
    assert_eq!(entry.board.name, "Karnataka State Board");
    assert_eq!(entry.entry.level, "SSLC");
    assert_eq!(entry.entry.subject, "Mathematics");
    assert_eq!(entry.entry.kind, MaterialType::PreviousYearPaper);
    assert_eq!(entry.entry.language, Language::Kn);
    assert_eq!(entry.entry.year, Some(2025));
    assert_eq!(entry.entry.source, "KSEAB");
    assert_eq!(
        entry.entry.page.as_deref(),
        Some("https://example.com/kseeb/question-papers")
    );
}

#[test]
fn user_catalog_directory_merges_with_embedded_catalogs() {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(
        directory.path().join("pu-get").join("config.toml"),
        directory.path().join("cache"),
    );
    paths.ensure_config_dirs().unwrap();
    fs::copy(fixture("kseeb.toml"), paths.catalog_dir.join("kseeb.toml")).unwrap();

    let catalog = Catalog::load(&paths).unwrap();
    let entry = find(&catalog, "SSLC Mathematics Previous Year Paper 2025");
    assert_eq!(entry.board.id, "kseeb");
    assert_eq!(
        entry.entry.url,
        "https://example.com/kseeb/sslc-maths-2025.pdf"
    );
}

#[test]
fn later_catalog_overrides_entries_by_url() {
    let mut catalog = Catalog::from_file(&fixture("kseeb.toml")).unwrap();
    let override_catalog = Catalog::from_file(&fixture("override/kseeb.toml")).unwrap();
    catalog.merge(override_catalog);

    let kseeb = catalog.find_board("kseeb").unwrap();
    assert_eq!(kseeb.entries.len(), 4);

    let corrected = find(
        &catalog,
        "SSLC Mathematics Previous Year Paper 2025 (corrected)",
    );
    assert_eq!(corrected.entry.year, Some(2025));

    let added = find(&catalog, "SSLC Mathematics Model Paper 2026");
    assert_eq!(added.entry.year, Some(2026));
}

#[test]
fn latest_only_keeps_newest_dated_entry() {
    let catalog = fixture_catalog();

    let all = Filter {
        board: Some("kseeb".to_owned()),
        level: Some("10".to_owned()),
        subject: Some("math".to_owned()),
        kind: Some(MaterialType::PreviousYearPaper),
        ..Filter::default()
    };
    assert_eq!(all.apply(&catalog).len(), 2);

    let latest = Filter {
        latest_only: true,
        ..all
    };
    let results = latest.apply(&catalog);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entry.year, Some(2025));
}

#[test]
fn undated_entries_survive_latest_only() {
    let catalog = fixture_catalog();
    let filter = Filter {
        board: Some("ncert".to_owned()),
        latest_only: true,
        ..Filter::default()
    };

    assert_eq!(filter.apply(&catalog).len(), 2);
}

#[test]
fn karnataka_level_aliases_are_board_aware() {
    let catalog = fixture_catalog();

    for level in ["SSLC", "10", "Class 10"] {
        let filter = Filter {
            board: Some("kseeb".to_owned()),
            level: Some(level.to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.apply(&catalog).len(), 3, "level alias {level}");
    }

    for level in ["11", "PUC 1", "I PUC", "Class 11"] {
        let filter = Filter {
            board: Some("pue".to_owned()),
            level: Some(level.to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.apply(&catalog).len(), 2, "level alias {level}");
    }
}

#[test]
fn stream_subject_language_and_search_filters_combine() {
    let catalog = fixture_catalog();

    let arts = Filter {
        board: Some("pue".to_owned()),
        stream: Some("Arts".to_owned()),
        ..Filter::default()
    };
    assert_eq!(arts.apply(&catalog).len(), 1);

    let kannada = Filter {
        board: Some("pue".to_owned()),
        language: Some(Language::Kn),
        ..Filter::default()
    };
    assert_eq!(kannada.apply(&catalog).len(), 3);

    let accountancy = Filter {
        search: Some("accountancy commerce".to_owned()),
        ..Filter::default()
    };
    assert_eq!(accountancy.apply(&catalog).len(), 1);
}

#[test]
fn flat_json_contains_board_and_type() {
    let catalog = fixture_catalog();
    let entry = find(
        &catalog,
        "CBSE Class 12 Chemistry Sample Question Paper 2026-27",
    );

    let value = serde_json::to_value(entry.flat()).unwrap();
    assert_eq!(value["board"], "cbse");
    assert_eq!(value["board_name"], "CBSE");
    assert_eq!(value["type"], "sample_paper");
    assert_eq!(value["year"], 2026);
    assert_eq!(value["language"], "en");
}

#[test]
fn loading_is_deterministic() {
    assert_eq!(fixture_catalog(), fixture_catalog());
}
