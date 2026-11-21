# Releasing pu-get

Releases are tag-driven. Pushing a tag such as `v0.0.1` runs
[`release.yml`](../.github/workflows/release.yml), builds binaries, writes
checksums and publishes a GitHub release.

The workflow refuses to publish if the tag does not match `version` in
`Cargo.toml`, so the version bump and the tag must agree.

## Versioning

- Pre-release versions use the `0.0.x` series for this project.
- Tags are `v` + the Cargo version: `v0.0.1`.
- `v0.0.x` releases are marked as GitHub pre-releases by the workflow.
- SemVer applies: new behavior as `0.0.x` before a stable API commitment.

## Release checklist

### Code and data

- [ ] `version` in `Cargo.toml` is the intended release.
- [ ] `CHANGELOG.md` has an entry for the version and date.
- [ ] Generated catalogs are current, or the README snapshot counts/date are
      accurate for the shipped catalogs.
- [ ] `crawl-report.txt` has been reviewed locally (it is intentionally ignored by
      git).
- [ ] README supported-board counts, limitations and install notes are accurate.
- [ ] Contributor docs and source policy are current.
- [ ] Commit convention docs and `.github/commit-types.json` agree, and the PR
      title follows the convention.
- [ ] Archive names still match `install.sh` / `install.ps1` and the workflows.
- [ ] Docs/screenshots match the current UI (`cargo run --example screenshots`).

### Local gate

Run from the repository root with the committed lockfile:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
CARGO_TARGET_DIR=target/msrv188 cargo +1.88.0 check --locked --all-targets
cargo build --release --locked
```

Also sanity-check:

```sh
./target/release/pu-get --version
./target/release/pu-get list --board cbse --limit 3
sh -n install.sh
./install.sh --version v0.0.1 --dry-run
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/commit_lint.py --check-docs docs/COMMITS.md
```

Optionally run the
[**Binaries**](../.github/workflows/binaries.yml) workflow manually before the
tag. It builds all four archives plus `SHA256SUMS.txt` as downloadable Actions
artifacts, without publishing a release.

For a UI release, run the TUI in a real terminal at 80×24 and at a wider size, and
check `--no-color`.

Validate workflows and documentation links:

```sh
find .github -name '*.yml' -print0 |
  xargs -0 -n1 python3 -c 'import sys, yaml; yaml.safe_load(open(sys.argv[1]))'
# Optionally, with actionlint installed:
actionlint .github/workflows/*.yml
# Optionally use a Markdown link checker for README/docs.
```

### Commit and tag

```sh
git add .
git commit -m "Prepare v0.0.1"
git push origin main
git tag -a v0.0.1 -m "pu-get v0.0.1"
git push origin v0.0.1
```

Do not reuse or move a published version tag. If a release is wrong, publish a new
patch version.

## What the workflow builds

The tag-driven release workflow and the manual **Binaries** workflow use the same
archive layout. The release workflow publishes the downloads; the Binaries
workflow only uploads them as Actions artifacts.

| Runner | Target | Archive |
|---|---|---|
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `macos-latest` | `aarch64-apple-darwin` | `.tar.gz` |
| `macos-latest` | `x86_64-apple-darwin` (cross-compiled) | `.tar.gz` |
| `windows-latest` | `x86_64-pc-windows-msvc` | `.zip` |

Each archive contains:

```text
pu-get-v0.0.1-<target>/
  pu-get          # pu-get.exe on Windows
  README.md
  LICENSE
  CHANGELOG.md
```

The publish job downloads all archives, generates `SHA256SUMS.txt`, and creates the
GitHub release with generated release notes. `v0.0.x` tags are pre-releases.

Requirements:

- The repository must allow `GITHUB_TOKEN` with `contents: write` (the workflow
  requests this permission).
- No extra secrets are needed.
- macOS and Windows binaries are currently unsigned and not notarised; document
  the OS warning behavior rather than claiming otherwise.

## Verify the published release

1. Open the release page and check all four archives plus `SHA256SUMS.txt`.
2. Download and verify at least one archive:

   ```sh
   sha256sum -c SHA256SUMS.txt
   tar -tzf pu-get-v0.0.1-x86_64-unknown-linux-gnu.tar.gz
   tar -xzf pu-get-v0.0.1-x86_64-unknown-linux-gnu.tar.gz
   ./pu-get-v0.0.1-x86_64-unknown-linux-gnu/pu-get --version
   ```

3. Run a small `list` command and, if possible, launch the TUI.
4. Confirm the release is marked as a pre-release if it is `v0.0.x`.
5. Confirm the release notes and links render correctly.
6. Test the one-line installers against the published release:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/dumbramkrishna/pu-get/main/install.sh | sh -s -- --dry-run
   curl -fsSL https://raw.githubusercontent.com/dumbramkrishna/pu-get/main/install.sh | sh
   pu-get --version
   ```

   On Windows, run `install.ps1` in a fresh PowerShell session. The installers
   verify `SHA256SUMS.txt`; a checksum failure is a release blocker.

   If the release is marked as a pre-release, confirm that the installer resolved
   it through the releases list rather than
   `https://api.github.com/repos/.../releases/latest`, which only returns stable
   releases.

## Post-release

- Watch for source/link reports and triage them with the source issue form.
- Update the catalog snapshot in a later release rather than mutating release
  assets.
- Keep the MSRV and CI badges accurate.
- If a security issue affects a release, follow [SECURITY.md](../SECURITY.md).

## Hotfixes

For `v0.0.2` or later:

1. Make the minimal fix on `main`.
2. Update `Cargo.toml` and `CHANGELOG.md`.
3. Run the full gate.
4. Tag and push the new version.
5. Verify the new archives and checksums.

Do not delete a tag/release to hide a broken build unless it was published by
mistake and has not been distributed; prefer a new patch release.
