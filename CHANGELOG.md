# Changelog

All notable changes to pu-get are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Nothing yet.

## [0.0.1] - 2026-11-21

Initial release.

### Added

- Interactive ratatui/crossterm TUI: splash, board, level, stream, subject and
  material-type selection, with a lightweight brand/breadcrumb header, calm
  selection styling, status/footer hints, grouped settings, a two-column
  shortcut overlay, resize handling and a minimum size of 80×24.
- PDF browser with fuzzy search (`/`), year filter (`y`), latest-only toggle (`t`,
  on by default), multi-select (`Space`, `a`), and download (`d`/`Enter`).
- Download queue with compact status rows, per-file progress meters, transfer
  speed, ETA, pause/resume, retry and cancel, concise retry messages, plus
  explicit skip/rename/overwrite/cancel handling for existing files.
- Settings screen for download directory, folder template, language preference,
  concurrency and latest-only default, persisted atomically to `config.toml`.
- Help overlay, `NO_COLOR`/`--no-color` support, and a clean non-TTY error message.
- Non-interactive CLI:
  - `list` with board/level/stream/subject/type/year/language/search filters,
    latest-only selection, limits and JSON output.
  - `get` with destination override, conflict policy, concurrency override,
    interactive confirmation, dry-run and resume-aware progress.
  - `config path|show|list|get|set`.
  - `verify` with bounded-concurrency HEAD/range-GET validation and reports.
  - `crawl` for official sources, generated catalogs and crawl reports.
  - `update` for checksum-verified remote catalogs, with `--check` and `--force`.
- Official-source resolver framework with registry, source status, year probing
  and saved-HTML fixture tests:
  - KSEAB SSLC question papers and model answers.
  - Karnataka PUE PUC 1/PUC 2 textbooks.
  - CBSE Academic sample papers and marking schemes.
  - NCERT textbook chapters, Classes 1-12.
  - Generic same-host PDF index resolver for new/unverified sources.
- Embedded generated catalogs: 6,844 entries total (CBSE 2,155; KSEEB 391;
  NCERT 4,141; PUE 157), plus user catalog overrides and remote catalog updates.
- Streaming download manager with bounded concurrency (default 3), exponential
  retry backoff, `Range`-based resume, PDF content-type and `%PDF` magic
  validation, `.part` files, atomic publication, filename sanitisation, folder
  templates and conflict policies.
- Configuration discovery through platform directories, `--config` /
  `PU_GET_CONFIG`, and `PU_GET_CACHE_DIR`.
- Documentation and community:
  - `README.md` with install, usage, keyboard shortcuts, configuration, catalog
    layering and a concise source-integration summary.
  - `docs/`: architecture, adding sources/resolvers, development, releasing and
    screenshot guides, plus a documentation index.
  - `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, issue forms for bugs,
    source requests and features, and a pull-request template.
  - Path-based PR labels (`.github/labeler.yml`), synchronized repository label
    definitions (`.github/labels.yml`) and automatic triage labels on issue forms.
  - A project commit convention with a contributor-editable type dictionary
    (`.github/commit-types.json`), a standard-library linter, a commit-msg hook,
    and CI checks for PR titles, commit ranges and the generated docs table.
  - README screenshots generated from the real TUI.
- One-line installers for Linux/macOS (`install.sh`) and Windows (`install.ps1`)
  with platform detection, release resolution, SHA-256 verification, dry runs and
  PATH hints.
- A `Binaries` workflow for downloading unsigned archives from GitHub Actions
  before a tag is published.
- MIT license, GitHub Actions CI and a tag-driven Linux/macOS/Windows release
  workflow with checksums.

### Notes and known limitations

- `pue.karnataka.gov.in` currently serves an incomplete TLS certificate chain.
  The PUE catalog is seeded from the saved official page using the PUE resolver.
  `crawl` reports the TLS error; TLS verification is not disabled.
- Recent KSEAB question-paper pages expose downloads only through ASP.NET
  postbacks. The catalog includes only stable direct PDF links; postback-only
  pages are reported in `crawl-report.txt`.
- The embedded catalogs are a snapshot. Official portals move and remove files;
  run `pu-get verify` to check links and `pu-get update`/`pu-get crawl` to refresh.
- Concurrency changes made in the TUI settings take effect the next time pu-get
  starts.
- Mouse input is not enabled; the TUI is keyboard-first.
- Minimum supported Rust version: **1.88**, enforced by CI.

[Unreleased]: https://github.com/dumbramkrishna/pu-get/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/dumbramkrishna/pu-get/releases/tag/v0.0.1
