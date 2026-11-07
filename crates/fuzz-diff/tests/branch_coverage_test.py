"""Branch evidence must be real, scoped and retained when a campaign fails."""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("branch_report", SCRIPTS / "branch-report.py")
reporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reporter)


class BranchReportTests(unittest.TestCase):
    def item(self, root, name="crates/tier-3-app-modules/sdk/src/coins.rs"):
        return {"filename": str(root / name), "branches": [[4, 2, 4, 9, 3, 0, 0, 0, 4]],
                "summary": {"branches": {"count": 2, "covered": 1}}}

    def test_arm_locations_and_exclusions(self):
        root = Path("/evidence")
        items = [self.item(root), self.item(root, "dependency/src/lib.rs"),
                 self.item(root, "crates/tier-1-protocol-types/celestia-proto/src/lib.rs"),
                 self.item(root, "crates/tier-3-app-modules/sdk/src/testing.rs")]
        report = reporter.collect({"data": [{"files": items}]}, root)
        self.assertEqual((report["count"], report["covered"]), (2, 1))
        self.assertEqual(len(report["files"]), 1)
        branch = next(iter(report["files"].values()))["uncovered"][0]
        self.assertEqual(branch["uncovered_arms"], ["false"])
        self.assertEqual(branch["start"], [4, 2])
        self.assertEqual(branch["review"], "pending")

    def test_missing_malformed_duplicate_and_inconsistent_are_errors(self):
        root = Path("/evidence")
        invalid = []
        missing = self.item(root)
        missing["branches"] = []
        invalid.append([missing])
        zero = self.item(root)
        zero["branches"] = []
        zero["summary"]["branches"] = {"count": 0, "covered": 0}
        invalid.append([zero])
        bad = self.item(root)
        bad["branches"][0][4] = -1
        invalid.append([bad])
        bad_summary = self.item(root)
        bad_summary["summary"]["branches"]["covered"] = True
        invalid.append([bad_summary])
        absent_summary = self.item(root)
        absent_summary["summary"] = {}
        invalid.append([absent_summary])
        invalid.append([self.item(root), self.item(root)])
        for items in invalid:
            with self.subTest(items=items), self.assertRaises(ValueError):
                reporter.collect({"data": [{"files": items}]}, root)

    def test_exported_regions_are_not_used_as_summary_denominator(self):
        root = Path("/evidence")
        item = self.item(root)
        item["branches"].append([8, 2, 8, 9, 0, 0, 0, 0, 4])
        report = reporter.collect({"data": [{"files": [item]}]}, root)
        self.assertEqual((report["count"], report["covered"]), (2, 1))
        file = next(iter(report["files"].values()))
        self.assertEqual(file["exported_region_arms"], {"count": 4, "covered": 1})
        self.assertEqual(len(file["uncovered"]), 2)

    def test_zero_summary_can_export_uncalled_regions(self):
        root = Path("/evidence")
        uncalled = self.item(root, "crates/tier-3-app-modules/sdk/src/tx.rs")
        uncalled["branches"][0][4:6] = [0, 0]
        uncalled["summary"]["branches"] = {"count": 0, "covered": 0}
        report = reporter.collect({"data": [{"files": [self.item(root), uncalled]}]}, root)
        self.assertEqual(report["count"], 2)
        self.assertEqual(report["files"]["crates/tier-3-app-modules/sdk/src/tx.rs"]["count"], 0)


class CoverageDriverTests(unittest.TestCase):
    def run_driver(self, branch="1", toolchain="nightly-2026-11-02", **extra):
        repeat = extra.pop("repeat", False)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            scripts = root / "crates/fuzz-diff"
            scripts.mkdir(parents=True)
            for name in ("coverage.sh", "branch-report.py"):
                shutil.copyfile(SCRIPTS / name, scripts / name)
            for name in ("deliver-diff.sh", "exec-diff.sh", "pp-diff.sh"):
                path = scripts / name
                path.write_text('#!/bin/sh\n[ "${MOCK_RUN_FAILURE:-0}" = 0 ]\n')
                path.chmod(0o755)
            (root / "bins").mkdir()
            tools = root / "tools"
            tools.mkdir()
            cargo = tools / "cargo"
            cargo.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
a = sys.argv[1:]
with open(os.environ['MOCK_COMMANDS'], 'a') as f: f.write(json.dumps(a)+'\\n')
if a and a[0].startswith('+'): a.pop(0)
if 'show-env' in a:
    if os.environ.get('MOCK_SETUP_FAILURE'): sys.exit(7)
elif a[0] == 'build':
    p = pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'release/fuzz-diff'
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text('#!/bin/sh\\nexit 0\\n'); p.chmod(0o755)
elif '--output-path' in a:
    p = pathlib.Path(a[a.index('--output-path')+1])
    if '--json' in a:
        p.write_text(os.environ['MOCK_LLVM'])
    else: p.write_text('lcov')
''')
            cargo.chmod(0o755)
            for name in ("rustc", "go"):
                p = tools / name
                p.write_text("#!/bin/sh\necho mock-toolchain\n")
                p.chmod(0o755)
            item = BranchReportTests().item(root)
            if extra.pop("no_branches", False):
                item["branches"] = []
                item["summary"]["branches"] = {"count": 0, "covered": 0}
            env = {**os.environ, "PATH": str(tools)+os.pathsep+os.environ["PATH"],
                   "DEVNET_BIN": str(root / "bins"), "CARGO_TARGET_DIR": str(root / "target"),
                   "COVERAGE_BRANCH": branch, "COVERAGE_TOOLCHAIN": toolchain,
                   "MOCK_COMMANDS": str(root / "commands"),
                   "MOCK_LLVM": json.dumps({"data": [{"files": [item]}]}), **extra}
            result = subprocess.run(["bash", str(scripts / "coverage.sh"), str(root / "out"), "1"],
                                    cwd=root, env=env, text=True, capture_output=True)
            if repeat:
                self.assertEqual(result.returncode, 0, result.stderr)
                result = subprocess.run(["bash", str(scripts / "coverage.sh"), str(root / "out"), "1"],
                                        cwd=root, env={**env, "COVERAGE_BRANCH": "0"}, text=True, capture_output=True)
            outputs = {p.name: p.read_text() for p in (root / "out").glob("*")}
            commands = [json.loads(s) for s in (root / "commands").read_text().splitlines()] if (root / "commands").exists() else []
            return result, outputs, commands

    def test_branch_mode_selects_toolchain_and_retains_inventory(self):
        result, outputs, commands = self.run_driver()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(c[0] == "+nightly-2026-11-02" for c in commands))
        self.assertTrue(any("show-env" in c and "--branch" in c for c in commands))
        self.assertEqual(json.loads(outputs["branches.json"])["covered"], 1)
        self.assertIn("rustc.txt", outputs)
        self.assertIn("send-authz-authz-feegrant.out", outputs)
        self.assertIn("send-authz-v9-authz-feegrant.out", outputs)
        self.assertIn("authz-grant-validation-authz-feegrant.out", outputs)
        self.assertIn("authz-grant-validation-v9-authz-feegrant.out", outputs)
        self.assertIn("ica-encodings-ibc.out", outputs)
        self.assertIn("ica-encodings-v9-ibc.out", outputs)
        self.assertIn("ica-order-ibc.out", outputs)
        self.assertIn("ica-order-v9-ibc.out", outputs)
        self.assertIn("upgrade-timeout-ibc.out", outputs)
        self.assertIn("upgrade-timeout-v9-ibc.out", outputs)
        self.assertIn("upgrade-timeout-boundary-ibc.out", outputs)
        self.assertIn("upgrade-timeout-boundary-v9-ibc.out", outputs)
        self.assertIn("upgrade-confirm-timeout-ibc.out", outputs)
        self.assertIn("upgrade-confirm-timeout-v9-ibc.out", outputs)
        self.assertIn("upgrade-try-fast-forward-ibc.out", outputs)
        self.assertIn("upgrade-try-fast-forward-v9-ibc.out", outputs)
        self.assertIn("upgrade-open-flushcomplete-ibc.out", outputs)
        self.assertIn("upgrade-open-flushcomplete-v9-ibc.out", outputs)

    def test_stable_mode_does_not_require_branch_data(self):
        result, outputs, commands = self.run_driver(branch="0", toolchain="", no_branches=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("branches.json", outputs)
        self.assertFalse(any("--branch" in c for c in commands))

    def test_setup_failure_stops_before_uninstrumented_build(self):
        result, _, commands = self.run_driver(MOCK_SETUP_FAILURE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("build" in c for c in commands))

    def test_missing_branch_counters_and_failed_runs_stay_failures(self):
        result, _, _ = self.run_driver(no_branches=True)
        self.assertNotEqual(result.returncode, 0)
        result, outputs, _ = self.run_driver(MOCK_RUN_FAILURE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("branches.json", outputs)

    def test_branch_requires_explicit_toolchain_and_valid_mode(self):
        for settings in ({"toolchain": ""}, {"branch": "true"}):
            result, _, commands = self.run_driver(**settings)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(commands, [])

    def test_reusing_output_cannot_mix_old_branch_inventory_with_new_mode(self):
        result, outputs, _ = self.run_driver(repeat=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("fresh output directory", result.stderr)
        self.assertIn("branch=1", outputs["coverage-mode.txt"])
        self.assertIn("branches.json", outputs)


if __name__ == "__main__":
    unittest.main()
