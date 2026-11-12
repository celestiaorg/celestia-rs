"""Observer receipts must preserve missing evidence and process boundaries."""

import base64
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("observe", Path(__file__).with_name("observe_validator.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
AUDIT_SPEC = importlib.util.spec_from_file_location("receipt_audit", Path(__file__).with_name("receipt_coverage.py"))
AUDIT = importlib.util.module_from_spec(AUDIT_SPEC)
AUDIT_SPEC.loader.exec_module(AUDIT)


class ObserverTest(unittest.TestCase):
    def test_results_reject_invalid_data_and_preserve_large_signed_gas(self):
        row = {"height": "42", "txs_results": [{"gas_wanted": "9223372036854775807", "gas_used": "-1"}]}
        self.assertEqual(MODULE.results(row, 42)[0]["gas_wanted"], "9223372036854775807")
        for value in (False, 0.5, "unknown", "9223372036854775808"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.results({"height": "42", "txs_results": [{"gas_used": value}]}, 42)
        for value in (None, False, {"data": "bad base64"}, {"code": -1}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.results({"height": "42", "txs_results": [value]}, 42)
        self.assertEqual(MODULE.results({"height": "42", "txs_results": None}, 42), [])
        with self.assertRaises(ValueError):
            MODULE.results(row, 43)

    def test_reference_participation_distinguishes_signed_nil_and_unknown(self):
        entry = {"validator_address": "A", "block_id_flag": 2,
                 "signature": base64.b64encode(b"s"*64).decode(), "timestamp": "now"}
        self.assertEqual(MODULE.participation({"signatures": [entry]}, "A")["outcome"], "signed")
        self.assertEqual(MODULE.participation({"signatures": [{**entry, "block_id_flag": 3}]}, "A")["outcome"], "nil")
        self.assertEqual(MODULE.participation({"signatures": []}, "A")["outcome"], "absent_or_not_in_set")
        with self.assertRaises(ValueError):
            MODULE.participation({"signatures": [entry, entry]}, "A")

    def observer(self, folder):
        root = Path(folder)
        binary = root / "binary"
        binary.write_bytes(b"candidate bytes")
        log = root / "node.log"
        log.write_text("")
        config = {"candidate_commit": "a"*40, "binary": str(binary), "pid": 123,
                  "home": str(root / "home"), "log": str(log), "chain_id": "test",
                  "validator_address": "A", "local_rpc": "local", "reference_rpc": "reference"}
        self.current_identity = {"pid": 123, "inode": 1}
        self.height = 40
        self.fail_reference = False
        self.nil_signature = False
        self.fail_results = False
        self.wrong_app_hash = False
        self.wrong_results_hash = False

        def request(endpoint, route):
            if endpoint == "reference" and self.fail_reference:
                raise OSError("reference unavailable")
            if route == "/status":
                return {"node_info": {"network": "test"}, "validator_info": {"address": "A"},
                        "sync_info": {"latest_block_height": str(self.height), "catching_up": False}}
            if route.startswith("/block_results"):
                if self.fail_results and endpoint == "reference":
                    raise OSError("results unavailable")
                return {"height": str(self.height-1), "txs_results": [{"code": 0}]}
            if route.startswith("/block?"):
                return {"block": {"header": {"chain_id": "test", "height": str(self.height),
                                              "last_results_hash": "e"*64 if self.wrong_results_hash else MODULE.results_hash([
                                                  {"code": 0, "gas_wanted": 0, "gas_used": 0, "data": ""}]),
                                              "app_hash": "c"*64 if self.wrong_app_hash else "b"*64}}}
            if route.startswith("/commit?"):
                return {"signed_header": {"header": {"chain_id": "test"}, "commit": {
                    "height": str(self.height-1), "block_id": {"hash": "d"*64},
                    "signatures": [] if self.nil_signature else [{"validator_address": "A", "block_id_flag": 2,
                        "signature": base64.b64encode(b"s"*64).decode(), "timestamp": "now"}]}}}
            if route.startswith("/validators?"):
                return {"block_height": str(self.height-1), "validators": [{"address": "A"}], "total": "1"}
            raise AssertionError(route)

        with patch.object(MODULE.subprocess, "check_output", return_value="commit: " + "a"*40 + "\n"):
            observer = MODULE.Observer(config, root / "out", request, lambda _: self.current_identity)
        self.addCleanup(observer.pool.shutdown)
        self.addCleanup(observer.log.close)
        return observer, log

    def advance(self, log):
        self.height = 42
        with log.open("a") as target:
            target.write("2026-11-12T00:00:00Z committed block height=41 hash=" + "d"*64 + " app_hash=" + "b"*64 + "\n")

    def test_complete_height_joins_execution_results_reference_and_signing(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, log = self.observer(folder)
            observer.step()
            self.advance(log)
            observer.step()
            row = json.loads((observer.output / "heights.jsonl").read_text())
            self.assertEqual(row["outcome"], "match")
            self.assertEqual(row["executed_height"], 41)
            self.assertEqual(row["participation"]["outcome"], "signed")
            self.assertEqual(observer.next_height, 42)
            self.assertEqual(len((observer.output / "samples.jsonl").read_text().splitlines()), 2)
            for kind in ("apphash", "tx-results"):
                audit = AUDIT.audit([observer.output / "heights.jsonl"], "test", "a"*40, 41, 41, kind)
                self.assertTrue(audit["complete"], audit)

    def test_failed_attempt_is_retained_before_retry_and_cursor_does_not_skip(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, log = self.observer(folder)
            observer.step()
            self.advance(log)
            self.fail_results = True
            observer.step()
            self.assertEqual(observer.next_height, 41)
            self.fail_results = False
            observer.step()
            rows = [json.loads(line) for line in (observer.output / "heights.jsonl").read_text().splitlines()]
            self.assertEqual([row["outcome"] for row in rows], ["unavailable", "match"])

    def test_equal_results_must_match_next_header_commitment(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, log = self.observer(folder)
            observer.step()
            self.advance(log)
            self.wrong_results_hash = True
            with self.assertRaisesRegex(ValueError, "parity mismatch"):
                observer.step()
            row = json.loads((observer.output / "heights.jsonl").read_text())
            self.assertEqual(row["outcome"], "mismatch")
            self.assertEqual(row["local_results_sha256"], row["reference_results_sha256"])
            self.assertNotEqual(row["local_results_hash"], row["header_results_hash"])
            self.assertEqual(row["local_tx_results"], row["reference_tx_results"])
            self.assertEqual(observer.next_height, 41)

    def test_reference_failure_retains_local_tip_sample(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, _ = self.observer(folder)
            self.fail_reference = True
            observer.step()
            sample = json.loads((observer.output / "samples.jsonl").read_text())
            self.assertEqual(sample["outcome"], "unavailable")
            self.assertEqual(sample["local_status"]["sync_info"]["latest_block_height"], "40")

    def test_pid_change_and_log_loss_end_observation(self):
        for failure in ("pid", "log"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as folder:
                observer, log = self.observer(folder)
                observer.step()
                self.advance(log)
                if failure == "pid":
                    self.current_identity = {"pid": 124, "inode": 1}
                else:
                    log.rename(log.with_suffix(".old"))
                    log.write_text("")
                with self.assertRaises(ValueError):
                    observer.step()

    def test_missed_signature_uses_reference_validator_set(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, log = self.observer(folder)
            observer.step()
            self.advance(log)
            self.nil_signature = True
            observer.step()
            row = json.loads((observer.output / "heights.jsonl").read_text())
            self.assertEqual(row["participation"]["outcome"], "missed")

    def test_execution_hash_mismatch_is_recorded_without_advancing(self):
        with tempfile.TemporaryDirectory() as folder:
            observer, log = self.observer(folder)
            observer.step()
            self.advance(log)
            self.wrong_app_hash = True
            with self.assertRaises(ValueError):
                observer.step()
            row = json.loads((observer.output / "heights.jsonl").read_text())
            self.assertEqual(row["outcome"], "mismatch")
            self.assertEqual(observer.next_height, 41)


if __name__ == "__main__":
    unittest.main()
