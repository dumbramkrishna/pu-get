# Adding a board, source or resolver

This guide is for contributors who want pu-get to cover another official board,
fix a moved portal, or add more material from an existing board. The UI derives
everything from catalog entries, so a new board normally needs **no UI changes**.

If you only want to check that a link is correct, use the
[source or broken-link issue form](https://github.com/dumbramkrishna/pu-get/issues/new?template=source_request.yml)
instead of a code change.

## 1. The source policy is not optional

pu-get only indexes material that is:

- published by an official board, government body or public institution; or
- published under an open licence that permits linking and downloading.

Do **not** add:

- rehosted paid guidebooks, notes or test-series PDFs;
- sites that mirror another board's papers without permission;
- login-walled material;
- scraped links whose source page you cannot identify;
- hand-written or guessed PDF URLs.

If a portal only exposes downloads through a postback, JavaScript-only flow or
login, report the limitation. Do not bypass it or fabricate a URL. See the
existing KSEAB postback handling and the PUE TLS note for examples of documented
limitations.

## 2. Choose the smallest integration that works

### A. The page is a simple list of direct PDF links

Use the built-in `generic_index` resolver. No Rust is needed: add a registry
entry and crawl it.

`generic_index`:

- fetches the base page;
- only emits direct `.pdf` links;
- follows a bounded number of same-host pages whose text matches `include_terms`;
- respects `max_pages`;
- infers titles, levels, years, languages and material types from link text where
  possible.

This is usually enough for a straightforward “question papers” page. If the
metadata is wrong or the page needs pagination/selectors, write a dedicated
resolver (option C).

### B. The portal already has a resolver, but you have another official page

Check the existing `resolver` values in
[`sources/registry.toml`](../sources/registry.toml). If the page uses the same
markup pattern as an existing source, you may only need to add a source entry.

Do not stretch an existing resolver over incompatible markup “until it mostly
works”. Deterministic parser tests are the deciding factor.

### C. The page needs its own parser

Add a resolver module under `src/resolver/`, plus a registry entry, a fixture and
tests. This is the right choice when the portal has structured tables, class
codes, chapter counts or year-specific pages.

## 3. Add (or update) a registry entry

Registry entries live in [`sources/registry.toml`](../sources/registry.toml).

```toml
[[source]]
id = "example-board-papers"          # stable id used by crawl --source
name = "Example Board"               # label written into entries
resolver = "generic_index"           # generic_index, ncert, cbse_academic, ...
board = "example"                    # generated catalog id
base_url = "https://papers.example/" # placeholder, not a real board
levels = ["10", "12"]
year_range = [2016, 2026]
status = "unverified"
include_terms = ["question paper", "model paper"]
max_pages = 3
```

Field rules enforced by `Registry::validate`:

| Field | Rule |
|---|---|
| `id` | unique, ASCII letters/digits/`-`/`_` |
| `name` | source label shown in the catalog |
| `resolver` | one of the supported resolver kinds |
| `board` | ASCII letters/digits/`-`/`_` |
| `base_url` | absolute HTTP(S) URL |
| `levels` | at least one non-empty level |
| `year_range` | inclusive, inside 1900-2100, start <= end |
| `status` | `verified` or `unverified` |
| `include_terms` | optional; used by `generic_index` |
| `max_pages` | 1-200; optional, defaults to 1 |

Keep `status = "unverified"` until you have actually validated the source. If two
sources contribute to the same board, use the same `board` id and they will be
merged into one generated catalog.

## 4. Generic index walkthrough

1. Add the entry with `status = "unverified"`.
2. Inspect what the resolver finds without writing or validating:

   ```sh
   cargo run -- crawl --source example-board-papers --no-validate --dry-run
   ```

3. Run the crawl and validate the URLs:

   ```sh
   cargo run -- crawl --source example-board-papers
   ```

4. Inspect `sources/generated/example.toml` and
   `sources/generated/crawl-report.txt`. Check titles, years, languages, levels
   and that the URLs are direct PDFs on the official host.
5. Check the catalog in the CLI and TUI:

   ```sh
   cargo run -- list --board example --limit 20
   cargo run
   ```

6. Once the index and files are confirmed, change `status = "verified"` and run
   the targeted crawl again so the generated catalog is from a verified source.

For a simple index, tuning `include_terms` and `max_pages` is often all that is
needed. Keep `max_pages` small; government portals can be slow and the crawl has a
bounded request model.

## 5. Writing a custom resolver

### 5.1 Create the module

Create `src/resolver/example.rs`. The following skeleton is deliberately compact;
replace the subject inference with real parsing from the page.

```rust
//! Resolver for the Example Board paper index.

use reqwest::Client;
use scraper::Selector;
use url::Url;

use super::{
    clean_text, extract_year, fetch_html, infer_level, infer_material_type, is_pdf_url,
    language_from_text, parse_document, resolve_url, stream_for_subject, Resolved,
    SourceDefinition,
};
use crate::catalog::Entry;
use crate::error::ResolverResult;

/// Fetch the official index and turn it into catalog entries.
pub async fn resolve(client: &Client, source: &SourceDefinition) -> ResolverResult<Resolved> {
    let base = source.base_url()?;
    let html = fetch_html(client, &base).await?;

    let mut resolved = Resolved {
        entries: parse_index(&html, &base, source),
        pages_fetched: 1,
        ..Resolved::default()
    };

    if resolved.entries.is_empty() {
        resolved.note("no direct PDF links were found on the index page");
    }
    resolved.dedupe();
    Ok(resolved)
}

/// Parse links from one saved/official page.
///
/// Keep this function public and free of network I/O so it can be tested against
/// `tests/fixtures/resolvers/example.html`.
#[must_use]
pub fn parse_index(html: &str, page_url: &Url, source: &SourceDefinition) -> Vec<Entry> {
    let document = parse_document(html);
    let anchors = Selector::parse("a[href]").expect("static selector");
    let mut entries = Vec::new();

    for anchor in document.select(&anchors) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(url) = resolve_url(page_url, href) else {
            continue;
        };
        if !is_pdf_url(&url) {
            continue;
        }

        let title = clean_text(&anchor.text().collect::<String>());
        if title.is_empty() {
            continue;
        }

        // Replace this with real subject parsing for the board. Falling back to
        // the title is only a starting point.
        let subject = title.clone();
        let stream = stream_for_subject(&subject);
        let year = extract_year(&title)
            .filter(|year| (source.year_range[0]..=source.year_range[1]).contains(year));

        entries.push(Entry {
            level: infer_level(&title)
                .or_else(|| source.levels.first().cloned())
                .unwrap_or_else(|| "General".to_owned()),
            stream,
            subject,
            kind: infer_material_type(&title),
            title,
            language: language_from_text(&title),
            year,
            url: url.to_string(),
            source: source.name.clone(),
            page: Some(page_url.to_string()),
            size_bytes: None,
        });
    }

    entries
}
```

The helpers used above are shared resolver utilities:

| Helper | Purpose |
|---|---|
| `fetch_html` / `fetch_optional_html` | bounded official-page fetch, with retry for transient failures |
| `parse_document` | `scraper` document construction |
| `resolve_url` | join and filter links to HTTP(S) |
| `is_pdf_url` | direct `.pdf` check |
| `clean_text` | whitespace normalisation |
| `extract_year` | find a plausible year |
| `infer_level` | `SSLC`, `PUC 1`, `PUC 2`, numeric class |
| `infer_material_type` | textbook/model/previous/sample/marking/syllabus |
| `language_from_text` | en/kn/hi/ur inference |
| `stream_for_subject` | Science/Commerce/Arts inference |
| `humanize_identifier` | readable fallback title from a filename |

Do not perform network I/O inside `parse_index`. Use `fetch_optional_html` for
year probes; a 404/410 is a normal “this year does not exist”, not an error.

### 5.2 Register the resolver

In `src/resolver/mod.rs`:

```rust
pub mod example;

pub enum ResolverKind {
    // ... existing variants
    Example,
}

// In resolve_source:
ResolverKind::Example => example::resolve(client, source).await,

// If this adds a new board id, add a display name:
// Add this arm to the existing `board_name` match:
"example" => "Example Board".to_owned(),
// The existing fallback capitalises unknown ids.
```

Set `resolver = "example"` in the registry. Serde renames snake-case enum
variants, so the registry value is the variant name in `snake_case`.

If the new board should appear near Karnataka in the TUI board list, add it to
`board_rank` in `src/app/state.rs`. Otherwise it sorts alphabetically after the
known boards. This is presentation only; no other UI change is needed.

## 6. Fixtures and parser tests

1. Save a representative **official** page as
   `tests/fixtures/resolvers/example.html`.
2. Strip personal data if present. Keep the markup that the parser relies on.
3. Add a test to `tests/resolvers_fixtures.rs`:

```rust
#[test]
fn parses_example_board_papers() {
    let registry = Registry::from_toml(
        "inline",
        r#"
[[source]]
id = "example-board-papers"
name = "Example Board"
resolver = "example"
board = "example"
base_url = "https://papers.example/"
levels = ["10", "12"]
year_range = [2016, 2026]
status = "unverified"
"#,
    )
    .unwrap();
    let source = registry.find("example-board-papers").unwrap();
    let page = Url::parse("https://papers.example/").unwrap();
    let html = include_str!("fixtures/resolvers/example.html");

    let entries = example::parse_index(html, &page, source);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].year, Some(2025));
    assert!(entries.iter().all(|entry| entry.url.ends_with(".pdf")));
}
```

Tests must never fetch the live portal. Local wiremock tests are for the HTTP and
download layers, not for real board pages.

Useful test cases:

- a page with no PDFs;
- a relative link;
- an off-host link;
- a postback/JavaScript link that must be ignored;
- a filename without a year;
- language variants;
- duplicate links;
- pagination when your resolver follows pages.

If the parser returns no entries for a valid page, keep the fixture and fix the
parser; do not replace it with a hand-written URL list.

## 7. Validate and regenerate the catalog

Targeted development:

```sh
# Show what would be discovered; no writes, no URL validation.
cargo run -- crawl --source example-board-papers --no-validate --dry-run

# Crawl and HEAD-validate; writes sources/generated/example.toml and a report.
cargo run -- crawl --source example-board-papers

# Re-check an already generated catalog without crawling.
cargo run -- verify --board example --limit 20
cargo run -- verify --board example --report target/example-verify.txt
```

Full crawl:

```sh
cargo run -- crawl
```

This is network-heavy and can take minutes because NCERT has thousands of
chapter URLs. Prefer targeted crawls while iterating.

What `crawl` does:

- runs verified sources by default; use `--include-unverified` or an explicit
  `--source` for unverified ones;
- probes URLs with HEAD and a ranged GET fallback;
- drops failing entries and records notes;
- writes one generated TOML per board and a crawl report;
- leaves an existing board catalog untouched if every source for that board fails.

The generated files are compiled into the binary by `rust-embed` from
`sources/generated/` at build time. Rebuild after a crawl to test the new data.

## 8. Marking a source verified

Change `status = "verified"` only when:

- the index is the official page you expect;
- direct PDF links are on the official host;
- the files open and are PDFs;
- the catalog metadata is sensible;
- you have run a targeted crawl with validation;
- the generated catalog and fixture/test are included in the contribution.

If a source is temporarily broken, keep it (or its last good generated catalog)
and document the problem rather than deleting evidence or disabling TLS. For PUE,
the generated catalog is seeded from the saved official page because the portal's
TLS chain is currently incomplete; see [sources/schema.md](../sources/schema.md).

## 9. User catalogs (no code)

If you need a private/experimental catalog immediately, place a `.toml` or `.json`
file in `<config-dir>/pu-get/catalogs/`. User catalogs merge over embedded entries
by URL. This is documented in
[Catalogs and updates](../README.md#catalogs-and-updates) and
[sources/schema.md](../sources/schema.md).

User catalogs are held to the same official/open-licence policy. They are not a
way to distribute paid or rehosted material.

## 10. Contribution checklist

- [ ] The source is official or openly licensed.
- [ ] No PDF URL was written by hand.
- [ ] A registry entry exists (with `status = "unverified"` initially).
- [ ] A custom parser is pure and public enough to test, if one was added.
- [ ] A saved official fixture is included.
- [ ] Fixture tests cover success and obvious failure modes.
- [ ] Targeted crawl and validation were run.
- [ ] The generated catalog is included and contains direct official PDFs.
- [ ] The source is marked `verified` only if confirmed.
- [ ] Board display name/ordering was considered.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --locked --all-targets -- -D warnings` passes.
- [ ] `cargo test --locked --all-targets` passes.
- [ ] README/schema/changelog updated if behavior or supported boards changed.
- [ ] The pull request follows
      [`.github/PULL_REQUEST_TEMPLATE.md`](../.github/PULL_REQUEST_TEMPLATE.md).

## 11. Maintaining sources after release

Official portals move files. When reports arrive:

1. Reproduce with `cargo run -- verify --board <board> --limit ...`.
2. Re-crawl the specific source without validation first to inspect it.
3. Update the parser and fixture if the markup changed.
4. Re-run a validating crawl and regenerate the board catalog.
5. Keep the source policy: never replace a dead official URL with an unofficial
   mirror.
