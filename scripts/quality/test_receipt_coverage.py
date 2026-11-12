"""Offline evidence audits must preserve gaps and refuse misleading matches."""

import copy
import json
import hashlib
import tempfile
import unittest
from pathlib import Path

from receipt_coverage import audit
from follow_receipt_audit import results_hash


CANDIDATE = "a" * 40


def receipt(height=10):
    return {
        "time_utc": "2026-11-06T05:19:48Z", "executed_height": height,
        "chain_id": "celestia", "reference_chain_id": "celestia",
        "candidate_commit": CANDIDATE, "outcome": "match", "error": None,
        "local_app_hash": "B" * 64, "reference_app_hash": "B" * 64,
        "reference_header_height": height + 1,
        "compared_fields": ["code", "data", "gas_wanted", "gas_used"],
        "local_tx_count": 3, "reference_tx_count": 3,
        "local_results_sha256": "c" * 64, "reference_results_sha256": "c" * 64,
        "first_difference_index": None,
    }


class ReceiptCoverageTests(unittest.TestCase):
    def run_audit(self, rows, first=10, last=12, kind="tx-results", require_commitment=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "receipts.jsonl"
            path.write_text("".join(json.dumps(row) + "\n" for row in rows))
            return audit([path], "celestia", CANDIDATE, first, last, kind, require_commitment)

    def committed(self):
        row = receipt()
        rows = [{"code": "0", "data": "YQ==", "gas_wanted": "9223372036854775807", "gas_used": "-1"}]
        digest = hashlib.sha256(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        root = results_hash([{**rows[0], "code": 0, "gas_wanted": 2**63-1, "gas_used": -1}])
        row.update(local_tx_count=1, reference_tx_count=1,
                   local_results_sha256=digest, reference_results_sha256=digest,
                   local_results_hash=root, reference_results_hash=root, header_results_hash=root,
                   local_tx_results=rows, reference_tx_results=copy.deepcopy(rows))
        return row

    def test_commitments_recomputed_and_legacy_requirement_explicit(self):
        report = self.run_audit([self.committed()], last=10, require_commitment=True)
        self.assertTrue(report["complete"])
        self.assertEqual(report["result_commitment_checked_heights"], 1)
        self.assertFalse(self.run_audit([receipt()], last=10, require_commitment=True)["complete"])
        self.assertTrue(self.run_audit([receipt()], last=10)["complete"])
        self.assertEqual(self.run_audit([receipt()], last=10)["result_commitment_checked_heights"], 0)

    def test_partial_or_tampered_commitments_fail_without_opt_in(self):
        for changes in ({"header_results_hash": "d"*64}, {"local_results_hash": None},
                        {"local_tx_results": []}, {"reference_tx_results": [{}]},
                        {"reference_header_height": 10}, {"local_tx_count": 2, "reference_tx_count": 2}):
            with self.subTest(changes=changes):
                row = self.committed();row.update(changes)
                self.assertFalse(self.run_audit([row], last=10)["complete"])
        row = receipt();row["header_results_hash"] = "e"*64
        self.assertFalse(self.run_audit([row], last=10)["complete"])
        for data in ("@@", "YR=="):
            row = self.committed()
            for prefix in ("local", "reference"):
                row[prefix+"_tx_results"][0]["data"] = data
                row[prefix+"_results_sha256"] = hashlib.sha256(json.dumps(row[prefix+"_tx_results"], sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            self.assertFalse(self.run_audit([row], last=10)["complete"])

    def test_duplicate_scrapes_are_not_additional_height_coverage(self):
        report = self.run_audit([receipt(10), receipt(10), receipt(12)])
        self.assertEqual(report["covered_ranges"], [[10, 10], [12, 12]])
        self.assertEqual(report["missing_ranges"], [[11, 11]])
        self.assertEqual(report["counts"]["duplicates"], 1)
        self.assertFalse(report["complete"])

    def test_empty_input_and_large_ranges_have_compact_gaps(self):
        report = self.run_audit([], first=1, last=10**12)
        self.assertEqual(report["missing_ranges"], [[1, 10**12]])
        self.assertIsNone(report["receipt_time_range_utc"])

    def test_wrong_candidate_or_chain_cannot_fill_a_gap(self):
        for field, value in (("candidate_commit", "d" * 40), ("chain_id", "mocha-5"),
                             ("reference_chain_id", "mocha-5")):
            with self.subTest(field=field):
                row = receipt(11)
                row[field] = value
                report = self.run_audit([receipt(10), row, receipt(12)])
                self.assertEqual(report["missing_ranges"], [[11, 11]])
                self.assertEqual(report["counts"]["identity_rejected"], 1)

    def test_unavailable_check_cannot_be_erased_by_later_success(self):
        row = receipt(11)
        row.update(outcome="unavailable", error="reference_block_results_unavailable")
        for rows in ([row, receipt(11)], [receipt(11), row]):
            report = self.run_audit([receipt(10), *rows, receipt(12)])
            self.assertFalse(report["complete"])
            self.assertEqual(report["rejected_ranges"], [[11, 11]])

    def test_identity_rejected_attempt_remains_a_gap_after_successful_retry(self):
        for field, value in (("reference_chain_id", None), ("chain_id", None),
                             ("candidate_commit", None), ("candidate_commit", "d" * 40)):
            for kind in ("apphash", "tx-results"):
                with self.subTest(field=field, kind=kind):
                    row = receipt(11)
                    row.update(outcome="unavailable", error="reference_status_unavailable")
                    row[field] = value
                    for rows in ([row, receipt(11)], [receipt(11), row]):
                        report = self.run_audit([receipt(10), *rows, receipt(12)], kind=kind)
                        self.assertFalse(report["complete"])
                        self.assertEqual(report["covered_ranges"], [[10, 10], [12, 12]])
                        self.assertEqual(report["covered_heights"], 2)
                        self.assertEqual(report["missing_ranges"], [[11, 11]])
                        self.assertEqual(report["rejected_ranges"], [[11, 11]])
                        self.assertEqual(report["counts"]["identity_rejected"], 1)

    def test_match_label_alone_does_not_establish_result_parity(self):
        for changes in ({"reference_results_sha256": "d" * 64},
                        {"reference_tx_count": 4}, {"compared_fields": ["code"]},
                        {"local_tx_count": True, "reference_tx_count": True},
                        {"first_difference_index": 0}):
            with self.subTest(changes=changes):
                row = receipt(11)
                row.update(changes)
                report = self.run_audit([receipt(10), row, receipt(12)])
                self.assertEqual(report["missing_ranges"], [[11, 11]])

    def test_conflicting_successful_duplicates_fail_in_both_orders(self):
        row = receipt(11)
        row.update(local_results_sha256="d" * 64, reference_results_sha256="d" * 64)
        for rows in ([row, receipt(11)], [receipt(11), row]):
            report = self.run_audit([receipt(10), *rows, receipt(12)])
            self.assertEqual(report["rejected_ranges"], [[11, 11]])
            self.assertFalse(report["complete"])

    def test_app_hash_requires_next_header_and_matching_hash(self):
        for changes in ({"reference_header_height": 11},
                        {"reference_app_hash": "D" * 64}, {"local_app_hash": ""}):
            with self.subTest(changes=changes):
                row = receipt(11)
                row.update(changes)
                report = self.run_audit([receipt(10), row, receipt(12)], kind="apphash")
                self.assertEqual(report["missing_ranges"], [[11, 11]])
        self.assertTrue(self.run_audit([receipt(h) for h in range(10, 13)], kind="apphash")["complete"])

    def test_complete_coverage_counts_unique_heights_and_hashes_inputs(self):
        report = self.run_audit([receipt(12), receipt(10), receipt(11), receipt(10)])
        self.assertTrue(report["complete"])
        self.assertEqual(report["covered_ranges"], [[10, 12]])
        self.assertEqual(report["covered_heights"], 3)
        self.assertRegex(report["artifacts"][0]["sha256"], r"^[a-f0-9]{64}$")

    def test_invalid_json_height_and_timestamp_fail_closed(self):
        for changes in ({"executed_height": True}, {"executed_height": "11"},
                        {"time_utc": "2026-11-06T05:19:48"}, {"time_utc": None}):
            with self.subTest(changes=changes):
                row = copy.deepcopy(receipt(11))
                row.update(changes)
                with self.assertRaises(ValueError):
                    self.run_audit([row])
        with self.assertRaises(ValueError):
            self.run_audit([[]])

    def test_unlocated_failure_prevents_complete_claim(self):
        row = receipt()
        row.update(executed_height=None, outcome="unavailable", error="local_abci_unavailable")
        report = self.run_audit([receipt(h) for h in range(10, 13)] + [row])
        self.assertEqual(report["missing_ranges"], [])
        self.assertFalse(report["complete"])


if __name__ == "__main__":
    unittest.main()
