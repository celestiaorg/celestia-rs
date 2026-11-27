#!/usr/bin/env python3
"""Tests for release_version.py: PR titles, version bumps, release notes."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import release_version as rv  # noqa: E402


def c(subject, body=""):
    return rv.Commit(subject, body)


class TitleTests(unittest.TestCase):
    def test_conventional_titles_pass(self):
        for title in ("feat: add x", "fix(rpc): handle y", "feat(cli)!: drop --z",
                      "ci: pin action", "build(deny): bump", "revert: feat: x",
                      "docs: fix typo (#12)"):
            with self.subTest(title=title):
                self.assertIsNone(rv.check_title(title))

    def test_other_titles_fail(self):
        for title in ("Add x", "feature: x", "fix:x", "fix: ", "FIX: x", "feat(): x", "wip"):
            with self.subTest(title=title):
                self.assertIsNotNone(rv.check_title(title))


class BumpTests(unittest.TestCase):
    def test_first_release_is_the_crate_version_line(self):
        self.assertEqual(rv.next_version("0.1.0-rc.9", None, [c("feat: x")]), "0.1.0")
        self.assertEqual(rv.next_version("0.2.0", None, [c("fix: x")]), "0.2.0")

    def test_before_1_0_breaking_bumps_minor_else_patch(self):
        self.assertEqual(rv.next_version("0.1.0", "0.1.4", [c("fix: a"), c("feat: b")]), "0.1.5")
        self.assertEqual(rv.next_version("0.1.0", "0.1.4", [c("feat(cli)!: drop z")]), "0.2.0")
        self.assertEqual(rv.next_version("0.1.0", "0.1.4",
                                         [c("fix: a", "BREAKING CHANGE: config key renamed")]), "0.2.0")

    def test_automatic_releases_stay_below_1_0(self):
        self.assertEqual(rv.next_version("0.1.0", "0.99.4", [c("feat!: x")]), "0.100.0")

    def test_from_1_0_breaking_major_feat_minor_else_patch(self):
        self.assertEqual(rv.next_version("1.0.0", "1.2.3", [c("fix: a"), c("chore: b")]), "1.2.4")
        self.assertEqual(rv.next_version("1.0.0", "1.2.3", [c("fix: a"), c("feat: b")]), "1.3.0")
        self.assertEqual(rv.next_version("1.0.0", "1.2.3", [c("feat!: b")]), "2.0.0")

    def test_non_conventional_commits_count_as_patches(self):
        self.assertEqual(rv.next_version("1.0.0", "1.2.3", [c("Merge branch x")]), "1.2.4")

    def test_a_crate_version_ahead_of_the_last_tag_wins(self):
        # A human bumped the crate to 0.3.0 after v0.2.7.
        self.assertEqual(rv.next_version("0.3.0", "0.2.7", [c("fix: a")]), "0.3.0")
        self.assertEqual(rv.next_version("0.3.0", "0.3.0", [c("fix: a")]), "0.3.1")


class NotesTests(unittest.TestCase):
    def test_notes_group_by_type(self):
        notes = rv.release_notes([
            c("feat(cli)!: drop --z (#5)"),
            c("feat: add x (#1)"),
            c("fix(rpc): handle y (#2)"),
            c("fix: z", "BREAKING CHANGE: renamed"),
            c("ci: pin (#3)"),
            c("Merge stuff"),
        ])
        self.assertEqual(notes, "\n".join([
            "### Breaking changes",
            "",
            "- **cli:** drop --z (#5)",
            "- z",
            "",
            "### Features",
            "",
            "- add x (#1)",
            "",
            "### Fixes",
            "",
            "- **rpc:** handle y (#2)",
            "",
            "### Other changes",
            "",
            "- ci: pin (#3)",
            "- Merge stuff",
            "",
        ]))

    def test_no_commits(self):
        self.assertEqual(rv.release_notes([]), "No changes.\n")


class CliTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, str(Path(__file__).with_name("release_version.py")), *args],
                              capture_output=True, text=True)

    def test_next_and_notes_read_a_git_log(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "log"
            # git log --format=%s%x1f%b%x1e
            log.write_text("feat: a (#1)\x1f\x1e\nfix: b\x1fBREAKING CHANGE: x\n\x1e\n")
            r = self.run_cli("next", "--crate-version", "0.1.0", "--last", "0.1.3", "--log", str(log))
            self.assertEqual((r.returncode, r.stdout), (0, "0.2.0\n"), r.stderr)
            r = self.run_cli("next", "--crate-version", "0.1.0-rc.9", "--last", "", "--log", str(log))
            self.assertEqual(r.stdout, "0.1.0\n")
            r = self.run_cli("notes", "--log", str(log))
            self.assertIn("### Breaking changes\n\n- b\n", r.stdout)

    def test_check_title(self):
        self.assertEqual(self.run_cli("check-title", "feat: x").returncode, 0)
        r = self.run_cli("check-title", "Add x")
        self.assertEqual(r.returncode, 1)
        self.assertIn("conventional commit", r.stderr)


if __name__ == "__main__":
    unittest.main()
