"""Retained samples cannot silently fill time/identity/height gaps."""

from copy import deepcopy
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import observation_audit as audit


BASE = datetime(2026, 11, 12, tzinfo=timezone.utc)
IDENTITY = {"pid": 1, "inode": 2, "device": 3, "size": 4, "mtime_ns": 5}
CANDIDATE, BINARY, ADDRESS = "a"*40, "b"*64, "C"*40


def at(seconds):
    return (BASE+timedelta(seconds=seconds)).isoformat()


def receipt(**fields):
    return {"time_utc": at(0), "chain_id": "celestia", "candidate_commit": CANDIDATE, **fields}


def sample(seconds):
    status = {"node_info": {"network": "celestia"}, "validator_info": {"address": ADDRESS},
              "sync_info": {"latest_block_height": "100", "catching_up": False,
                            "latest_block_time": at(seconds-2)}}
    return receipt(time_utc=at(seconds), outcome="observed", identity=IDENTITY,
                   binary_sha256=BINARY, binary_measured_at_utc=at(seconds-2),
                   local_status_time_utc=at(seconds-1), reference_status_time_utc=at(seconds-0.5),
                   local_status=deepcopy(status), reference_status=status, reference_height_minus_local=0)


def height(h, seconds):
    return receipt(time_utc=at(seconds+1), executed_height=h, outcome="match", identity=IDENTITY,
                   reference_chain_id="celestia", local_app_hash="e"*64, reference_app_hash="e"*64,
                   reference_header_height=h+1, reference_block_hash="f"*64,
                   local_results_sha256="d"*64, reference_results_sha256="d"*64,
                   local_tx_count=2, reference_tx_count=2,
                   compared_fields=["code", "data", "gas_wanted", "gas_used"],
                   participation={"outcome": "signed"},
                   execution={"app_hash": "e"*64, "block_hash": "f"*64,
                              "log_line": f"{at(seconds)} committed block height={h} hash={'f'*64} app_hash={'e'*64}"})


class ObservationAuditTest(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        self.events = [receipt(event="start", config={"validator_address": ADDRESS},
                               identity={**IDENTITY, "binary_sha256": BINARY, "candidate_commit": CANDIDATE}),
                       receipt(event="baseline", first_required_height=41,
                               local_status={"validator_info": {"address": ADDRESS}})]
        self.samples = [sample(t) for t in (1, 11, 21, 29)]
        self.heights = [height(41, 5), height(42, 15)]

    def run_audit(self, **overrides):
        for name, rows in (("events", self.events), ("samples", self.samples), ("heights", self.heights)):
            (self.root/(name+".jsonl")).write_text("".join(json.dumps(row)+"\n" for row in rows))
        options = dict(directory=self.root, chain="celestia", candidate=CANDIDATE, binary=BINARY,
                       first=41, last=42, start=at(0), end=at(30))
        return audit.audit(**{**options, **overrides})

    def test_bounded_sample_and_height_checks_do_not_certify_qualification(self):
        report = self.run_audit()
        self.assertTrue(report["sample_requirements_met"])
        self.assertTrue(all(row["complete"] for row in report["height_audits"].values()))
        self.assertEqual(report["participation_counts"]["signed"], 2)
        self.assertFalse(report["qualification_established"])
        self.assertEqual(len(report["artifacts"]), 3)

    def test_requested_trailing_time_and_height_gaps_are_not_inferred_away(self):
        report = self.run_audit(end=at(61), last=43)
        self.assertFalse(report["sample_requirements_met"])
        self.assertEqual(report["sample_gaps"][-1]["seconds"], 32)
        self.assertEqual(report["missing_participation_ranges"], [[43, 43]])
        self.assertEqual(report["height_audits"]["apphash"]["missing_ranges"], [[43, 43]])

    def test_unavailable_sample_survives_recovery(self):
        self.samples[1].update(outcome="unavailable", error="reference timeout")
        report = self.run_audit()
        self.assertEqual(len(report["sample_failures"]), 1)
        self.assertFalse(report["sample_requirements_met"])
        self.assertEqual(report["valid_sample_count"], 3)

    def test_sample_identity_and_validator_changes_are_rejected(self):
        original = deepcopy(self.samples)
        for field, value in (("candidate_commit", "f"*40), ("binary_sha256", "f"*64),
                              ("identity", {**IDENTITY, "pid": 2})):
            with self.subTest(field=field):
                self.samples = deepcopy(original)
                self.samples[1][field] = value
                self.assertFalse(self.run_audit()["sample_requirements_met"])
        self.samples = deepcopy(original)
        self.samples[1]["local_status"]["validator_info"]["address"] = "D"*40
        self.assertFalse(self.run_audit()["sample_requirements_met"])

    def test_stale_block_time_and_inconsistent_lag_are_rejected(self):
        self.samples[1]["local_status"]["sync_info"]["latest_block_time"] = at(-100)
        self.assertIn("stale", self.run_audit()["sample_failures"][0]["error"])
        self.samples[1] = sample(11)
        self.samples[1]["reference_height_minus_local"] = 100
        self.assertIn("lag", self.run_audit()["sample_failures"][0]["error"])

    def test_duplicate_samples_and_collector_stop_are_preserved(self):
        self.samples.insert(2, deepcopy(self.samples[1]))
        self.events.append(receipt(event="stopped", time_utc=at(20), error="process changed"))
        report = self.run_audit()
        self.assertIn("duplicate", report["sample_failures"][0]["error"])
        self.assertEqual(len(report["collector_interruptions"]), 1)

    def test_execution_link_and_conflicting_participation_are_rejected(self):
        self.heights[0]["execution"]["app_hash"] = "a"*64
        self.assertEqual(len(self.run_audit()["participation_errors"]), 1)
        self.heights[0] = height(41, 5)
        self.heights.append({**height(41, 5), "participation": {"outcome": "missed"}})
        self.assertEqual(len(self.run_audit()["participation_errors"]), 1)

    def test_unavailable_height_attempt_is_not_erased_by_success(self):
        self.heights.insert(1, receipt(time_utc=at(7), executed_height=41, outcome="unavailable", error="timeout"))
        report = self.run_audit()
        self.assertFalse(report["height_audits"]["apphash"]["complete"])
        self.assertEqual(report["height_audits"]["apphash"]["missing_ranges"], [[41, 41]])

    def test_invalid_window_or_initial_binary_fails(self):
        for overrides in ({"start": at(-1)}, {"first": 40}, {"binary": "f"*64},
                          {"max_gap": float("nan")}, {"max_age": float("inf")}):
            with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                self.run_audit(**overrides)

    def test_inputs_that_change_during_audit_are_rejected(self):
        original = audit.receipt_coverage.audit
        def changed(*args, **kwargs):
            result = original(*args, **kwargs)
            with (self.root/"samples.jsonl").open("a") as target:
                target.write("\n")
            return result
        with patch.object(audit.receipt_coverage, "audit", side_effect=changed):
            with self.assertRaisesRegex(ValueError, "changed during audit"):
                self.run_audit()


if __name__ == "__main__":
    unittest.main()
