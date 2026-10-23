"""Evidence accounting must not turn retries or failed runs into saturation."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "saturation_report", Path(__file__).parents[1] / "saturation-report.py"
)
REPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPORT)


class EvidenceTest(unittest.TestCase):
    def run_evidence(self, root, name, *, revision="a" * 40, rc=0, dirty=False, **stats):
        folder = Path(root) / name
        folder.mkdir()
        (folder / "revision.txt").write_text(revision + "\n")
        (folder / "worktree.status").write_text(" M file\n" if dirty else "")
        (folder / "worktree.patch").write_text("")
        (folder / "go-build-info.txt").write_text(
            f"v10/celestia-appd: {REPORT.PINNED_GO}\n"
            f"v9/celestia-appd: {REPORT.PINNED_GO}\n"
        )
        result = dict(scenario="gov", seed=1, requested_blocks=10,
                      txs_per_block=8, blocks=10, delivered=17,
                      delivered_ok=5, mismatches=0, split=False)
        result.update(stats)
        (folder / "run.err").write_text("DELIVER_STATS " + json.dumps(result) + "\n")
        (folder / "run.exit").write_text(str(rc))
        (folder / "run.env").write_text(f"GO_APPD=/{name}/appd\nDELIVER_IBC=1\n")
        return folder

    def test_only_clean_complete_successful_runs_count(self):
        with tempfile.TemporaryDirectory() as root:
            self.run_evidence(root, "clean")
            self.run_evidence(root, "retry")
            self.run_evidence(root, "other-seed", seed=2)
            self.run_evidence(root, "dirty", dirty=True)
            self.run_evidence(root, "failure", rc=1)
            self.run_evidence(root, "mismatch", mismatches=1)
            self.run_evidence(root, "split", split=True)
            self.run_evidence(root, "short", blocks=9)
            runs, excluded = REPORT.collect([root])
            self.assertEqual(len(runs), 2)
            self.assertEqual(sum(s["delivered"] for s in runs.values()), 34)
            self.assertEqual(excluded, {"duplicate": 1, "uncommitted source": 1,
                                        "failed or mismatched": 3, "incomplete block count": 1})

    def test_revisions_are_separate_and_conflicting_duplicates_fail(self):
        with tempfile.TemporaryDirectory() as root:
            self.run_evidence(root, "first")
            self.run_evidence(root, "other-revision", revision="b" * 40)
            runs, _ = REPORT.collect([root])
            self.assertEqual(len(runs), 2)
            self.run_evidence(root, "conflict", delivered=18)
            with self.assertRaisesRegex(ValueError, "conflicting duplicate"):
                REPORT.collect([root])

    def test_missing_exit_evidence_and_impossible_counters_fail(self):
        with tempfile.TemporaryDirectory() as root:
            folder = self.run_evidence(root, "missing")
            (folder / "run.exit").unlink()
            with self.assertRaisesRegex(ValueError, "invalid evidence"):
                REPORT.collect([root])
        with tempfile.TemporaryDirectory() as root:
            self.run_evidence(root, "bad-count", delivered_ok=18)
            with self.assertRaisesRegex(ValueError, "invalid success count"):
                REPORT.collect([root])

    def test_unpinned_go_builds_do_not_count(self):
        with tempfile.TemporaryDirectory() as root:
            folder = self.run_evidence(root, "newer-go")
            (folder / "go-build-info.txt").write_text(
                "v10/celestia-appd: go1.27.0\nv9/celestia-appd: go1.27.0\n"
            )
            runs, excluded = REPORT.collect([root])
            self.assertFalse(runs)
            self.assertEqual(excluded, {"unpinned Go toolchain": 1})

if __name__ == "__main__":
    unittest.main()
