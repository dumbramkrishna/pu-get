//! Fixture-based integration tests for every resolver and the curated registry.

use std::path::Path;

use pu_get::catalog::MaterialType;
use pu_get::config::Language;
use pu_get::resolver::{
    self, cbse_academic, generic_index, kseab, ncert, pue, Registry, ResolverKind,
    SourceDefinition, SourceStatus,
};
use url::Url;

fn registry() -> Registry {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("sources/registry.toml");
    Registry::load(Some(path.as_path())).expect("registry loads")
}

fn source(id: &str) -> SourceDefinition {
    registry().find(id).expect("source exists").clone()
}

fn generic_source() -> SourceDefinition {
    let registry = Registry::from_toml(
        "inline",
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
fn registry_contains_verified_and_unverified_official_sources() {
    let registry = registry();
    assert_eq!(
        registry.find("cbse-sqp").unwrap().status,
        SourceStatus::Verified
    );
    assert_eq!(
        registry.find("ncert-textbooks").unwrap().status,
        SourceStatus::Verified
    );
    assert_eq!(
        registry.find("pue-textbooks").unwrap().status,
        SourceStatus::Verified
    );
    assert_eq!(
        registry.find("kseab-sslc-question-papers").unwrap().status,
        SourceStatus::Verified
    );

    for id in [
        "ktbs-textbooks",
        "dsert-materials",
        "nios-materials",
        "diksha-materials",
        "epathshala-materials",
        "swayam-courses",
        "nptel-courses",
    ] {
        assert_eq!(
            registry.find(id).unwrap().status,
            SourceStatus::Unverified,
            "{id}"
        );
    }

    assert!(registry
        .sources
        .iter()
        .all(|source| source.resolver != ResolverKind::GenericIndex
            || !source.include_terms.is_empty()));
}

#[test]
fn pue_table_is_parsed() {
    let page = Url::parse("https://pue.karnataka.gov.in/new-page/TEXT%20BOOKS/kn").unwrap();
    let resolved = pue::parse_index(
        include_str!("fixtures/resolvers/pue_textbooks.html"),
        &page,
        &source("pue-textbooks"),
    );

    assert!(resolved.entries.len() >= 10);
    assert!(resolved
        .entries
        .iter()
        .any(|entry| entry.level == "PUC 1" && entry.subject == "Physics"));
    assert!(resolved.entries.iter().any(|entry| entry.level == "PUC 2"
        && entry.subject == "Business Studies"
        && entry.stream.as_deref() == Some("Commerce")));
    assert!(resolved
        .entries
        .iter()
        .any(|entry| entry.language == Language::Kn));
}

#[test]
fn real_pue_page_seed_fixture_parses() {
    let page = Url::parse("https://pue.karnataka.gov.in/new-page/TEXT%20BOOKS/kn").unwrap();
    let resolved = pue::parse_index(
        include_str!("fixtures/resolvers/pue_textbooks_full.html"),
        &page,
        &source("pue-textbooks"),
    );

    assert!(
        resolved.entries.len() >= 80,
        "{} entries",
        resolved.entries.len()
    );
    assert!(resolved.entries.iter().any(|entry| entry.level == "PUC 1"));
    assert!(resolved.entries.iter().any(|entry| entry.level == "PUC 2"));
    assert!(resolved.entries.iter().all(|entry| entry
        .url
        .starts_with("https://pue.karnataka.gov.in/storage/pdf-files/")));
}

#[test]
fn cbse_sample_papers_are_parsed() {
    let page = Url::parse("https://cbseacademic.nic.in/SQP_CLASSXII_2026-27.html").unwrap();
    let entries = cbse_academic::parse_page(
        include_str!("fixtures/resolvers/cbse_class12.html"),
        &page,
        "12",
        2026,
        &source("cbse-sqp"),
    );

    assert_eq!(entries.len(), 6);
    assert!(entries.iter().any(|entry| {
        entry.subject == "Accountancy" && entry.kind == MaterialType::SamplePaper
    }));
    assert!(entries.iter().any(|entry| {
        entry.subject == "Biology"
            && entry.kind == MaterialType::MarkingScheme
            && entry.language == Language::Hi
    }));
}

#[test]
fn kseab_index_and_direct_pdfs_are_parsed() {
    let index_page =
        Url::parse("https://kseab.karnataka.gov.in/new-page/SSLC%20QUESTION%20PAPERS/en").unwrap();
    let pages = kseab::parse_index(
        include_str!("fixtures/resolvers/kseab_index.html"),
        &index_page,
        &[2016, 2026],
    );
    assert!(pages.iter().any(|page| page.year == 2026));
    assert!(pages.iter().any(|page| page.year == 2022));

    let direct_page = Url::parse(
        "https://kseab.karnataka.gov.in/new-page/2022SSLCMainExaminationQuestionpapers/en",
    )
    .unwrap();
    let (entries, postback) = kseab::parse_question_page(
        include_str!("fixtures/resolvers/kseab_2022.html"),
        &direct_page,
        2022,
        "March/April 2022 SSLC Main Examination Question Paper",
        &source("kseab-sslc-question-papers"),
    );
    assert_eq!(entries.len(), 4);
    assert!(!postback);
    assert!(entries
        .iter()
        .all(|entry| entry.kind == MaterialType::PreviousYearPaper));
    assert!(entries
        .iter()
        .any(|entry| entry.subject == "Mathematics" && entry.title.contains("81 K/E")));

    let postback_page = Url::parse("https://kseeb.karnataka.gov.in/QP/SSLC2025_EXAM1_QP").unwrap();
    let (entries, postback) = kseab::parse_question_page(
        include_str!("fixtures/resolvers/kseab_postback.html"),
        &postback_page,
        2025,
        "SSLC 2025 EXAM-1 QUESTION PAPERS",
        &source("kseab-sslc-question-papers"),
    );
    assert!(entries.is_empty());
    assert!(postback);

    let model_page = Url::parse(
        "https://kseab.karnataka.gov.in/new-page/JUNE%20JULY-2020-SCIENCE%20MODEL-ANSWERS/en",
    )
    .unwrap();
    let model_entries = kseab::parse_model_answer_page(
        include_str!("fixtures/resolvers/kseab_model_answers.html"),
        &model_page,
        2020,
        &source("kseab-sslc-question-papers"),
    );
    assert_eq!(model_entries.len(), 2);
    assert!(model_entries
        .iter()
        .all(|entry| entry.kind == MaterialType::MarkingScheme));
}

#[test]
fn ncert_book_codes_and_chapters_are_parsed() {
    let page = Url::parse("http://ncert.nic.in/textbook.php").unwrap();
    let resolved = ncert::parse_index(
        include_str!("fixtures/resolvers/ncert_textbook.html"),
        &page,
        &source("ncert-textbooks"),
    );

    assert_eq!(resolved.entries.len(), 127);
    assert!(resolved.entries.iter().any(|entry| {
        entry.url == "http://ncert.nic.in/textbook/pdf/leph101.pdf"
            && entry.level == "12"
            && entry.subject == "Physics"
            && entry.language == Language::En
    }));
    assert!(resolved
        .entries
        .iter()
        .any(|entry| entry.url.ends_with("jusc116.pdf") && entry.language == Language::Ur));
}

#[test]
fn generic_index_stays_on_host_and_classifies_pdfs() {
    let page = Url::parse("https://example.com/").unwrap();
    let parsed = generic_index::parse_page(
        include_str!("fixtures/resolvers/generic_index.html"),
        &page,
        &generic_source(),
    );

    assert_eq!(parsed.entries.len(), 3);
    assert!(parsed
        .entries
        .iter()
        .any(|entry| entry.kind == MaterialType::Syllabus
            && entry.subject == "Science"
            && entry.level == "10"));
    assert_eq!(parsed.follow.len(), 1);
    assert_eq!(resolver::board_name("kseeb"), "Karnataka State Board");
}
