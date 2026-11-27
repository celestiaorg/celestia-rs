"""The report turns a collected block-range run into the headline table."""

import json
from pathlib import Path
import tempfile
import unittest

import report

GIB = 1024 ** 3


def summary(role, tip_s, mem_gib, rss_gib, home_gib, final=16206600):
    return {
        "role": role, "campaign": "block-range", "outcome": "ok", "snapshot": "16202000",
        "tip_t": str(tip_s), "tip_h": "16202500",
        "host": {"machine_type": "n2-highcpu-32", "cpu_model": "Intel Xeon", "vcpus": "32",
                 "mem_KiB": "32000000", "disk_layout": "nvme-raid0:4", "gfni": True, "sha_ni": True},
        "range": {"scheduled_start": 16205600, "blocks": 1000, "entry_h": 16205600, "exit_h": 16206600,
                  "elapsed_s": 6000.0, "final_app_height": final,
                  "mem_peak_B": int(mem_gib * GIB), "rss_peak_KiB": int(rss_gib * GIB / 1024),
                  "home_bytes": int(home_gib * GIB)},
    }


class ReportTest(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        pairs = {1: ((300, 2, 1, 10), (1500, 8, 6, 20)),
                 2: ((360, 3, 2, 12), (1200, 9, 6, 24)),
                 3: ((330, 2, 1, 11), (1400, 8, 5, 22))}
        for p, (rs, go) in pairs.items():
            self.write(f"bench-block-range-7-p{p}-rs", summary("rs", *rs))
            self.write(f"bench-block-range-7-p{p}-go", summary("go", *go))
        (self.root/"pairs.txt").write_text("pair p1: valid\npair p2: valid\npair p3: discard:different_snapshots\n")

    def write(self, vm, data):
        (self.root/vm).mkdir()
        (self.root/vm/"summary.json").write_text(json.dumps(data))

    def test_table_has_each_valid_pair_and_the_median(self):
        out = report.render(self.root)
        self.assertIn("| Time to verified tip (min) | p1 | 25.0 | 5.0 | 0.20 |", out)
        self.assertIn("| Peak cgroup memory over range (GiB) | p2 | 9.00 | 3.00 | 0.33 |", out)
        self.assertIn("| Node home after shutdown (GiB) | p1 | 20.0 | 10.0 | 0.50 |", out)
        # Median of pairs 1 and 2 only: pair 3 is discarded.
        self.assertIn("| Time to verified tip (min) | median | 22.5 | 5.5 | 0.24 |", out)
        self.assertIn("p3: discard:different_snapshots", out)
        self.assertNotIn("| p3 |", out)
        self.assertIn("n2-highcpu-32", out)
        self.assertIn("nvme-raid0:4", out)

    def test_ratio_of_medians_not_median_of_ratios(self):
        out = report.render(self.root)
        # Go median memory 8.5 GiB, Rust 2.5 GiB.
        self.assertIn("| Peak cgroup memory over range (GiB) | median | 8.50 | 2.50 | 0.29 |", out)

    def test_missing_side_is_reported_not_crashed(self):
        (self.root/"bench-block-range-7-p2-go"/"summary.json").unlink()
        (self.root/"pairs.txt").write_text("pair p1: valid\npair p2: discard:missing_or_invalid_summary\n")
        out = report.render(self.root)
        self.assertIn("p2: discard:missing_or_invalid_summary", out)
        self.assertIn("| Time to verified tip (min) | median | 25.0 | 5.0 | 0.20 |", out)

    def test_no_valid_pair(self):
        (self.root/"pairs.txt").write_text("pair p1: discard:incomplete_run\n")
        out = report.render(self.root)
        self.assertIn("No valid pair", out)


if __name__ == "__main__":
    unittest.main()
