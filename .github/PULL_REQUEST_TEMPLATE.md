## Summary

<!-- What changed, and why? Keep it focused. -->

## Related issue

<!-- e.g. Closes #123. Use "N/A" if there is no issue. -->

Closes #

## Type of change

- [ ] Bug fix
- [ ] New source, board or resolver
- [ ] Catalog/crawl/verification change
- [ ] TUI/UX change
- [ ] Documentation
- [ ] Refactor or internal change
- [ ] Build/CI/release change

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked --all-targets`
- [ ] `cargo build --release --locked`
- [ ] MSRV check: `cargo +1.88.0 check --locked --all-targets` (if relevant)

## Sources and resolvers

<!-- Skip if this PR does not add or change a source. -->

- Source id(s):
- Board id(s):
- Registry status after this PR (`unverified` / `verified`):
- Official page(s) checked:
- Fixture added/updated:
- Targeted crawl command used:
- Confirmation:
  - [ ] The source is official or openly licensed.
  - [ ] No PDF URL was written by hand.
  - [ ] Tests use saved HTML, not the live portal.
  - [ ] The generated catalog was regenerated, if applicable.
  - [ ] Limitations (postbacks, missing years, TLS) are documented.

## TUI changes

<!-- Skip if there is no UI change. -->

- Screens affected:
- Checked at 80×24:
- Checked with `--no-color`:
- Keyboard-only navigation works:
- Screenshots regenerated with `cargo run --example screenshots`:

<!-- Attach or link a screenshot for visual changes. -->

## Breaking changes

<!-- Does this change config, catalog schema, CLI flags, folder templates or the update format? -->

- [ ] No breaking change
- [ ] Yes — described below:

## Checklist

- [ ] The change is limited to one logical purpose.
- [ ] The PR title follows `type(scope): summary` and the
      [commit guide](../docs/COMMITS.md).
- [ ] Commit messages follow the project dictionary
      ([`.github/commit-types.json`](../.github/commit-types.json)).
- [ ] Documentation/README/schema/CHANGELOG updated where behavior changed.
- [ ] New functionality has tests or fixtures.
- [ ] I have not added rehosted, paid or unlicensed material.
- [ ] I have read [CONTRIBUTING.md](../CONTRIBUTING.md) and follow the project's
      [Code of Conduct](../CODE_OF_CONDUCT.md).
- [ ] I agree that this contribution is licensed under the MIT License.
