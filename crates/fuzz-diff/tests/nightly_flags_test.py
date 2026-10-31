"""Check that nightly rotations combine signed ISMs with gas sweeps."""

from pathlib import Path
import re
import os
import shlex
import tempfile
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
            self.assertEqual("DELIVER_HL_REJECT" in flags, flags["DELIVER_HL_ISM"] != "noop")
            combinations.add((flags["DELIVER_HL_ISM"], "DELIVER_SWEEP" in flags))
        self.assertEqual(combinations, {
            (mode, sweep)
            for mode in ("noop", "message-id", "merkle-root")
            for sweep in (False, True)
        })


    def dry_runs(self, job, seed):
        script = Path(__file__).resolve().parents[1] / "nightly.sh"
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            # No oracle invocation or network access in a selection test.
            (root / "go").write_text("#!/bin/sh\nexit 0\n")
            (root / "go").chmod(0o755)
            result = subprocess.run(
                ["bash", str(script), job, str(seed), str(root / "findings")],
                env={**os.environ, "DEVNET_BIN": folder, "DRY_RUN": "1",
                     "BUDGET_SECS": "5400", "BLOCKS": "300",
                     "PATH": folder + os.pathsep + os.environ["PATH"]},
                check=True, capture_output=True, text=True,
            )
        runs = []
        for line in result.stdout.splitlines():
            if not line.startswith("RUN "):
                continue
            words = shlex.split(line.partition(": ")[2])
            flags = {}
            while "=" in words[0]:
                key, value = words.pop(0).split("=", 1)
                self.assertNotIn(key, flags)
                flags[key] = value
            runs.append((flags, words))
        return runs

    def test_upgrade_rotation_covers_signed_modes_and_flag_combinations(self):
        observed = set()
        for seed in range(18):
            runs = self.dry_runs("upgrade-0of1", seed)
            self.assertEqual(len(runs), 3)
            for flags, command in runs:
                self.assertEqual(command[-4], "v9-upgrade")
                self.assertEqual(flags["DELIVER_HL"], "1")
                self.assertIn("GO_APPD_NEXT", flags)
                self.assertIn("DELIVER_UPGRADE", flags)
                self.assertNotIn("DELIVER_HL_REJECT", flags)
                observed.add((flags["DELIVER_HL_ISM"],
                              "DELIVER_WIRE" in flags,
                              "DELIVER_SWEEP" in flags,
                              "DELIVER_RECHECK" in flags))
        # Existing WIRE and RECHECK alternate in opposite settings.
        self.assertEqual(observed, {
            (mode, wire, sweep, not wire)
            for mode in ("noop", "message-id", "merkle-root")
            for wire in (False, True) for sweep in (False, True)
        })

    def test_full_campaign_includes_each_mode_and_preserves_upgrade_seeds(self):
        runs = self.dry_runs("campaign-0of1", 7)
        scenarios = {"hyperlane", "zkism-forwarding", "v9-hyperlane",
                     "v9-zkism-forwarding"}
        coverage = set()
        upgrades = []
        for flags, command in runs:
            scenario = command[-4]
            if scenario == "v9-upgrade":
                upgrades.append((int(command[-1]), flags["DELIVER_HL_ISM"]))
                self.assertNotIn("DELIVER_HL_REJECT", flags)
                self.assertTrue(all(flags[key] == "1" for key in
                                    ("DELIVER_IBC", "DELIVER_HL", "DELIVER_WIRE",
                                     "DELIVER_SWEEP", "DELIVER_RECHECK")))
            elif scenario in scenarios:
                mode = flags["DELIVER_HL_ISM"]
                self.assertEqual(flags["DELIVER_HL"], "1")
                self.assertEqual("DELIVER_HL_REJECT" in flags, mode != "noop")
                coverage.add((scenario, mode, "DELIVER_GOVPASS" in flags))
            else:
                self.assertNotIn("DELIVER_HL_ISM", flags)
        self.assertEqual(coverage, {
            (scenario, mode, gov)
            for scenario in scenarios
            for mode in ("noop", "message-id", "merkle-root")
            for gov in (False, True)
        })
        self.assertEqual({seed for seed, _ in upgrades}, set(range(701, 713)))
        self.assertEqual(len(upgrades), 12)
        self.assertEqual([mode for _, mode in upgrades],
                         ["noop", "message-id", "merkle-root"] * 4)

    def test_campaign_shards_partition_required_signed_runs(self):
        full = self.dry_runs("campaign-0of1", 7)
        shards = [run for shard in range(8)
                  for run in self.dry_runs(f"campaign-{shard}of8", 7)]
        def identity(run):
            flags, command = run
            return (tuple(sorted((k, v) for k, v in flags.items()
                                 if k not in ("GO_APPD", "GO_APPD_NEXT"))),
                    tuple(command))
        self.assertCountEqual(list(map(identity, shards)), list(map(identity, full)))


if __name__ == "__main__":
    unittest.main()
