import tempfile
import unittest
from pathlib import Path

from coverage import missing, verified_range


class CoverageTest(unittest.TestCase):
    def parse(self, text):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "follow.log"
            path.write_text(text)
            return verified_range(path)

    def test_finite_run(self):
        stop = "2026-11-28T00:00:00Z  INFO celestia_rs::follow: stop height reached height=30 verified_header_height=31\n"
        end = "2026-11-28T00:00:00Z  INFO celestia_rs::follow: follow finished first=21 last=30 blocks=10 txs=1 secs=42\n"
        self.assertEqual(self.parse(stop + end), (21, 30))
        for invalid in [end, stop, end + stop, stop.replace("=31", "=32") + end,
                        stop + end.replace("blocks=10", "blocks=9"), stop + end + stop + end]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.parse(invalid)

    def test_restarts_overlap_and_gaps(self):
        ranges = [(20, 28), (10, 15), (13, 16), (20, 28), (40, 41)]
        self.assertEqual(list(missing(ranges, 1, 30)), [(1, 9), (17, 19), (29, 30)])
        self.assertEqual(list(missing(ranges, 20, 21)), [])
