"""Evidence admission must fail closed without relabelling legacy responses."""
from copy import deepcopy
import importlib.util
from contextlib import redirect_stderr, redirect_stdout
import io
from types import SimpleNamespace
from unittest.mock import patch
import json
from pathlib import Path
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / "coverage.py"
SPEC = importlib.util.spec_from_file_location("rpc_coverage", MODULE)
coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(coverage)


class EvidenceAdmission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.policy = coverage.read_json(coverage.POLICY)
        self.review_ledger = coverage.read_json(
            coverage.ROOT / self.policy["reviewed_evidence"]
        )
        responses = {}
        for route, result in (("status", {"node_info": {"network": "celestia"}}),
                              ("abci_info", {"response": {"version": "10.5.0"}})):
            body = json.dumps({"result": result})
            responses[route] = {"body": body, "sha256": coverage.hashlib.sha256(body.encode()).hexdigest()}
        observation = {"version": "10.5.0", "endpoint": "https://rpc.example.org", "observed_at": "2026-10-30T20:00:00+00:00", "responses": responses}
        (self.root / "observation.json").write_text(json.dumps(observation))
        (self.root / "input").write_bytes(b"test input")
        (self.root / "test.rs").write_text("#[test]\nfn regression() {}\n")
        def artifact(path):
            return {"path": path, "sha256": coverage.digest(self.root / path)}
        self.manifest = {
            "oracle": {**self.policy["oracle"], "compiler": "go1.26.9", "build_tags": []},
            "binary_sha256": {"go": "a" * 64, "rs": "b" * 64},
            "inputs": {key: artifact("observation.json" if key == "mainnet_observation" else "input")
                       for key in ("genesis", "workload", "capture_source", "mainnet_observation")},
            "matched_state": {side: {"chain_id": "local-1", "height": 20, "app_hash": "c" * 64} for side in ("go", "rs")},
            "capture_command": "bounded-local-capture", "captured_at": "2026-10-30T20:30:00+00:00",
            "cases": [{"transport": "comet-uri", "endpoint": "status", "behavior": "runtime-identity", "comparison": "exact", "outcome": "exact",
                       **{key: artifact("input") for key in ("request", "go_response", "rs_response")},
                       "test": {"package": "rpc", "target": "corpus", "filter": "regression", "source": "test.rs"}}]}

    def validate(self):
        return coverage.validate_evidence(self.manifest, self.policy, self.root)

    def test_complete_manifest_is_admitted(self):
        self.validate()

    def test_old_release_cannot_be_admitted_as_current(self):
        self.manifest["oracle"]["celestia_app"] = "v10.4.0"
        with self.assertRaisesRegex(ValueError, "oracle mismatch"):
            self.validate()

    def test_dependency_commits_must_match_selected_releases(self):
        for key in ("celestia_core_commit", "cosmos_sdk_commit"):
            with self.subTest(key=key):
                original = self.manifest["oracle"][key]
                self.manifest["oracle"][key] = "a" * 40
                with self.assertRaisesRegex(ValueError, "oracle mismatch"):
                    self.validate()
                self.manifest["oracle"][key] = original

    def test_current_runner_rejects_zero_selected_tests(self):
        report = {"endpoints": [{"evidence": self.manifest["cases"]}],
                  "current_evidence_manifests": 1, "legacy_fixtures": []}
        with patch.object(coverage, "inventory", return_value=report), \
             patch.object(coverage.sys, "argv", ["coverage.py", "--run-current"]), \
             patch.object(coverage.subprocess, "run", return_value=SimpleNamespace(stdout="another_test: test\n")) as run, \
             redirect_stderr(io.StringIO()) as errors, redirect_stdout(io.StringIO()):
            self.assertEqual(coverage.main(), 1)
            self.assertIn("selected zero tests", errors.getvalue())
            self.assertEqual(run.call_count, 1, "do not run an empty passing filter")

    def test_changed_input_is_not_silently_accepted(self):
        (self.root / "input").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            self.validate()

    def test_mismatched_height_and_root_are_rejected(self):
        for key, value in (("height", 21), ("app_hash", "d" * 64)):
            with self.subTest(key=key):
                original = deepcopy(self.manifest["matched_state"])
                self.manifest["matched_state"]["rs"][key] = value
                with self.assertRaisesRegex(ValueError, "must match"):
                    self.validate()
                self.manifest["matched_state"] = original

    def test_normalization_cannot_count_as_exact(self):
        self.manifest["cases"][0]["comparison"] = "legacy-rpc-consensus-state"
        with self.assertRaisesRegex(ValueError, "never count as exact"):
            self.validate()

    def test_normalized_result_requires_a_normalization_policy(self):
        self.manifest["cases"][0].update(outcome="normalized", reason="sorted map entries")
        with self.assertRaisesRegex(ValueError, "never count as exact"):
            self.validate()

    def test_exact_comparison_may_record_a_mismatch(self):
        self.manifest["cases"][0].update(outcome="mismatch", reason="different bytes")
        self.validate()

    def test_missing_explanation_and_selector_fail(self):
        self.manifest["cases"][0]["outcome"] = "excluded"
        with self.assertRaisesRegex(ValueError, "why a comparison"):
            self.validate()
        self.manifest["cases"][0].update(outcome="exact")
        self.manifest["cases"][0]["test"]["filter"] = "does_not_exist"
        with self.assertRaisesRegex(ValueError, "function absent"):
            self.validate()

    def test_stale_or_later_mainnet_observation_fails(self):
        for capture in ("2026-10-29T20:00:00+00:00", "2026-11-01T20:00:00+00:00"):
            self.manifest["captured_at"] = capture
            with self.assertRaisesRegex(ValueError, "preceding mainnet observation"):
                self.validate()

    def test_out_of_tree_artifact_fails(self):
        self.manifest["inputs"]["genesis"]["path"] = "../outside"
        with self.assertRaisesRegex(ValueError, "out-of-tree"):
            self.validate()

    def test_inventory_distinguishes_declarations_and_reflection(self):
        endpoints = coverage.endpoints()
        indexed = {(e["transport"], e["endpoint"]): e for e in endpoints}
        self.assertIn(("comet-uri", "status"), indexed)
        self.assertNotIn(("comet-uri", "subscribe"), indexed)
        self.assertEqual(indexed[("cosmos-grpc", "/cosmos.bank.v1beta1.Query/Balance")]["declaration"], "descriptor-candidate")
        self.assertIn(("rest", "GET /cosmos/bank/v1beta1/balances/{address}/by_denom"), indexed)
        self.assertEqual(sum(e["declaration"] == "required-registration-check" for e in endpoints), 4)

    def test_merged_current_release_reviews_remain_unadmitted(self):
        reviews = coverage.validate_reviews(self.review_ledger, self.policy)
        self.assertEqual({review["issue"] for review in reviews}, {182, 184, 185, 186, 187, 188})
        self.assertTrue(all(review["status"] == "not-admitted" for review in reviews))
        self.assertTrue(all(review["matched_state"]["rs"] is None for review in reviews))

    def test_review_cannot_claim_an_unrecorded_rust_binary(self):
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 185)
        review["binary_sha256"]["rs"] = "d" * 64
        with self.assertRaisesRegex(ValueError, "inconsistent rs binary identity"):
            coverage.validate_reviews(self.review_ledger, self.policy)

    def test_review_artifact_and_regression_source_hashes_are_bound(self):
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 186)
        review["bindings"]["go_responses"][0]["sha256"] = "e" * 64
        with self.assertRaisesRegex(ValueError, "review artifact hash mismatch"):
            coverage.validate_reviews(self.review_ledger, self.policy)
        self.review_ledger = coverage.read_json(
            coverage.ROOT / self.policy["reviewed_evidence"]
        )
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 187)
        review["regressions"][0]["source_sha256"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "review artifact hash mismatch"):
            coverage.validate_reviews(self.review_ledger, self.policy)

    def test_review_cannot_hide_missing_matched_state(self):
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 182)
        review["admission_blockers"].remove("missing-rs-matched-state")
        with self.assertRaisesRegex(ValueError, "missing Rust capture provenance|matched state"):
            coverage.validate_reviews(self.review_ledger, self.policy)

    def test_every_artifact_class_has_a_symmetric_presence_blocker(self):
        present = {
            "oracle_build": (185, "missing-oracle-build-receipt"),
            "dependency_receipts": (185, "missing-dependency-receipts"),
            "genesis": (185, "missing-committed-genesis"),
            "workload": (185, "missing-committed-workload"),
            "capture_source": (185, "missing-capture-source"),
            "mainnet_observation": (185, "missing-complete-mainnet-observation"),
            "go_responses": (185, "missing-committed-go-response"),
        }
        for name, (issue, blocker) in present.items():
            with self.subTest(name=name, mutation="block-present-artifact"):
                ledger = deepcopy(self.review_ledger)
                review = next(item for item in ledger["reviews"] if item["issue"] == issue)
                review["admission_blockers"].append(blocker)
                with self.assertRaisesRegex(ValueError, rf"inconsistent {name} presence/blocker"):
                    coverage.validate_reviews(ledger, self.policy)
        absent = {
            "genesis": (182, "missing-committed-genesis"),
            "workload": (182, "missing-committed-workload"),
            "mainnet_observation": (182, "missing-complete-mainnet-observation"),
            "rs_responses": (185, "missing-committed-rs-response"),
            "oracle_build": (187, "missing-oracle-build-receipt"),
            "dependency_receipts": (187, "missing-dependency-receipts"),
            "capture_source": (187, "missing-capture-source"),
            "go_responses": (187, "missing-committed-go-response"),
        }
        for name, (issue, blocker) in absent.items():
            with self.subTest(name=name, mutation="hide-missing-artifact"):
                ledger = deepcopy(self.review_ledger)
                review = next(item for item in ledger["reviews"] if item["issue"] == issue)
                review["admission_blockers"].remove(blocker)
                with self.assertRaisesRegex(ValueError, rf"inconsistent {name} presence/blocker"):
                    coverage.validate_reviews(ledger, self.policy)

    def test_valid_artifact_cannot_be_rebound_to_another_class(self):
        for destination, source in (("genesis", "workload"),
                                    ("go_responses", "mainnet_observation")):
            with self.subTest(destination=destination, source=source):
                ledger = deepcopy(self.review_ledger)
                review = next(item for item in ledger["reviews"] if item["issue"] == 185)
                artifact = deepcopy(review["bindings"][source])
                review["bindings"][destination] = [artifact] if destination.endswith("responses") else artifact
                with self.assertRaisesRegex(ValueError, "artifact classes changed"):
                    coverage.validate_reviews(ledger, self.policy)

    def test_artifact_class_sets_reject_deletion_and_duplicates(self):
        for name in ("oracle_build", "dependency_receipts", "genesis", "workload",
                     "capture_source", "mainnet_observation", "go_responses"):
            with self.subTest(name=name, mutation="delete"):
                ledger = deepcopy(self.review_ledger)
                review = next(item for item in ledger["reviews"] if item["issue"] == 185)
                review["bindings"][name] = [] if isinstance(review["bindings"][name], list) else None
                with self.assertRaisesRegex(ValueError, "artifact classes changed"):
                    coverage.validate_reviews(ledger, self.policy)
        for name in ("dependency_receipts", "go_responses"):
            with self.subTest(name=name, mutation="duplicate"):
                ledger = deepcopy(self.review_ledger)
                review = next(item for item in ledger["reviews"] if item["issue"] == 185)
                review["bindings"][name].append(deepcopy(review["bindings"][name][0]))
                with self.assertRaisesRegex(ValueError, "artifact classes changed"):
                    coverage.validate_reviews(ledger, self.policy)
        ledger = deepcopy(self.review_ledger)
        review = next(item for item in ledger["reviews"] if item["issue"] == 185)
        review["bindings"]["rs_responses"].append(deepcopy(review["bindings"]["go_responses"][0]))
        with self.assertRaisesRegex(ValueError, "artifact classes changed"):
            coverage.validate_reviews(ledger, self.policy)

    def test_bound_state_values_are_parsed_from_the_receipt(self):
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 185)
        review["matched_state"]["go"]["height"] += 1
        with self.assertRaisesRegex(ValueError, "matched state disagrees"):
            coverage.validate_reviews(self.review_ledger, self.policy)

    def test_endpoint_and_selector_sets_reject_deletion_and_duplicates(self):
        for key, issue, error in (("endpoints", 182, "endpoint set changed"),
                                  ("regressions", 187, "regression selector set changed")):
            for mutation in ("delete", "duplicate"):
                with self.subTest(key=key, issue=issue, mutation=mutation):
                    ledger = deepcopy(self.review_ledger)
                    review = next(item for item in ledger["reviews"] if item["issue"] == issue)
                    if mutation == "delete":
                        review[key].pop()
                    else:
                        review[key].append(deepcopy(review[key][0]))
                    with self.assertRaisesRegex(ValueError, error):
                        coverage.validate_reviews(ledger, self.policy)

    def test_broadcast_review_binds_all_available_core_selectors(self):
        review = next(item for item in self.review_ledger["reviews"] if item["issue"] == 187)
        self.assertEqual(
            {item["filter"] for item in review["regressions"]},
            {"broadcast_tx_reaches_check_tx", "core_broadcast_projects_the_go_response_fields",
             "core_broadcast_is_served_over_http2"},
        )


if __name__ == "__main__":
    unittest.main()
