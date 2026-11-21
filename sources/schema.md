# Catalog, registry and update schemas

All catalogs are generated from official index pages by resolvers. PDF URLs are
never written by hand.

## Generated catalog (`sources/generated/<board>.toml`)

One file describes one board:

```toml
[board]
id = "kseeb"
name = "Karnataka State Board"

[[entries]]
level = "SSLC"                    # SSLC | PUC 1 | PUC 2 | 1-12
stream = "Science"                # optional: Science | Commerce | Arts
subject = "Mathematics"
type = "previous_year_paper"      # textbook | model_paper | previous_year_paper
                                  # sample_paper | marking_scheme | syllabus
title = "SSLC Mathematics Previous Year Paper 2025"
language = "kn"                   # en | kn | hi | ur
year = 2025                       # optional for undated textbooks
url = "https://..."               # direct PDF URL, discovered by a resolver
source = "KSEAB"                  # official source label
page = "https://..."              # official page where the file was found
size_bytes = 1234567              # optional, populated when known
```

Rules:

* Entries with an empty `url` are skipped by the loader.
* URLs must be absolute `http(s)` URLs.
* Years must be between 1900 and 2100.
* `board.id` must contain only ASCII letters, digits, `-` and `_`.
* User catalogs in `<config-dir>/pu-get/catalogs/*.toml|*.json` are merged over
  the embedded catalogs. Entries with the same URL are replaced; new boards are
  appended. JSON accepts a single board, a bare array, or `{ "boards": [...] }`.

## Source registry (`sources/registry.toml`)

```toml
[[source]]
id = "cbse-sqp"                   # stable CLI id
name = "CBSE Academic"            # source label written into entries
resolver = "cbse_academic"        # ncert | cbse_academic | kseab | pue | generic_index
board = "cbse"                    # board id for the generated catalog
base_url = "https://cbseacademic.nic.in/"
levels = ["10", "12"]
year_range = [2016, 2026]
status = "verified"               # verified | unverified
include_terms = ["syllabus"]      # optional, generic_index only
max_pages = 2                     # optional, index-following resolvers
```

`crawl` runs verified sources by default. Unverified sources are included with
`crawl --include-unverified`, or individually with `crawl --source <id>`.
Only official boards, government bodies, or openly licensed publishers belong
in the registry.

### Resolver notes

* **CBSE** probes `SQP_CLASSXII_<year>-<yy>.html` and
  `SQP_CLASSX_<year>-<yy>.html` for every year in the range. Missing years are
  normal and are not errors.
* **NCERT** parses the book codes, class/subject conditions and chapter counts
  embedded in `textbook.php`, then builds the chapter PDF URLs the page itself
  uses.
* **PUE** parses the I PUC / II PUC textbook table. The live portal currently
  serves an incomplete TLS chain, so `crawl` reports a certificate error. The
  checked-in `pue.toml` was seeded from the saved official page
  (`tests/fixtures/resolvers/pue_textbooks_full.html`) using the same resolver;
  regenerate with `crawl` once the portal's chain is fixed.
* **KSEAB** follows SSLC question-paper year pages and only emits direct PDF
  links. Recent KSEAB pages generate downloads through ASP.NET postbacks, which
  do not expose stable GET URLs; those pages are reported in
  `crawl-report.txt` instead of producing fabricated URLs.
* **generic_index** only emits direct PDF links and follows a bounded number of
  same-host pages whose text matches `include_terms`.

## Update manifest (`pu-get update`)

The manifest can be TOML or JSON:

```toml
schema_version = 1
catalog_version = "2026.11.21"
generated_at = "2026-11-21T12:00:00Z"          # optional
catalog_url = "https://raw.githubusercontent.com/.../catalog.json"
sha256 = "0123...abcdef"                       # lowercase SHA-256 of the catalog
```

`catalog_url` may be relative to the manifest URL. The catalog may be:

* a single-board TOML file (same schema as generated catalogs), or
* JSON containing one board, an array of boards, or `{ "boards": [...] }`.

`update` verifies the SHA-256 before parsing, validates the catalog, writes it
atomically to `<config-dir>/pu-get/catalogs/remote.{toml,json}`, and stores the
installed version in `<cache-dir>/pu-get/state/update-state.json`. A matching
version and checksum are treated as up to date unless `--force` is passed.
