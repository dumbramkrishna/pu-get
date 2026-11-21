# pu-get documentation

The [project README](../README.md) covers installation and day-to-day use. These
guides are for people working on the code, catalogs or release process.

Start with **[Architecture](ARCHITECTURE.md)** if you want the mental model, then
read **[Adding sources](ADDING_SOURCES.md)** if you want to add or fix a board.

## Contributor guides

| Guide | What it covers |
|---|---|
| [Architecture](ARCHITECTURE.md) | Modules, data flow, invariants and the download lifecycle |
| [Adding sources](ADDING_SOURCES.md) | Registry rules, `generic_index`, custom resolvers, fixtures, crawl and verification |
| [Commit messages](COMMITS.md) | Type dictionary, scopes, examples and how to add a type |
| [Development](DEVELOPMENT.md) | Toolchain, commands, testing, debugging, screenshots and local overrides |
| [Releasing](RELEASING.md) | Versioning, the release checklist, tags, artifacts and verification |
| [Screenshots](screenshots/README.md) | How the README images are generated |

## Reference

- [Catalog and update schema](../sources/schema.md)
- [Source registry](../sources/registry.toml)
- [Changelog](../CHANGELOG.md)
- [Contributing](../CONTRIBUTING.md)
- [Security policy](../SECURITY.md)
- [Code of Conduct](../CODE_OF_CONDUCT.md)

## User-facing topics

- [Install and quick start](../README.md#install)
- [Keyboard shortcuts](../README.md#keyboard-shortcuts)
- [CLI reference](../README.md#command-line-usage)
- [Configuration](../README.md#configuration)
- [Catalog behavior and updates](../README.md#catalogs-and-updates)

## Ground rules for this project

- Only official boards/government bodies or openly licensed publishers belong in
  the registry.
- PDF URLs are discovered by parsers, never written by hand.
- Parser tests use saved HTML fixtures, not the live network.
- A source is `unverified` until its index and files have been checked.
- Never disable TLS verification or bypass access controls to make a source work.
- Keep the TUI pure: rendering reads state, input handling changes it.
