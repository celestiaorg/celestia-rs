"""The behavior matrix must not upgrade or silently omit scoped evidence."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import tempfile
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

    def test_live_transaction_closure_stays_bound_with_its_limits(self):
        result = audit.validate(self.value)
        row = next(row for row in result["packages"] if row["id"] == "live-tx-closure")
        self.assertEqual(row["comparison"], "paired-live-transport-closure")
        self.assertEqual(row["transports"], [
            "comet-uri", "comet-jsonrpc", "comet-websocket",
            "core-grpc", "cosmos-grpc", "rest",
        ])
        self.assertIn("28-case", row["scope"])
        self.assertEqual(len(row["normalizations"]), 5)
        self.assertEqual(len(row["known_mismatches"]), 1)
        self.assertTrue(any("Go1.26.9" in item for item in row["provenance_limits"]))
        behavior = next(row for row in result["behaviors"] if row["issue"] == 187)
        self.assertEqual(behavior["packages"], ["live-transactions", "live-tx-closure"])
        self.assertIn("Acceptance is covered", behavior["remaining_gaps"][0])
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
            ("scope", ""),
            ("provenance_limits", []),
            ("exclusions", []),
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

    def test_populated_governance_scope_and_remaining_gaps_stay_bound(self):
        result = audit.validate(self.value)
        row = next(row for row in result["packages"]
                   if row["id"] == "populated-governance")
        self.assertEqual(row["comparison"],
                         "exact-protobuf-structural-json-comparison")
        self.assertEqual(row["transports"],
                         ["comet-jsonrpc", "cosmos-grpc", "rest"])
        self.assertIn("four transport paths", row["scope"])
        self.assertIn("three Rust listeners", row["scope"])
        self.assertEqual(row["normalizations"], [])
        self.assertEqual(row["known_mismatches"], [])
        self.assertTrue(any("Go proof" in item for item in row["exclusions"]))
        self.assertEqual(
            row["runtime_regressions"][0]["function"],
            "admitted_populated_inputs_match_all_rust_transports",
        )
        behavior = next(row for row in result["behaviors"] if row["issue"] == 188)
        self.assertEqual(
            behavior["packages"],
            ["module-closure", "populated-governance", "populated-staking"],
        )
        gaps = " ".join(behavior["remaining_gaps"])
        for required in ("IBC", "authz", "feegrant", "vesting", "Hyperlane",
                         "fibre", "forwarding", "zkism", "UTF-8", "cursor",
                         "history", "proof"):
            self.assertIn(required, gaps)
        self.assertFalse(result["provider_ready"])

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

    def test_audit_text_is_not_pinned_by_a_hash(self):
        # Scope, gaps and the baseline commit are prose and data that ordinary
        # pull requests edit. Only their structure is checked.
        self.value["audited_main"] = "0" * 40
        package = next(row for row in self.value["packages"] if row["id"] == "reflection")
        package["scope"] += " (reworded)"
        row = next(row for row in self.value["behaviors"] if row["issue"] == 189)
        row["remaining_gaps"].append("another gap")
        audit.validate(self.value)
        self.assertFalse(hasattr(audit, "REVIEWED_AUDIT_SHA256"))

    def test_scope_limits_and_gaps_cannot_be_emptied(self):
        for field, value in (("scope", ""), ("provenance_limits", []), ("exclusions", [])):
            with self.subTest(field=field):
                self.value = json.loads(audit.AUDIT.read_text())
                package = next(row for row in self.value["packages"] if row["id"] == "reflection")
                package[field] = value
                with self.assertRaisesRegex(ValueError, "scope, provenance limits and exclusions"):
                    audit.validate(self.value)
        self.value = json.loads(audit.AUDIT.read_text())
        row = next(row for row in self.value["behaviors"] if row["issue"] == 189)
        row["remaining_gaps"] = []
        with self.assertRaisesRegex(ValueError, "remaining gaps"):
            audit.validate(self.value)

    def test_living_files_are_bound_by_path_and_frozen_evidence_by_hash(self):
        # Documentation and Rust sources change in ordinary pull requests, so
        # they are bound by path. Receipts and verifiers stay hash-bound.
        for row in self.value["packages"]:
            self.assertEqual(set(row["documentation"]), {"path"})
            self.assertEqual(set(row["receipt"]), {"path", "sha256"})
            if row["verifier"] is not None:
                self.assertEqual(set(row["verifier"]), {"path", "sha256"})
            for regression in row["runtime_regressions"]:
                self.assertEqual(set(regression["source"]), {"path"})
        audit.validate(self.value)
        self.value["packages"][0]["documentation"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "bound by path only"):
            audit.validate(self.value)
        self.value = json.loads(audit.AUDIT.read_text())
        self.value["packages"][0]["documentation"]["path"] = "conformance/mainnet-rpc/missing.md"
        with self.assertRaisesRegex(ValueError, "receipt/documentation class"):
            audit.validate(self.value)

    def test_living_file_must_exist_in_tree_and_not_be_a_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "real.md").write_text("text")
            (root / "link.md").symlink_to(root / "real.md")
            self.assertEqual(audit.living(root, {"path": "real.md"}), root / "real.md")
            for name in ("missing.md", "link.md", "../outside.md"):
                with self.subTest(name=name):
                    with self.assertRaisesRegex(ValueError, "missing, symlinked or out-of-tree"):
                        audit.living(root, {"path": name})

    def test_runtime_source_with_a_hash_is_rejected(self):
        row = next(row for row in self.value["packages"] if row["runtime_regressions"])
        row["runtime_regressions"][0]["source"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "bound by path only"):
            audit.validate(self.value)

    def test_stale_verifier_hash_is_rejected(self):
        row = next(row for row in self.value["packages"] if row["verifier"])
        row["verifier"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
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
