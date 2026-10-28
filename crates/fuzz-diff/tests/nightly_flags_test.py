"""Check that nightly rotations combine signed ISMs with gas sweeps."""

from pathlib import Path
import re
import subprocess
import unittest


class NightlyFlagsTest(unittest.TestCase):
    def test_each_ism_mode_runs_with_and_without_gas_sweeps(self):
        source = (Path(__file__).resolve().parents[1] / "nightly.sh").read_text()
        functions = []
        for name in ("pick", "deliver_flags"):
            match = re.search(rf"^{name}\(\) \{{\n.*?^\}}", source, re.M | re.S)
            self.assertIsNotNone(match, f"missing {name} function")
            functions.append(match.group())
        result = subprocess.run(
            ["bash", "-c", "\n".join(functions) +
             '\nfor seed in {0..8}; do deliver_flags hyperlane "$seed"; done'],
            check=True, capture_output=True, text=True,
        )
        combinations = set()
        for line in result.stdout.splitlines():
            flags = dict(word.split("=", 1) for word in line.split())
            self.assertEqual(flags["DELIVER_HL"], "1")
            combinations.add((flags["DELIVER_HL_ISM"], "DELIVER_SWEEP" in flags))
        self.assertEqual(combinations, {
            (mode, sweep)
            for mode in ("noop", "message-id", "merkle-root")
            for sweep in (False, True)
        })


if __name__ == "__main__":
    unittest.main()
