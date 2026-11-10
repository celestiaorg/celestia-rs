"""Offline evidence audits must preserve gaps and refuse misleading matches."""

import copy
import json
import tempfile
import unittest
from pathlib import Path

from receipt_coverage import audit


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
    def run_audit(self, rows, first=10, last=12, kind="tx-results"):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "receipts.jsonl"
            path.write_text("".join(json.dumps(row) + "\n" for row in rows))
            return audit([path], "celestia", CANDIDATE, first, last, kind)

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
