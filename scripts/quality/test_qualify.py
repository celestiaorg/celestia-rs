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


if __name__ == "__main__":
    unittest.main()
