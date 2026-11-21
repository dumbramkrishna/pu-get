#!/usr/bin/env python3
"""Validate pu-get commit messages against .github/commit-types.json.

This script intentionally uses only the Python standard library, so it works in
CI and on a fresh contributor machine without installing packages.

Examples:
    python3 scripts/commit_lint.py --message "tui(browse): align the chips"
    python3 scripts/commit_lint.py --range origin/main..HEAD
    python3 scripts/commit_lint.py --types
    python3 scripts/commit_lint.py --check-docs docs/COMMITS.md
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / ".github" / "commit-types.json"
BEGIN_MARKER = "<!-- BEGIN COMMIT TYPES -->"
END_MARKER = "<!-- END COMMIT TYPES -->"

HEADER_RE = re.compile(
    r"^(?P<type>[a-z][a-z0-9-]*)"
    r"(?:\((?P<scope>[^)]*)\))?"
    r"(?P<breaking>!)?: "
    r"(?P<summary>.*)$"
)
MISSING_SPACE_RE = re.compile(r"^[a-z][a-z0-9-]*(?:\([^)]*\))?!?:")
TYPE_NAME_RE = re.compile(r"^[a-z][a-z0-9-]*$")
# GitHub appends " (#123)" to the PR title when it creates a squash commit.
SQUASH_SUFFIX_RE = re.compile(r" \(#\d+\)$")


class ConfigError(Exception):
    """The commit-type dictionary is missing or malformed."""


class GitError(Exception):
    """git could not produce commit messages for the requested range."""


def load_config(path: Path) -> dict:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ConfigError("commit-type dictionary not found: %s" % path)
    except (OSError, json.JSONDecodeError) as exc:
        raise ConfigError("cannot read %s: %s" % (path, exc))

    if not isinstance(data, dict):
        raise ConfigError("%s must contain a JSON object" % path)

    for key in ("header_max", "summary_max", "scope_pattern", "types"):
        if key not in data:
            raise ConfigError("%s is missing `%s`" % (path, key))

    if not isinstance(data["header_max"], int) or data["header_max"] < 1:
        raise ConfigError("header_max must be a positive integer")
    if not isinstance(data["summary_max"], int) or data["summary_max"] < 1:
        raise ConfigError("summary_max must be a positive integer")
    if not isinstance(data["scope_pattern"], str):
        raise ConfigError("scope_pattern must be a string")
    try:
        re.compile(data["scope_pattern"])
    except re.error as exc:
        raise ConfigError("scope_pattern is not a valid regex: %s" % exc)

    types = data["types"]
    if not isinstance(types, dict) or not types:
        raise ConfigError("types must be a non-empty object")
    for name, meta in types.items():
        if not TYPE_NAME_RE.fullmatch(name):
            raise ConfigError(
                "type `%s` must be lowercase words separated by hyphens" % name
            )
        if not isinstance(meta, dict):
            raise ConfigError("type `%s` must be an object" % name)
        for field in ("description", "example"):
            if not isinstance(meta.get(field), str) or not meta[field].strip():
                raise ConfigError("type `%s` needs a non-empty `%s`" % (name, field))

    patterns = data.get("ignore_patterns", [])
    if not isinstance(patterns, list) or not all(isinstance(p, str) for p in patterns):
        raise ConfigError("ignore_patterns must be an array of strings")
    for pattern in patterns:
        try:
            re.compile(pattern)
        except re.error as exc:
            raise ConfigError("ignore pattern `%s` is not valid: %s" % (pattern, exc))

    if not isinstance(data.get("forbid_trailing_period", True), bool):
        raise ConfigError("forbid_trailing_period must be a boolean")
    return data


def first_line(message: str) -> str:
    for line in message.replace("\r\n", "\n").splitlines():
        if line.strip():
            return line.rstrip()
    return ""


def is_ignored(message: str, config: dict) -> bool:
    header = first_line(message)
    for pattern in config.get("ignore_patterns", []):
        if re.search(pattern, header):
            return True
    return False


def validate_message(message: str, config: dict) -> list[str]:
    """Return a list of human-readable problems for one message."""
    if not message.strip():
        return ["message is empty"]
    if is_ignored(message, config):
        return []

    header = first_line(message)
    issues = []

    match = HEADER_RE.match(header)
    if not match:
        if MISSING_SPACE_RE.match(header):
            issues.append(
                "put a single space after the colon, for example "
                "`fix(queue): ...`"
            )
        else:
            issues.append(
                "expected `type(scope): summary` or `type: summary` "
                "(scope is optional)"
            )
        return issues

    type_name = match.group("type")
    if type_name not in config["types"]:
        suggestion = difflib.get_close_matches(
            type_name, list(config["types"]), n=1, cutoff=0.55
        )
        hint = " (did you mean `%s`?)" % suggestion[0] if suggestion else ""
        issues.append("unknown type `%s`%s" % (type_name, hint))
        issues.append("known types: " + ", ".join(config["types"]))

    scope = match.group("scope")
    if scope is not None:
        if not scope:
            issues.append(
                "scope cannot be empty; use `type: summary` or "
                "`type(scope): summary`"
            )
        elif not re.fullmatch(config["scope_pattern"], scope):
            issues.append(
                "scope `%s` must match %s" % (scope, config["scope_pattern"])
            )

    summary = match.group("summary").strip()
    summary_for_rules = SQUASH_SUFFIX_RE.sub("", summary)
    if not summary_for_rules:
        issues.append("summary is empty")
    else:
        if len(summary_for_rules) > config["summary_max"]:
            issues.append(
                "summary is %d characters; keep it to %d or fewer"
                % (len(summary_for_rules), config["summary_max"])
            )
        if config.get("forbid_trailing_period", True) and summary_for_rules.endswith(
            "."
        ):
            issues.append("do not end the summary with a period")

    if len(header) > config["header_max"]:
        issues.append(
            "header is %d characters; keep it to %d or fewer"
            % (len(header), config["header_max"])
        )

    return issues


def render_types_table(config: dict) -> str:
    def escape(value: str) -> str:
        return value.replace("|", "\\|")

    lines = ["| Type | Use for | Example |", "|---|---|---|"]
    for name, meta in config["types"].items():
        lines.append(
            "| `%s` | %s | `%s` |"
            % (name, escape(meta["description"]), escape(meta["example"]))
        )
    return "\n".join(lines)


def check_docs(path: Path, config: dict) -> Optional[str]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        return "cannot read %s: %s" % (path, exc)

    if BEGIN_MARKER not in text or END_MARKER not in text:
        return "%s is missing the %s / %s markers" % (
            path,
            BEGIN_MARKER,
            END_MARKER,
        )

    start = text.index(BEGIN_MARKER) + len(BEGIN_MARKER)
    end = text.index(END_MARKER, start)
    actual = text[start:end].strip()
    expected = render_types_table(config)
    if actual == expected:
        return None

    diff = difflib.unified_diff(
        expected.splitlines(),
        actual.splitlines(),
        fromfile=".github/commit-types.json (generated table)",
        tofile=str(path),
        lineterm="",
    )
    return "\n".join(diff)


def read_git_range(revision_range: str) -> list[tuple[str, str]]:
    command = [
        "git",
        "-C",
        str(REPO_ROOT),
        "log",
        "--no-merges",
        "-z",
        "--format=%H%n%B",
        revision_range,
    ]
    try:
        result = subprocess.run(command, check=True, capture_output=True)
    except FileNotFoundError:
        raise GitError("git is not installed or not on PATH")
    except subprocess.CalledProcessError as exc:
        detail = exc.stderr.decode("utf-8", "replace").strip()
        raise GitError(
            "git log failed for %s%s"
            % (revision_range, ": " + detail if detail else "")
        )

    messages = []
    for record in result.stdout.decode("utf-8", "replace").split("\x00"):
        record = record.strip("\n")
        if not record:
            continue
        sha, separator, body = record.partition("\n")
        if not separator:
            body = ""
        messages.append((sha.strip()[:12], body))
    return messages


def report(label: str, message: str, issues: list[str]) -> None:
    header = first_line(message) or "(empty message)"
    print("%s: %s" % (label, header), file=sys.stderr)
    for issue in issues:
        print("  - %s" % issue, file=sys.stderr)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate pu-get commit messages against the project dictionary."
    )
    parser.add_argument(
        "--config",
        metavar="PATH",
        help="commit-type dictionary (default: .github/commit-types.json)",
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--message", metavar="TEXT", help="validate one message or PR title"
    )
    mode.add_argument(
        "--message-file",
        metavar="PATH",
        help="validate a git commit message file (for the optional commit-msg hook)",
    )
    mode.add_argument(
        "--range",
        metavar="REV",
        help="validate non-merge commits in a git revision range, "
        "for example origin/main..HEAD",
    )
    mode.add_argument(
        "--types",
        action="store_true",
        help="print the Markdown table generated from the dictionary",
    )
    mode.add_argument(
        "--check-docs",
        metavar="PATH",
        help="check that a Markdown file contains the generated type table",
    )
    return parser


def main(argv: Optional[list[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    config_path = Path(args.config).expanduser() if args.config else DEFAULT_CONFIG

    try:
        config = load_config(config_path)
    except ConfigError as exc:
        print("error: %s" % exc, file=sys.stderr)
        return 2

    if args.types:
        print(render_types_table(config))
        return 0

    if args.check_docs:
        path = Path(args.check_docs)
        problem = check_docs(path, config)
        if problem:
            print("error: %s" % problem, file=sys.stderr)
            print(
                "Regenerate the table with: python3 scripts/commit_lint.py --types",
                file=sys.stderr,
            )
            return 1
        print("%s: commit-type table is up to date" % path)
        return 0

    messages: list[tuple[str, str]] = []
    if args.message is not None:
        messages.append(("message", args.message))
    elif args.message_file is not None:
        try:
            messages.append(
                (
                    str(args.message_file),
                    Path(args.message_file).read_text(encoding="utf-8"),
                )
            )
        except OSError as exc:
            print(
                "error: cannot read %s: %s" % (args.message_file, exc),
                file=sys.stderr,
            )
            return 2
    else:
        try:
            messages = read_git_range(args.range)
        except GitError as exc:
            print("error: %s" % exc, file=sys.stderr)
            return 2
        if not messages:
            print("No non-merge commits to check.")
            return 0

    failures = 0
    for label, message in messages:
        issues = validate_message(message, config)
        if issues:
            failures += 1
            report(label, message, issues)

    if failures:
        print(
            "\n%d of %d message(s) do not follow the pu-get convention."
            % (failures, len(messages)),
            file=sys.stderr,
        )
        print("See docs/COMMITS.md and .github/commit-types.json.", file=sys.stderr)
        return 1

    print("%d message(s) follow the pu-get commit convention." % len(messages))
    return 0


if __name__ == "__main__":
    sys.exit(main())
