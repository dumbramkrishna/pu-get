"""Tests for scripts/commit_lint.py (standard library only)."""

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = REPO_ROOT / "scripts" / "commit_lint.py"

spec = importlib.util.spec_from_file_location("commit_lint", SCRIPT_PATH)
commit_lint = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(commit_lint)


class CommitMessageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config = commit_lint.load_config(commit_lint.DEFAULT_CONFIG)

    def assert_valid(self, message):
        self.assertEqual(
            commit_lint.validate_message(message, self.config),
            [],
            "expected a valid message: %r" % message,
        )

    def assert_invalid(self, message, fragment):
        issues = commit_lint.validate_message(message, self.config)
        self.assertTrue(
            any(fragment in issue for issue in issues),
            "expected %r in %r for %r" % (fragment, issues, message),
        )

    def test_every_dictionary_type_is_accepted(self):
        for name in self.config["types"]:
            self.assert_valid("%s: keep the change focused" % name)

    def test_scope_is_optional(self):
        self.assert_valid("fix: keep the queue stable")
        self.assert_valid("fix(queue): keep the retry ETA stable")

    def test_breaking_marker_is_accepted(self):
        self.assert_valid("cli!: remove the legacy output format")
        self.assert_valid("build(config)!: rename the request timeout key")

    def test_body_is_ignored(self):
        self.assert_valid("source(ncert): parse the new index\n\nLonger body.\n")

    def test_unknown_type_gets_a_hint(self):
        issues = commit_lint.validate_message("feature: add a thing", self.config)
        self.assertTrue(any("unknown type" in issue for issue in issues))
        self.assertTrue(any("known types: add" in issue for issue in issues))

    def test_empty_scope_is_rejected(self):
        self.assert_invalid("fix(): keep the queue stable", "scope cannot be empty")

    def test_missing_space_is_rejected(self):
        self.assert_invalid("fix:keep the queue stable", "single space")

    def test_missing_format_is_rejected(self):
        self.assert_invalid("updated the docs", "expected `type(scope): summary`")

    def test_uppercase_scope_is_rejected(self):
        self.assert_invalid("fix(Browse): align the chips", "scope `Browse`")

    def test_trailing_period_is_rejected(self):
        self.assert_invalid("docs: update the README.", "period")

    def test_squash_merge_number_is_accepted(self):
        self.assert_valid("fix(queue): keep retry ETA stable (#123)")
        self.assert_valid("fix: %s (#123)" % ("x" * 72))

    def test_long_summary_is_rejected(self):
        self.assert_invalid("fix: %s" % ("x" * 80), "keep it to")

    def test_empty_message_is_rejected(self):
        self.assert_invalid("   \n\n", "message is empty")

    def test_known_generated_messages_are_ignored(self):
        for message in (
            "Merge branch 'main' into feature",
            'Revert "fix: keep the queue stable"',
            "fixup! fix: keep the queue stable",
            "squash! fix: keep the queue stable",
            "Bump serde from 1.0.0 to 1.0.1",
        ):
            self.assert_valid(message)

    def test_types_table_mentions_every_type(self):
        table = commit_lint.render_types_table(self.config)
        for name in self.config["types"]:
            self.assertIn("`%s`" % name, table)

    def test_docs_table_is_generated_from_dictionary(self):
        problem = commit_lint.check_docs(
            REPO_ROOT / "docs" / "COMMITS.md", self.config
        )
        self.assertIsNone(problem, problem)

    def test_malformed_config_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "commit-types.json"
            path.write_text(json.dumps({"types": {}}), encoding="utf-8")
            with self.assertRaises(commit_lint.ConfigError):
                commit_lint.load_config(path)

    def test_read_git_range_parses_non_merge_commits(self):
        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp)
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            file_path = repo / "notes.txt"

            def commit(message):
                subprocess.run(
                    ["git", "-C", str(repo), "add", "notes.txt"], check=True
                )
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(repo),
                        "-c",
                        "user.name=pu-get tests",
                        "-c",
                        "user.email=tests@example.invalid",
                        "commit",
                        "-q",
                        "-m",
                        message,
                    ],
                    check=True,
                )

            file_path.write_text("one\n", encoding="utf-8")
            commit("add: first fixture")
            file_path.write_text("two\n", encoding="utf-8")
            commit("fix: second commit")

            with mock.patch.object(commit_lint, "REPO_ROOT", repo):
                messages = commit_lint.read_git_range("HEAD~1..HEAD")

            self.assertEqual(len(messages), 1)
            self.assertIn("fix: second commit", messages[0][1])
            self.assertEqual(len(messages[0][0]), 12)


if __name__ == "__main__":
    unittest.main()
