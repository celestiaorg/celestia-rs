"""The evidence pipeline must reject missing/failed runs, not print green zeros."""

import json
from pathlib import Path
import tempfile
import unittest

import coverage
import fuzz_stats


class CoverageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/tier-4-runtime/abci-server", "crates/tier-1-protocol-types/celestia-proto", "crates/tier-0-primitives/hash"]\n'
        )
        self.path = "crates/tier-4-runtime/abci-server/src/codec.rs"

    def item(self, path, count=10, covered=8):
        return {
            "filename": str(self.root / path),
            "summary": {
                metric: {"count": count, "covered": covered}
                for metric in coverage.METRICS
            },
        }

    def test_aggregation_and_scope(self):
        raw = {
            "data": [
                {
                    "files": [
                        self.item(self.path),
                        self.item(
                            "crates/tier-4-runtime/abci-server/src/server.rs", 20, 1
                        ),
                        self.item(
                            "crates/tier-4-runtime/abci-server/src/tests.rs", 1000, 1000
                        ),
                        self.item(
                            "crates/tier-4-runtime/abci-server/tests/integration.rs"
                        ),
                        self.item(
                            "crates/tier-1-protocol-types/celestia-proto/src/lib.rs"
                        ),
                        self.item("crates/fuzz-diff/src/lib.rs"),
                        self.item("/tmp/dependency/src/lib.rs"),
                    ]
                }
            ]
        }
        crates, files = coverage.collect(raw, self.root)
        self.assertEqual(len(files), 2)
        self.assertEqual(
            crates["crates/tier-4-runtime/abci-server"]["lines"],
            {"count": 30, "covered": 9},
        )
        self.assertEqual(crates["crates/tier-0-primitives/hash"]["files"], 0)
        self.assertEqual(
            coverage.cell(crates["crates/tier-0-primitives/hash"]["lines"]),
            "not measured",
        )

    def test_budget_checks_uncovered_count_and_missing_files(self):
        policy = {self.path: {"max_uncovered_lines": 2}}
        _, files = coverage.collect(
            {"data": [{"files": [self.item(self.path)]}]}, self.root
        )
        self.assertEqual(coverage.check(files, policy), [])
        files[self.path]["lines"]["covered"] = 7
        self.assertEqual(len(coverage.check(files, policy)), 1)
        self.assertEqual(len(coverage.check({}, policy)), 1)

    def test_missing_duplicate_and_invalid_data_are_errors(self):
        for items in [
            [],
            [self.item(self.path)] * 2,
            [self.item(self.path, 10, 11)],
            [self.item(self.path, 0, 0)],
        ]:
            with self.subTest(items=items), self.assertRaises(ValueError):
                coverage.collect({"data": [{"files": items}]}, self.root)

    def test_machine_readable_roundtrip(self):
        crates, files = coverage.collect(
            {"data": [{"files": [self.item(self.path)]}]}, self.root
        )
        report = {
            "revision": "abc",
            "suite": "test",
            "worktree_dirty": False,
            "crates": crates,
            "files": files,
            "failures": [],
        }
        self.assertEqual(json.loads(json.dumps(report)), report)
        self.assertIn("not measured", coverage.render(report))


class FuzzTests(unittest.TestCase):
    LOG = """#2 INITED cov: 7 ft: 9 corp: 1/1b exec/s: 0 rss: 20Mb
#123 DONE cov: 27 ft: 39 corp: 8/80b lim: 4 exec/s: 61 rss: 22Mb
stat::number_of_executed_units: 123
stat::average_exec_per_sec:     61
stat::peak_rss_mb:             22
"""

    def test_complete_run(self):
        result = fuzz_stats.parse(self.LOG, 0)
        self.assertTrue(result["completed"])
        self.assertEqual(result["edges"], 27)
        self.assertEqual(result["features"], 39)
        self.assertEqual(result["executions"], 123)
        self.assertEqual(result["peak_rss_mb"], 22)

    def test_failed_or_incomplete_never_counts_as_success(self):
        for log, status in [
            (self.LOG, 77),
            (self.LOG, None),
            ("", 0),
            ("ERROR: AddressSanitizer", 1),
            (self.LOG.split("stat::")[0], 0),
            (self.LOG.replace("DONE", "NEW"), 0),
        ]:
            with self.subTest(log=log, status=status):
                self.assertFalse(fuzz_stats.parse(log, status)["completed"])


if __name__ == "__main__":
    unittest.main()
