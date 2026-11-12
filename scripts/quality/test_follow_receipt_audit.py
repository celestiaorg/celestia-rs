"""Follower evidence must preserve exact scope, order and result commitments."""

from copy import deepcopy
import gzip
import json
from pathlib import Path
import tempfile
import unittest

import follow_receipt_audit as audit

CANDIDATE = "c"*40
BINARY = "b"*64
VECTORS = json.loads((Path(__file__).parent/"fixtures/follow_results_vectors.json").read_text())


def ledger(first=1):
    def row(event, data):
        return {"schema_version": 1, "event": event, "time_utc": "2026-11-12T00:00:00Z", "data": data}
    last = first+1
    rows = [row("run_start", {
        "candidate_commit": CANDIDATE, "binary_sha256": BINARY, "software_version": "0.1.0",
        "chain_id": "test", "requested_stop_height": last, "process_proposal_checked": True,
        "witness_interval": 0, "reference_server_count": 1, "scope": audit.SCOPE,
    }), row("baseline", {"anchor_height": first-1, "anchor_block_hash": "" if first == 1 else "aa"*32,
                        "anchor_app_hash": "ab"*32, "first_execution_height": first})]
    if first > 1:
        rows.append(row("anchor_verified", {"height": first-1, "reference_header_height": first,
                                            "reference_header_hash": f"{first:02x}"*32}))
    for offset, height in enumerate(range(first, last+1)):
        vector = VECTORS["cases"][2+offset]
        rows.append(row("height", {
            "executed_height": height, "local_block_hash": f"{height:02x}"*32,
            "local_app_hash": "cd"*32, "reference_app_hash": "cd"*32,
            "local_results_hash": vector["hash"], "reference_results_hash": vector["hash"],
            "reference_header_height": height+1, "reference_header_hash": f"{height+1:02x}"*32,
            "reference_server_index": 0, "next_header_app_version": 10,
            "tx_count": len(vector["results"]), "tx_results": deepcopy(vector["results"]),
        }))
    rows.append(row("run_end", {"status": "complete", "execution_error": False,
                               "first_execution_height": first, "last_executed_height": last,
                               "last_verified_height": last, "executed_blocks": 2, "executed_transactions": 2}))
    return rows


class FollowerReceiptAudit(unittest.TestCase):
    def run_audit(self, rows, first=1, raw=None, compressed=False):
        data = raw if raw is not None else ("".join(json.dumps(row)+"\n" for row in rows)).encode()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/("ledger.jsonl.gz" if compressed else "ledger.jsonl")
            path.write_bytes(gzip.compress(data, mtime=0) if compressed else data)
            return audit.audit(path, "test", CANDIDATE, BINARY, first, first+1)

    def test_wire_and_merkle_hashes_match_pinned_go_core(self):
        v9 = json.loads((Path(__file__).parent/"fixtures/follow_results_vectors_v9.json").read_text())
        for vectors, core, go in [(VECTORS, "v0.42.8", "go1.26.10"), (v9, "v0.40.9", "go1.26.9")]:
            self.assertEqual(vectors["core"], "github.com/celestiaorg/celestia-core@"+core)
            self.assertEqual(vectors["go"], go)
            for vector in vectors["cases"]:
                with self.subTest(core=core, vector=vector):
                    self.assertEqual(audit.results_hash(vector["results"]), vector["hash"])

    def test_complete_plain_and_compressed_ledgers_keep_qualification_false(self):
        plain = self.run_audit(ledger())
        compressed = self.run_audit(ledger(), compressed=True)
        self.assertEqual(plain, compressed)
        self.assertEqual(plain["covered_range"], [1, 2])
        self.assertEqual(plain["transactions"], 2)
        self.assertTrue(plain["complete"])
        self.assertFalse(plain["qualification_established"])
        self.assertFalse(plain["network_signature_proof_verified"])

    def test_resume_anchor_is_not_reported_as_execution(self):
        result = self.run_audit(ledger(6), first=6)
        self.assertEqual(result["covered_range"], [6, 7])
        self.assertEqual(result["covered_heights"], 2)
        rows = ledger(6)
        del rows[2]
        with self.assertRaisesRegex(ValueError, "missing anchor"):
            self.run_audit(rows, first=6)

    def test_identity_and_scope_cannot_be_rebound(self):
        for key, value in (("chain_id", "another"), ("candidate_commit", "a"*40),
                           ("binary_sha256", "a"*64), ("requested_stop_height", 3),
                           ("process_proposal_checked", False), ("scope", "fully qualified")):
            rows = ledger()
            rows[0]["data"][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "identity or verification scope"):
                self.run_audit(rows)

    def test_missing_reordered_and_duplicated_heights_fail(self):
        missing, reordered, duplicate = ledger(), ledger(), ledger()
        del missing[2]
        reordered[2], reordered[3] = reordered[3], reordered[2]
        duplicate.insert(3, deepcopy(duplicate[2]))
        for rows in (missing, reordered, duplicate):
            with self.subTest(rows=rows), self.assertRaisesRegex(ValueError, "executed height"):
                self.run_audit(rows)

    def test_transaction_fields_and_roots_cannot_change(self):
        for key, value in (("code", 99), ("data", "AQ=="), ("gas_wanted", -99), ("gas_used", 999)):
            rows = ledger()
            rows[2]["data"]["tx_results"][0][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "result commitment"):
                self.run_audit(rows)
        rows = ledger()
        rows[2]["data"]["local_results_hash"] = rows[2]["data"]["reference_results_hash"] = "0"*64
        with self.assertRaisesRegex(ValueError, "result commitment"):
            self.run_audit(rows)

    def test_bad_numeric_base64_and_result_count_fail(self):
        for key, value in (("code", True), ("code", 2**32), ("gas_wanted", 2**63),
                           ("gas_used", -2**63-1), ("data", "not base64")):
            rows = ledger()
            rows[2]["data"]["tx_results"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.run_audit(rows)
        rows = ledger()
        rows[2]["data"]["tx_count"] = 2
        with self.assertRaisesRegex(ValueError, "result commitment"):
            self.run_audit(rows)

    def test_header_links_app_hash_and_reference_bounds_fail(self):
        for index, key, value in ((3, "local_block_hash", "f"*64), (2, "reference_app_hash", "e"*64),
                                  (2, "reference_header_height", 3), (2, "reference_server_index", 1)):
            rows = ledger()
            rows[index]["data"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.run_audit(rows)

    def test_noncomplete_missing_and_trailing_terminal_records_fail(self):
        for status in ("failed", "interrupted", "incomplete"):
            rows = ledger()
            rows[-1]["data"]["status"] = status
            with self.subTest(status=status), self.assertRaisesRegex(ValueError, "not complete"):
                self.run_audit(rows)
        with self.assertRaisesRegex(ValueError, "missing complete run_end"):
            self.run_audit(ledger()[:-1])
        with self.assertRaisesRegex(ValueError, "trailing record"):
            self.run_audit(ledger()+[ledger()[-1]])

    def test_terminal_counts_and_verified_boundary_are_bound(self):
        for key in ("first_execution_height", "last_executed_height", "last_verified_height",
                    "executed_blocks", "executed_transactions"):
            rows = ledger()
            rows[-1]["data"][key] = 0
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.run_audit(rows)
        rows = ledger()
        del rows[3]
        with self.assertRaisesRegex(ValueError, "missing requested heights"):
            self.run_audit(rows)

    def test_duplicate_keys_truncation_and_non_utc_timestamps_fail(self):
        raw = "".join(json.dumps(row)+"\n" for row in ledger())
        duplicate = raw.replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1', 1)
        for value in (duplicate.encode(), raw.rstrip().encode()):
            with self.assertRaises(ValueError):
                self.run_audit([], raw=value)
        rows = ledger()
        rows[0]["time_utc"] = "2026-11-12Z"
        with self.assertRaisesRegex(ValueError, "timestamp must be UTC"):
            self.run_audit(rows)


if __name__ == "__main__":
    unittest.main()
