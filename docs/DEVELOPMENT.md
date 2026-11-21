# Development guide

## Toolchain

- Rust **1.88 or newer** (stable recommended).
- Edition 2021.
- The MSRV is enforced by the `msrv` CI job with Rust 1.88.
- `rustfmt` and `clippy` are required for the full local gate.

Clone and build:

```sh
git clone https://github.com/dumbramkrishna/pu-get
cd pu-get
cargo build --locked
```

The binary is `target/debug/pu-get`; `cargo run -- …` is equivalent.

## Everyday commands

```sh
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

Run the TUI:

```sh
cargo run
cargo run -- --no-color
```

Run a non-interactive command:

```sh
cargo run -- list --board kseeb --level SSLC --limit 10
cargo run -- get --board ncert --level 12 --subject Physics --dry-run
```

Check a commit message or the current branch against the project convention:

```sh
python3 scripts/commit_lint.py --message "tui(queue): shorten retry messages"
python3 scripts/commit_lint.py --range origin/main..HEAD
python3 -m unittest discover -s scripts -p 'test_*.py'
```

See [Commit messages](COMMITS.md) for the type dictionary and how to add a new
type.

Regenerate the README screenshots:

```sh
cargo run --example screenshots
```

## Installers and release artifacts

The Linux/macOS installer is POSIX `sh` and must stay compatible with `dash`:

```sh
sh -n install.sh
./install.sh --help
./install.sh --version v0.0.1 --dir /tmp/pu-get-test --dry-run
```

The Windows installer is PowerShell 5.1+:

```powershell
.\install.ps1 -Version v0.0.1 -InstallDir "$env:TEMP\pu-get-test" -DryRun
```

Archive names are `pu-get-<tag>-<target>.tar.gz` (Unix) and
`pu-get-<tag>-<target>.zip` (Windows), with a release-level `SHA256SUMS.txt`.
Keep `install.sh`, `install.ps1`, `release.yml` and `binaries.yml` in sync when
the naming changes.

To produce a local Linux artifact without GitHub Actions:

```sh
version=v0.0.1
target=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release --locked --target "$target"
mkdir -p "dist/pu-get-$version-$target"
cp "target/$target/release/pu-get" README.md LICENSE CHANGELOG.md \
  "dist/pu-get-$version-$target/"
tar -czf "dist/pu-get-$version-$target.tar.gz" -C dist "pu-get-$version-$target"
(cd dist && sha256sum "pu-get-$version-$target.tar.gz" > SHA256SUMS.txt)
```

`dist/` is ignored by git. Use the manual **Binaries** workflow when you need
all four targets. These archives are unsigned, and no release is created.

## Isolating development state

The TUI and CLI read the normal platform config/catalog directories. To avoid
touching them:

```sh
mkdir -p /tmp/pu-get-dev
PU_GET_CACHE_DIR=/tmp/pu-get-dev/cache \
  cargo run -- --config /tmp/pu-get-dev/config.toml
```

`--config` changes the config file and user-catalog directory. `PU_GET_CONFIG` is
the environment equivalent. `PU_GET_CACHE_DIR` changes cache/state (including the
update state). The download directory is whatever is in the config, so set
`download_dir` in the temporary config when testing downloads.

Useful settings while developing:

```sh
cargo run -- config set download_dir '/tmp/pu-get-dev/downloads'
cargo run -- config set concurrency 2
cargo run -- config set latest_only false
```

The config-path command respects the same overrides:

```sh
cargo run -- config path
```

## TUI development

The TUI is split into:

- `src/app/state.rs` — navigation, selection, filters, queue and settings state;
- `src/app/events.rs` — key handling and input-mode dispatch;
- `src/app/mod.rs` — terminal setup, tokio event loop, manager channel;
- `src/ui/` — pure screen rendering.

Rules:

- Rendering takes `&App` and never performs I/O or mutates state.
- Keep the interface usable at **80×24**.
- Honour `NO_COLOR` and `--no-color`.
- Keep the accent single and the selection quiet enough for dense tables.
- Do not introduce mouse-only actions.
- Search/settings text inputs must consume printable keys before global shortcuts.

Rendering tests use `ratatui::backend::TestBackend` at 80×24. If you change layout,
add or update a test for the relevant screen and regenerate the screenshots.

## Testing

The suite is offline by design.

```sh
cargo test --locked --all-targets
```

Layers:

| Area | Location |
|---|---|
| Catalog model/loader/filter | `src/catalog/*`, `tests/catalog.rs` |
| Download engine | `tests/download_mock.rs` with a local wiremock server |
| Resolver parsers | `tests/resolvers_fixtures.rs`, `tests/fixtures/resolvers/` |
| Verify/update | `tests/verify_mock.rs`, `tests/update_mock.rs` |
| TUI render | unit tests in `src/ui/mod.rs` |
| CLI parsing | unit tests in `src/cli.rs` |

Guidelines:

- Never add a test that fetches a real board portal.
- A parser test needs a fixture and assertions about real output, not just “no
  panic”.
- Prefer pure parser functions so fixtures can exercise them directly.
- Wiremock pitfall: `set_body_string` forces `Content-Type: text/plain`; use
  `set_body_bytes` when testing PDF responses. In wiremock 0.6, HEAD mocks may
  report `Content-Length: 0`, so set the header explicitly when that matters.
- If a portal is flaky, test the parser with the fixture and handle the network
  behavior separately. Do not add sleeps or live retries to the test suite.

Run one test while iterating:

```sh
cargo test --locked --lib ui::tests::queue_screen_renders_progress_rows
cargo test --locked --test resolvers_fixtures
```

## Resolver and crawl development

Read [Adding sources](ADDING_SOURCES.md) first.

Targeted commands:

```sh
# Inspect without writing files or validating URLs.
cargo run -- crawl --source kseab-sslc-question-papers --no-validate --dry-run

# Crawl and validate one source.
cargo run -- crawl --source cbse-sqp

# Check existing catalog URLs.
cargo run -- verify --board cbse --limit 20 --concurrency 4
```

Full crawls are network-heavy. The NCERT chapter validation is usually the slow
part. Use `--source`, `--limit`, and `--out target/...` while iterating.

The generated files under `sources/generated/` are embedded by `rust-embed` at
compile time. Rebuild after a crawl. `crawl-report.txt` is a local runtime
artifact and is ignored by git.

## Logging and debugging

CLI commands accept `-v`, `-vv`, `-vvv` and honour `RUST_LOG`. Logs go to stderr so
stdout stays usable for tables and JSON.

```sh
cargo run -- -vv list --board cbse --limit 1
RUST_LOG=pu_get=debug cargo run -- verify --board cbse --limit 2
```

The TUI disables tracing while it owns the alternate screen; writing logs there
would corrupt the display. TUI errors appear in the status bar or as a final
`pu-get: error: …` after the terminal is restored.

`--dry-run` for `get` prints resolved destinations without downloading. `--dry-run`
for `crawl` reports discovery without writing. Both are preferable to poking at
real directories while debugging.

## Common issues

- **PUE TLS:** `pue.karnataka.gov.in` serves an incomplete certificate chain.
  rustls correctly refuses it. Do not disable verification. The checked-in PUE
  catalog is generated from the saved official page by the PUE resolver.
- **KSEAB recent papers:** some pages expose downloads only through ASP.NET
  postbacks. The resolver reports this instead of inventing GET URLs.
- **Catalog links go stale:** official portals move files. Run `pu-get verify` on
  a filtered subset, then re-crawl the source.
- **Slow full crawl:** crawl one source at a time; NCERT validation dominates.
- **Terminal too small:** the UI requires 80×24. Resize, or use `list`/`get`.
- **No colour in CI:** `NO_COLOR` and `--no-color` intentionally remove colour;
  selection still uses reverse video.
- **MSRV failure:** run `cargo +1.88.0 check --locked --all-targets`. The lockfile
  is committed and CI uses `--locked`.

## Documentation and screenshots

- User-facing behavior: `README.md`
- Contributor docs: `docs/`
- Catalog/update schema: `sources/schema.md`
- Release notes: `CHANGELOG.md`
- Screenshot generator: `examples/screenshots.rs`

When changing the UI, run `cargo run --example screenshots` and check the SVGs in
`docs/screenshots/`. When changing supported boards, catalog behavior or release
artifacts, update the README/CHANGELOG as appropriate.

## Before opening a pull request

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

Then read [CONTRIBUTING.md](../CONTRIBUTING.md) and use the PR template.
