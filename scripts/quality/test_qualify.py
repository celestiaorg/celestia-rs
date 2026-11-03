"""Qualification must preserve failed checks and reject changes during a run."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("qualify", Path(__file__).with_name("qualify.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class QualificationTest(unittest.TestCase):
    def repo(self, folder):
        root = Path(folder) / "repo"
        root.mkdir()
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        (root / "source.txt").write_text("original\n")
        subprocess.run(["git", "-C", str(root), "add", "."], check=True)
        subprocess.run(["git", "-C", str(root), "-c", "user.name=Test",
                        "-c", "user.email=test@example.invalid", "commit", "-qm", "source"],
                       check=True)
        return root

    def run_check(self, root, out, code, binaries=()):
        result = MODULE.qualify(root, out, [sys.executable, "-c", code], binaries)
        return result, json.loads((out / "evidence.json").read_text())

    def test_pass_and_failure_remain_separate(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            out = Path(folder) / "success"
            result, evidence = self.run_check(root, out, "print('checked')")
            self.assertTrue(result)
            self.assertEqual(evidence["source"], evidence["source_after"])
            self.assertEqual(evidence["log_sha256"], MODULE.digest(out / "check.log"))
            with self.assertRaises(FileExistsError):
                self.run_check(root, out, "pass")
            result, evidence = self.run_check(root, Path(folder) / "failure", "raise SystemExit(3)")
            self.assertFalse(result)
            self.assertEqual(evidence["exit_code"], 3)

    def test_mutating_source_or_binary_invalidates_successful_command(self):
        for change in ("source", "binary"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                root = self.repo(folder)
                binary = Path(folder) / "binary"
                binary.write_bytes(b"before")
                path = root / "source.txt" if change == "source" else binary
                result, evidence = self.run_check(
                    root, Path(folder) / "out",
                    f"from pathlib import Path; Path({str(path)!r}).write_text('changed')",
                    [binary],
                )
                self.assertFalse(result)
                self.assertEqual(evidence["exit_code"], 0)
                self.assertEqual(evidence["status"], "failed")

    def test_untracked_source_prevents_qualification(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            (root / "untracked.rs").write_text("changed")
            with self.assertRaisesRegex(ValueError, "clean worktree"):
                self.run_check(root, Path(folder) / "out", "pass")

    def test_git_config_cannot_hide_untracked_source(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            subprocess.run(["git", "-C", str(root), "config",
                            "status.showUntrackedFiles", "no"], check=True)
            (root / "untracked.rs").write_text("changed")
            with self.assertRaisesRegex(ValueError, "clean worktree"):
                self.run_check(root, Path(folder) / "out", "pass")

    def test_git_config_cannot_hide_source_created_during_check(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            subprocess.run(["git", "-C", str(root), "config",
                            "status.showUntrackedFiles", "no"], check=True)
            result, evidence = self.run_check(
                root, Path(folder) / "out",
                "from pathlib import Path; Path('untracked.rs').write_text('changed')",
            )
            self.assertFalse(result)
            self.assertEqual(evidence["exit_code"], 0)
            self.assertEqual(evidence["status"], "failed")

    def test_index_flags_cannot_hide_tracked_changes_before_check(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as folder:
                root = self.repo(folder)
                # NUL parsing must retain unusual paths without splitting records.
                name = "source with space\nand newline.txt"
                subprocess.run(["git", "-C", str(root), "mv", "source.txt", name], check=True)
                subprocess.run(["git", "-C", str(root), "-c", "user.name=Test",
                                "-c", "user.email=test@example.invalid", "commit", "-qm", "rename"], check=True)
                subprocess.run(["git", "-C", str(root), "update-index", flag, name], check=True)
                (root / name).write_text("hidden change\n")
                self.assertEqual(MODULE.source(root)["status"], "")
                flags = MODULE.source(root)["index_flags"]
                self.assertEqual(len(flags), 1)
                self.assertEqual(flags[0][2:], name)
                out = Path(folder) / "out"
                with self.assertRaisesRegex(ValueError, "index flags"):
                    self.run_check(root, out, "raise RuntimeError('must not run')")
                self.assertFalse(out.exists())
                self.assertEqual(MODULE.source(root)["index_flags"], flags)
                self.assertEqual((root / name).read_text(), "hidden change\n")

    def test_index_flags_added_during_check_invalidate_success(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as folder:
                root = self.repo(folder)
                code = ("import subprocess; from pathlib import Path; "
                        f"subprocess.run(['git', 'update-index', {flag!r}, 'source.txt'], check=True); "
                        "Path('source.txt').write_text('hidden change')")
                result, evidence = self.run_check(root, Path(folder) / "out", code)
                self.assertFalse(result)
                self.assertEqual(evidence["exit_code"], 0)
                self.assertEqual(evidence["source_after"]["status"], "")
                self.assertTrue(evidence["source_after"]["index_flags"])

    def test_fsmonitor_cache_cannot_hide_tracked_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            # A valid v2 hook reporting no changes leaves fsmonitor-valid bits
            # set. Qualification must inspect files instead of trusting it.
            hook = Path(folder) / "fsmonitor"
            hook.write_text('#!/bin/sh\nprintf "token\\0"\n')
            hook.chmod(0o755)
            for key, value in (("core.fsmonitor", str(hook)),
                               ("core.fsmonitorHookVersion", "2")):
                subprocess.run(["git", "-C", str(root), "config", key, value], check=True)
            MODULE.git(root, "status", "--porcelain")
            subprocess.run(["git", "-C", str(root), "update-index",
                            "--fsmonitor-valid", "source.txt"], check=True)
            (root / "source.txt").write_text("hidden fsmonitor change\n")
            self.assertEqual(MODULE.git(root, "status", "--porcelain"), "")
            with self.assertRaisesRegex(ValueError, "clean worktree"):
                self.run_check(root, Path(folder) / "out", "pass")

    def test_git_config_cannot_hide_modified_submodule(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.repo(folder)
            # Make a local gitlink without fetching any external repository.
            nested = self.repo(root)
            revision = MODULE.git(nested, "rev-parse", "HEAD")
            subprocess.run(["git", "-C", str(root), "update-index", "--add",
                            "--cacheinfo", f"160000,{revision},repo"], check=True)
            subprocess.run(["git", "-C", str(root), "-c", "user.name=Test",
                            "-c", "user.email=test@example.invalid", "commit",
                            "-qm", "gitlink"], check=True)
            subprocess.run(["git", "-C", str(root), "config",
                            "diff.ignoreSubmodules", "all"], check=True)
            self.assertEqual(MODULE.source(root)["status"], "")
            (nested / "source.txt").write_text("changed")
            with self.assertRaisesRegex(ValueError, "clean worktree"):
                self.run_check(root, Path(folder) / "out", "pass")


if __name__ == "__main__":
    unittest.main()
