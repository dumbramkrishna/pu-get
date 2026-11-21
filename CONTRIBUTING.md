# Contributing to pu-get

Thanks for helping make official study material easier to find. Contributions of
code, documentation, source coverage, tests and careful bug reports are all
welcome.

pu-get is an index and downloader, not a file host. Everything we add must point at
an official board/government source or material published under an open licence.

## Quick start

Requirements:

- Rust **1.88 or newer** (stable recommended)
- `rustfmt` and `clippy` components
- `git`

```sh
git clone https://github.com/dumbramkrishna/pu-get
cd pu-get
cargo build --locked
cargo test --locked --all-targets
cargo run
```

Useful guides:

- [Architecture](docs/ARCHITECTURE.md)
- [Adding sources and resolvers](docs/ADDING_SOURCES.md)
- [Development guide](docs/DEVELOPMENT.md)
- [Releasing](docs/RELEASING.md)
- [Catalog/update schema](sources/schema.md)

## Ways to contribute

- Report a bug with the
  [bug form](https://github.com/dumbramkrishna/pu-get/issues/new?template=bug_report.yml).
- Request a board/source or report a broken link with the
  [source form](https://github.com/dumbramkrishna/pu-get/issues/new?template=source_request.yml).
- Propose a feature with the
  [feature form](https://github.com/dumbramkrishna/pu-get/issues/new?template=feature_request.yml).
- Improve docs, examples, fixtures or tests.
- Add a resolver for an official board.
- Improve the TUI while keeping it keyboard-first and usable at 80×24.
- Report security issues privately; see [SECURITY.md](SECURITY.md).

Open an issue before a large change, especially a new resolver or a change to
catalog semantics. Small documentation and test fixes can go straight to a pull
request.

## The source policy

This is the most important rule in the project.

**Allowed**

- Official boards, government bodies and public institutions.
- Material published under an open licence that permits linking/downloading.
- Official pages that expose direct, public PDF URLs.

**Not allowed**

- Rehosted paid guidebooks, coaching notes or test-series PDFs.
- Unlicensed mirrors of another board's papers.
- Login-walled, paywalled or access-controlled material.
- Guessed or hand-written PDF URLs.
- Disabling TLS verification or bypassing a portal's access controls.

Additional rules:

1. **Never hand-write a PDF URL.** Resolvers discover links from official pages.
2. **Use saved HTML fixtures** for parser tests. Tests must not fetch a live board
   portal.
3. **Mark new sources `unverified`** until the index and files have been checked.
4. **Document limitations** (postbacks, missing years, incomplete TLS chains)
   instead of inventing a workaround.
5. **Keep generated catalogs in sync** with the resolver and fixture that produced
   them.

The full walkthrough is in [docs/ADDING_SOURCES.md](docs/ADDING_SOURCES.md).

## Local quality gate

Run these before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

For changes to the commit convention or its docs:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/commit_lint.py --check-docs docs/COMMITS.md
python3 scripts/commit_lint.py --range origin/main..HEAD
```

If you touch the MSRV-sensitive surface, also run:

```sh
cargo +1.88.0 check --locked --all-targets
```

The suite is intentionally offline. Network tests for downloads and verification
use a local wiremock server; resolver tests use saved fixtures.

## Code guidelines

- Rust 2021, MSRV 1.88. `unsafe` is forbidden.
- Keep domain errors in `thiserror` enums; convert to `anyhow` at the CLI boundary.
- Do not panic on user input or malformed catalog data.
- Keep TUI rendering pure: `ui::draw` and screen modules take `&App` and do no I/O.
- UI state changes belong in `src/app/state.rs`; key translation belongs in
  `src/app/events.rs`; the terminal/manager loop belongs in `src/app/mod.rs`.
- New dependencies need a clear reason and should fit the existing stack.
- Keep the terminal UI usable at 80×24, with keyboard-only access and `NO_COLOR`
  support. Avoid mouse-only actions.
- Parser functions should be testable without network access.
- Prefer focused changes over drive-by refactors.

When changing behavior, update the relevant README/docs, `CHANGELOG.md` and tests.
For UI changes, regenerate the images:

```sh
cargo run --example screenshots
```

## Tests and fixtures

- Add unit tests for pure logic (filtering, paths, years, state transitions).
- Add integration tests where HTTP behavior is involved, using wiremock.
- Add a resolver fixture under `tests/fixtures/resolvers/` and a parser test in
  `tests/resolvers_fixtures.rs`.
- Include negative cases: off-host links, postbacks, missing years, duplicates,
  malformed HTML.
- Do not add sleeps or live retries to the test suite.

## Pull requests

Use [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md). A good
PR:

- has one logical purpose;
- explains what changed and why;
- links the issue it resolves;
- includes tests/fixtures for new behavior;
- includes a screenshot for UI changes;
- states the verification status of any new source;
- passes the local quality gate.

Path labels (`tui`, `source`, `catalog`, `downloads`, `ci`, ...) are applied
automatically by the auto-label workflow. Leave them in place; they are derived
from [`.github/labeler.yml`](.github/labeler.yml).

Maintainers may ask for a fixture, a narrower change, a source status change or
additional failure handling. That is normal review, not rejection.

### Commit messages

pu-get uses its own commit-type dictionary instead of copying another project's
prefix list. The full guide is in [`docs/COMMITS.md`](docs/COMMITS.md); the short
form is:

```text
<type>(<scope>): <summary>
```

Examples:

```text
source(cbse): parse the new sample-paper index
tui(queue): shorten retry messages
docs(commits): add the performance type
```

The scope is optional. The summary stays under 72 characters and does not end
with a period. Types, descriptions and examples live in
[`.github/commit-types.json`](.github/commit-types.json); adding a type is a data
change reviewed like any other. CI validates every non-merge commit and the pull
request title, because a squash merge uses the title as the commit subject.

Validate locally:

```sh
python3 scripts/commit_lint.py --message "tui(queue): shorten retry messages"
python3 scripts/commit_lint.py --range origin/main..HEAD
```

Enable the optional commit-msg hook once:

```sh
git config core.hooksPath .githooks
```

## Issue reports

Use the issue forms so the right fields are present:

- **Bug:** version, OS/terminal, interface (TUI/CLI), reproduction steps, expected
  and actual behavior, relevant logs.
- **Source/broken link:** official page URL, board/level, material type, ownership
  or licence confirmation, and whether the link was checked.
- **Feature:** the problem, proposed behavior, alternatives and scope.

Please redact personal filesystem paths and any private information from logs.

## Security

Do not open a public issue for a security vulnerability. Follow
[SECURITY.md](SECURITY.md). This includes TLS/verification issues, path traversal,
unsafe update handling, malicious catalog behavior and supply-chain concerns.

## Community

Participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Be
respectful and assume good faith. This project exists to make official material
easier to find, not to redistribute it.

## License

By contributing, you agree that your contribution is licensed under the project's
[MIT License](LICENSE).
