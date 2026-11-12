"""Cross-platform traces must belong to the same qualified source."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import compare_determinism as compare


class DeterminismComparisonTest(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.roots = [Path(folder.name)/name for name in ("linux", "macos")]
        for root in self.roots:
            root.mkdir()
            for case in compare.CASES:
                (root/(case+"-default.trace")).write_text("h=1 "+"a"*64+"\n")
            for profile in ("release", "dist"):
                directory = root/"qualification"/profile
                directory.mkdir(parents=True)
                (directory/"check.log").write_bytes(b"passed\n")
                source = {"revision": "a"*40, "tree": "b"*40, "status": "", "index_flags": []}
                self.write(root, profile, {"status": "passed", "exit_code": 0, "inputs_unchanged": True,
                                          "source": source, "source_after": source,
                                          "machine": "x86_64" if root.name == "linux" else "arm64",
                                          "platform": root.name,
                                          "binary_sha256": {str(i): "c"*64 for i in range(3)},
                                          "log_sha256": hashlib.sha256(b"passed\n").hexdigest()})

    def write(self, root, profile, row):
        (root/"qualification"/profile/"evidence.json").write_text(json.dumps(row))

    def mutate(self, **updates):
        root = self.roots[1]
        row = json.loads((root/"qualification/release/evidence.json").read_text())
        row.update(updates)
        self.write(root, "release", row)

    def test_matching_traces_and_clean_exact_source_pass(self):
        report = compare.compare(self.roots)
        self.assertTrue(report["identical"])
        self.assertEqual(len(report["traces"]), 6)
        self.assertEqual(report["source"]["revision"], "a"*40)

    def test_different_source_is_rejected_even_when_traces_match(self):
        source = {"revision": "f"*40, "tree": "b"*40, "status": "", "index_flags": []}
        self.mutate(source=source, source_after=source)
        with self.assertRaisesRegex(ValueError, "different source"):
            compare.compare(self.roots)

    def test_missing_and_empty_traces_are_rejected(self):
        path = self.roots[1]/"blob-default.trace"
        path.unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            compare.compare(self.roots)
        path.write_text("")
        with self.assertRaisesRegex(ValueError, "empty"):
            compare.compare(self.roots)

    def test_changed_trace_is_rejected(self):
        (self.roots[1]/"blob-default.trace").write_text("different\n")
        with self.assertRaisesRegex(ValueError, "differs"):
            compare.compare(self.roots)

    def test_failed_or_changed_inputs_are_rejected(self):
        self.mutate(inputs_unchanged=False)
        with self.assertRaisesRegex(ValueError, "qualification failed"):
            compare.compare(self.roots)

    def test_missing_binary_hashes_and_changed_logs_are_rejected(self):
        self.mutate(binary_sha256={})
        with self.assertRaisesRegex(ValueError, "hashes"):
            compare.compare(self.roots)
        self.mutate(binary_sha256={str(i): "c"*64 for i in range(3)})
        (self.roots[1]/"qualification/release/check.log").write_text("changed")
        with self.assertRaisesRegex(ValueError, "log digest"):
            compare.compare(self.roots)

    def test_same_artifact_cannot_stand_in_for_two_platforms(self):
        with self.assertRaisesRegex(ValueError, "distinct"):
            compare.compare([self.roots[0], self.roots[0]])
        for profile in ("release", "dist"):
            row = json.loads((self.roots[1]/"qualification"/profile/"evidence.json").read_text())
            row.update(machine="x86_64", platform="linux")
            self.write(self.roots[1], profile, row)
        with self.assertRaisesRegex(ValueError, "distinct"):
            compare.compare(self.roots)


if __name__ == "__main__":
    unittest.main()
