"""The behavior matrix must not upgrade or silently omit scoped evidence."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

MODULE = Path(__file__).resolve().parents[1] / "scope_audit.py"
SPEC = importlib.util.spec_from_file_location("rpc_scope_audit_test", MODULE)
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class ScopedEvidenceAudit(unittest.TestCase):
    def setUp(self):
        self.value = json.loads(audit.AUDIT.read_text())

    def test_complete_scoped_inventory_preserves_readiness_limit(self):
        result = audit.validate(self.value)
        self.assertFalse(result["provider_ready"])
        self.assertEqual({row["issue"] for row in result["behaviors"]}, set(range(182, 193)))

    def test_package_deletion_and_duplicate_are_rejected(self):
        for packages in (self.value["packages"][:-1], self.value["packages"] + [self.value["packages"][0]]):
            self.value["packages"] = packages
            with self.assertRaisesRegex(ValueError, "package inventory"):
                audit.validate(self.value)

    def test_go_only_capture_cannot_be_promoted_to_parity(self):
        row = next(row for row in self.value["packages"] if row["id"] == "rest-post")
        row["comparison"] = "exact"
        with self.assertRaisesRegex(ValueError, "comparison class"):
            audit.validate(self.value)

    def test_merged_live_transaction_replay_stays_bound_with_its_limits(self):
        result = audit.validate(self.value)
        row = next(row for row in result["packages"] if row["id"] == "live-transactions")
        self.assertEqual(row["comparison"], "normalized-runtime-comparison")
        self.assertEqual(row["runtime_regressions"][0]["source"]["path"],
                         "crates/tier-5-node/celestia-rs/tests/live_tx_transports.rs")
        self.assertEqual(row["runtime_regressions"][0]["function"],
                         "admitted_live_transactions_match_rust_production_transports")
        self.assertIn("50319", row["normalizations"][0])
        self.assertIn("50307", row["normalizations"][0])
        self.assertTrue(row["known_mismatches"])
        self.assertTrue(any("Final Go/Rust app-root" in item for item in row["exclusions"]))
        self.assertFalse(result["provider_ready"])

    def test_proxy_tls_framing_closure_scope_and_limits_stay_bound(self):
        result = audit.validate(self.value)
        row = next(row for row in result["packages"] if row["id"] == "proxy-tls-framing")
        self.assertEqual(row["comparison"], "normalized-runtime-comparison")
        self.assertEqual(len(row["runtime_regressions"]), 3)
        self.assertIn("duplicate-preserving", row["normalizations"][0])
        self.assertIn("X-Server-Time", row["normalizations"][1])
        self.assertEqual(len(row["known_mismatches"]), 6)
        self.assertTrue(any("close_notify" in item for item in row["known_mismatches"]))
        self.assertTrue(any("Public-provider" in item for item in row["exclusions"]))
        self.assertTrue(any("Go1.26.9" in item for item in row["provenance_limits"]))
        behavior = next(row for row in result["behaviors"] if row["issue"] == 191)
        self.assertEqual(
            behavior["packages"],
            ["comet-http", "proxy-tls-framing", "rest-heights", "rest-post"],
        )
        self.assertIn("acceptance is covered", behavior["remaining_gaps"][0])

        mutations = (
            ("normalizations", []),
            ("known_mismatches", []),
            ("scope", "universal exact parity"),
            ("provenance_limits", ["none"]),
            ("exclusions", ["none"]),
            ("runtime_regressions", row["runtime_regressions"][:-1]),
        )
        for field, value in mutations:
            with self.subTest(field=field):
                changed = json.loads(audit.AUDIT.read_text())
                package = next(
                    package
                    for package in changed["packages"]
                    if package["id"] == "proxy-tls-framing"
                )
                package[field] = value
                with self.assertRaises(ValueError):
                    audit.validate(changed)

    def test_grpc_lifecycle_closure_scope_and_limits_stay_bound(self):
        result = audit.validate(self.value)
        row = next(row for row in result["packages"] if row["id"] == "grpc-lifecycle")
        self.assertEqual(row["comparison"], "relational-go-plus-rust-captures")
        self.assertEqual(row["transports"], ["core-grpc", "cosmos-grpc"])
        self.assertEqual(row["runtime_regressions"], [])
        self.assertIn("quota0/2", row["scope"])
        self.assertEqual(len(row["normalizations"]), 1)
        self.assertEqual(len(row["known_mismatches"]), 3)
        self.assertEqual(len(row["exclusions"]), 5)
        self.assertEqual(len(row["provenance_limits"]), 3)
        self.assertTrue(any("399f57e" in item for item in row["provenance_limits"]))
        behavior = next(row for row in result["behaviors"] if row["issue"] == 183)
        self.assertEqual(behavior["packages"], ["grpc-lifecycle", "grpc-limits"])
        self.assertIn("Acceptance is covered", behavior["remaining_gaps"][0])
        self.assertFalse(result["provider_ready"])

        for field, value in (
            ("normalizations", []),
            ("known_mismatches", []),
            ("scope", "universal exact parity"),
            ("provenance_limits", ["none"]),
            ("exclusions", ["none"]),
        ):
            with self.subTest(field=field):
                changed = json.loads(audit.AUDIT.read_text())
                package = next(
                    package for package in changed["packages"]
                    if package["id"] == "grpc-lifecycle"
                )
                package[field] = value
                with self.assertRaisesRegex(ValueError, "canonical reviewed audit"):
                    audit.validate(changed)

    def test_provider_readiness_and_missing_issue_cannot_be_hidden(self):
        self.value["provider_ready"] = True
        with self.assertRaisesRegex(ValueError, "provider readiness"):
            audit.validate(self.value)
        self.value["provider_ready"] = False
        self.value["behaviors"] = self.value["behaviors"][:-1]
        with self.assertRaisesRegex(ValueError, "every issue"):
            audit.validate(self.value)

    def test_rebound_valid_receipt_and_stale_hash_are_rejected(self):
        row = self.value["packages"][0]
        row["receipt"] = deepcopy(self.value["packages"][1]["receipt"])
        with self.assertRaisesRegex(ValueError, "receipt/documentation class"):
            audit.validate(self.value)
        self.value = json.loads(audit.AUDIT.read_text())
        self.value["packages"][0]["receipt"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            audit.validate(self.value)

    def test_offline_verifier_cannot_be_removed(self):
        row = next(row for row in self.value["packages"] if row["verifier"])
        row["verifier"] = None
        with self.assertRaisesRegex(ValueError, "verifier cannot"):
            audit.validate(self.value)

    def test_runtime_selectors_cannot_be_omitted_or_rebound(self):
        row = next(row for row in self.value["packages"] if row["runtime_regressions"])
        row["runtime_regressions"] = []
        with self.assertRaisesRegex(ValueError, "runtime selector set"):
            audit.validate(self.value)

    def test_websocket_closure_bindings_and_limits_cannot_regress(self):
        row = next(row for row in self.value["packages"] if row["id"] == "websocket-lifecycle")
        row["receipt"]["path"] = "conformance/websocket-lifecycle/testdata/manifest.json"
        with self.assertRaisesRegex(ValueError, "receipt/documentation class"):
            audit.validate(self.value)

        self.value = json.loads(audit.AUDIT.read_text())
        row = next(row for row in self.value["packages"] if row["id"] == "websocket-lifecycle")
        row["verifier"]["path"] = "conformance/websocket-lifecycle/verify.py"
        with self.assertRaisesRegex(ValueError, "verifier path changed"):
            audit.validate(self.value)

        self.value = json.loads(audit.AUDIT.read_text())
        row = next(row for row in self.value["packages"] if row["id"] == "websocket-lifecycle")
        row["runtime_regressions"].pop()
        with self.assertRaisesRegex(ValueError, "runtime selector set"):
            audit.validate(self.value)

        self.value = json.loads(audit.AUDIT.read_text())
        row = next(row for row in self.value["behaviors"] if row["issue"] == 190)
        row["remaining_gaps"] = ["complete exact parity"]
        with self.assertRaisesRegex(ValueError, "canonical reviewed audit"):
            audit.validate(self.value)

    def test_transport_scope_cannot_be_broadened(self):
        row = next(row for row in self.value["packages"] if row["id"] == "simulation")
        row["transports"].append("rest")
        with self.assertRaisesRegex(ValueError, "transport inventory"):
            audit.validate(self.value)

    def test_each_behavior_binds_every_package_and_retains_gaps(self):
        row = next(row for row in self.value["behaviors"] if row["issue"] == 191)
        row["packages"] = []
        with self.assertRaisesRegex(ValueError, "all packages"):
            audit.validate(self.value)

    def test_reviewed_baseline_cannot_be_rebound(self):
        self.value["audited_main"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "canonical reviewed audit"):
            audit.validate(self.value)

    def test_deviations_scope_and_provenance_limits_cannot_be_erased(self):
        for field, value in (("normalizations", []), ("known_mismatches", []),
                             ("scope", "universal exact parity"),
                             ("provenance_limits", ["none"]), ("exclusions", ["none"])):
            with self.subTest(field=field):
                self.value = json.loads(audit.AUDIT.read_text())
                package = next(row for row in self.value["packages"] if row["id"] == "reflection")
                package[field] = value
                with self.assertRaisesRegex(ValueError, "canonical reviewed audit"):
                    audit.validate(self.value)

    def test_uncovered_persistence_campaign_cannot_be_marked_complete(self):
        row = next(row for row in self.value["behaviors"] if row["issue"] == 189)
        row["remaining_gaps"] = ["complete"]
        with self.assertRaisesRegex(ValueError, "canonical reviewed audit"):
            audit.validate(self.value)

    def test_verifier_execution_uses_only_registered_python_sources(self):
        with patch.object(audit.subprocess, "run") as run:
            audit.verify_scoped(self.value)
        expected = [[audit.sys.executable, str(audit.ROOT / row["verifier"]["path"])]
                    for row in self.value["packages"] if row["verifier"]]
        self.assertEqual([call.args[0] for call in run.call_args_list], expected)
        self.assertTrue(all(call.kwargs["check"] for call in run.call_args_list))


if __name__ == "__main__":
    unittest.main()
