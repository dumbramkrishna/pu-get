# Commit messages

pu-get uses its own small commit convention. The goal is a history that is easy
to scan, filter and turn into release notes — not to copy another project's
prefix list.

The convention is enforced by `scripts/commit_lint.py`. The dictionary lives in
[`.github/commit-types.json`](../.github/commit-types.json), and contributors can
add a new type with a pull request.

## Format

```text
<type>(<scope>): <summary>
<type>: <summary>
<type>!: <summary>
```

The scope is optional. Use `!` before the colon when the change is breaking:

```text
cli!: remove the legacy --format flag

BREAKING CHANGE: the JSON output now uses `catalog_version`.
```

### Rules

| Rule | Detail |
|---|---|
| Type | Must be one of the types in the dictionary; lowercase |
| Scope | Optional; lowercase letters, digits, `.`, `_` or `-` |
| Summary | Imperative, no trailing period, 72 characters or fewer |
| Header | 100 characters or fewer including type and scope |
| Body | Optional; separate it from the header with a blank line |
| Breaking change | Add `!` and explain it in the body |

The first line is what matters for review, `git log --oneline` and squash merges.
Keep it specific: “fix queue” is less useful than “fix(queue): keep retry ETA
stable after reconnect”.

GitHub appends ` (#123)` to the subject when it creates a squash commit. The
linter ignores that suffix when checking the summary length, so the PR title and
the resulting `main` commit stay consistent.

Pull-request titles follow the same format. A squash merge uses the PR title as
the commit subject, so CI validates both the title and the commits.

## Types

<!-- BEGIN COMMIT TYPES -->
| Type | Use for | Example |
|---|---|---|
| `add` | A new user-visible capability or supported resource | `add(ncert): include Class 11 Biology textbooks` |
| `fix` | A bug fix | `fix(queue): keep retry ETA stable after reconnect` |
| `tui` | Screens, widgets, layout or keyboard interaction | `tui(browse): align the year and latest chips` |
| `cli` | Non-interactive commands, flags or output | `cli(list): add a --stream filter` |
| `source` | Registry, resolver, crawl or source-policy changes | `source(pue): parse the commerce textbook table` |
| `catalog` | Catalog data, filtering, search or generated catalogs | `catalog(filter): prefer the configured language` |
| `download` | Download engine, queue, progress, resume or validation | `download(queue): retry 502 responses with backoff` |
| `docs` | README, guides, schema or comments | `docs: explain the official-source policy` |
| `test` | Tests, fixtures or test tooling | `test(resolvers): add a KSEAB fixture` |
| `build` | Build, dependencies, CI, packaging or installers | `build(ci): add commit-message validation` |
| `chore` | Maintenance with no behaviour change | `chore: refresh the lockfile` |
| `refactor` | Internal restructuring without a behaviour change | `refactor(catalog): split the loader from the filter` |
| `security` | Security fixes or hardening | `security(download): constrain redirect hosts` |
| `release` | Version bumps, changelog and release preparation | `release: prepare v0.0.1` |
| `revert` | Revert an earlier commit | `revert: revert the experimental queue layout` |
<!-- END COMMIT TYPES -->

The table is generated from the dictionary. Do not edit it by hand; see
[Adding a type](#adding-a-type).

## Scopes

Scopes are optional and are not restricted to a fixed list. Prefer a module,
screen, board or command that a reader can recognise. The dictionary includes
these examples:

```text
browse  queue  settings  splash  help
ncert   cbse   kseeb     pue
catalog download config  installer  labels  ci
```

Use a scope when it adds information. `fix: keep retry ETA stable` is fine;
`fix(queue): keep retry ETA stable` is better when the change is specific to the
queue.

## Examples

Good:

```text
add(cbse): include marking schemes for Class 12
fix(download): resume after a truncated response
tui(help): show the settings key in the footer
source(ncert): parse the new textbook index
build(ci): check commit messages on pull requests
docs: document the optional git hook
```

Not accepted:

```text
feature: add a board          # unknown type; use `add`
fix(): tidy the footer        # empty scope; omit the parentheses
fix:tidy the footer           # missing space after the colon
TUI: move the chips           # types and scopes are lowercase
docs: update the README.      # trailing period
```

## Adding a type

The dictionary is data, not code. A new area can be added without changing the
linter.

1. Add an entry to
   [`.github/commit-types.json`](../.github/commit-types.json):

   ```json
   "performance": {
     "description": "Speed or memory improvements",
     "example": "performance(download): buffer writes in 64 KiB chunks"
   }
   ```

2. Regenerate the table in this document:

   ```sh
   python3 scripts/commit_lint.py --types
   ```

   Replace the block between the `BEGIN COMMIT TYPES` and `END COMMIT TYPES`
   markers with the output.

3. Check that this page and the dictionary agree:

   ```sh
   python3 scripts/commit_lint.py --check-docs docs/COMMITS.md
   ```

4. Run the linter tests:

   ```sh
   python3 -m unittest discover -s scripts -p 'test_*.py'
   ```

5. Open a PR. Use a message such as:

   ```text
   docs(commits): add the performance type
   ```

Guidelines for new types:

- A type should describe a durable area of the project, not a one-off task.
- Avoid near-duplicates (`ui` next to `tui`, `bugfix` next to `fix`).
- Keep the description short and give one realistic example.
- Adding a type is backward compatible. Renaming or removing one can break
  existing tooling and saved links, so discuss it in an issue first and document
  the migration in `CHANGELOG.md`.

## Local checks

Validate a proposed subject or PR title:

```sh
python3 scripts/commit_lint.py --message "tui(queue): shorten retry messages"
```

Validate every non-merge commit on a branch:

```sh
python3 scripts/commit_lint.py --range origin/main..HEAD
```

Print the generated type table:

```sh
python3 scripts/commit_lint.py --types
```

### Optional git hook

The repository includes a commit-msg hook. It is not enabled automatically:

```sh
git config core.hooksPath .githooks
```

After that, `git commit` validates the message before the commit is created. Use
`git commit --no-verify` only when you have a specific reason to bypass it.

## What CI checks

The [commit-lint workflow](../.github/workflows/commit-lint.yml) runs on pull
requests and pushes to `main`. It:

- runs the linter's unit tests;
- validates the pull-request title;
- validates every non-merge commit in the pull request or push range;
- verifies that this document's type table matches the dictionary.

Generated messages are ignored so normal maintenance does not fail:

- merge commits (`Merge ...`);
- reverts created by GitHub (`Revert "..."`);
- `fixup!`, `squash!` and `amend!` commits;
- Dependabot-style `Bump ... from ... to ...` commits;
- `Initial commit`.

Ignored messages still appear in history, so use them only when the tooling
created them.

## Dictionary settings

| Key | Purpose |
|---|---|
| `types` | Commit types, their descriptions and examples |
| `header_max` | Maximum length of the whole first line |
| `summary_max` | Maximum length of the summary after the colon |
| `scope_pattern` | Regex that scopes must match |
| `forbid_trailing_period` | Reject summaries ending in `.` |
| `ignore_patterns` | Generated messages that skip validation |
| `scope_examples` | Documentation-only list of useful scopes |

Keep the linter standard-library-only. It runs on Python 3.9 or newer.
