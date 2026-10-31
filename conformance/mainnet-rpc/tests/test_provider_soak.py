"""The closing #192 soak evidence must remain complete and fail closed."""

import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest


CAPTURE = (
    Path(__file__).resolve().parents[1]
    / "provider-soak/v10.5.0/2026-10-31-809bfca"
)
SPEC = importlib.util.spec_from_file_location("provider_soak_verify", CAPTURE / "verify.py")
verify = importlib.util.module_from_spec(SPEC)
OLD_DONT_WRITE_BYTECODE = sys.dont_write_bytecode
try:
    sys.dont_write_bytecode = True
    SPEC.loader.exec_module(verify)
finally:
    sys.dont_write_bytecode = OLD_DONT_WRITE_BYTECODE


class ProviderSoakEvidence(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.capture = Path(self.temp.name) / "capture"
        shutil.copytree(CAPTURE, self.capture)
        self.original_here = verify.HERE
        verify.HERE = self.capture
        self.addCleanup(setattr, verify, "HERE", self.original_here)

    def update_checksum(self, name):
        lines = []
        for line in (self.capture / "SHA256SUMS").read_text().splitlines():
            sha256, listed = line.split("  ", 1)
            if listed == name:
                sha256 = verify.digest(self.capture / name)
            lines.append(f"{sha256}  {listed}")
        (self.capture / "SHA256SUMS").write_text("\n".join(lines) + "\n")

    def mutate_json(self, name, change):
        path = self.capture / name
        value = json.loads(path.read_text())
        change(value)
        path.write_text(json.dumps(value, indent=2) + "\n")
        self.update_checksum(name)

    def mutate_observation(self, change):
        observation = json.loads((self.capture / "mainnet-observation.json").read_text())
        change(observation)
        (self.capture / "mainnet-observation.json").write_text(
            json.dumps(observation, indent=2) + "\n"
        )
        provenance = json.loads((self.capture / "provider-provenance.json").read_text())
        provenance["mainnet_observation"] = observation
        (self.capture / "provider-provenance.json").write_text(
            json.dumps(provenance, indent=2) + "\n"
        )
        self.update_checksum("mainnet-observation.json")
        self.update_checksum("provider-provenance.json")

    def test_complete_capture_passes(self):
        verify.verify_hashes()
        verify.verify_provenance()
        verify.verify_report()

    def test_changed_artifact_hash_fails(self):
        with (self.capture / "events.log").open("a") as output:
            output.write("{}\n")
        with self.assertRaisesRegex(ValueError, "capture hash mismatch"):
            verify.verify_hashes()

    def test_checksum_inventory_cannot_omit_an_artifact(self):
        sums = (self.capture / "SHA256SUMS").read_text().splitlines()
        (self.capture / "SHA256SUMS").write_text("\n".join(sums[:-1]) + "\n")
        with self.assertRaisesRegex(ValueError, "exact public capture set"):
            verify.verify_hashes()

    def test_unlisted_file_cannot_hide_outside_checksum_inventory(self):
        (self.capture / "private-material").write_text("must not be published\n")
        with self.assertRaisesRegex(ValueError, "exact public file set"):
            verify.verify_hashes()

    def test_publication_support_file_cannot_be_removed(self):
        (self.capture / "README.md").unlink()
        with self.assertRaisesRegex(ValueError, "exact public file set"):
            verify.verify_hashes()

    def test_non_final_report_fails_after_rehash(self):
        self.mutate_json("report.json", lambda report: report.__setitem__("final", False))
        verify.verify_hashes()
        with self.assertRaisesRegex(ValueError, "duration/height/health gate"):
            verify.verify_report()

    def test_missing_provider_operation_fails_after_rehash(self):
        def change(report):
            report["metrics"]["provider"]["operations"].pop("n4:rest-node-info")
        self.mutate_json("report.json", change)
        with self.assertRaisesRegex(ValueError, "matrix is incomplete"):
            verify.verify_report()

    def test_full_node_cannot_be_relabelled_as_validator(self):
        self.mutate_json(
            "report.json",
            lambda report: report["nodes"][4].__setitem__("validator", True),
        )
        with self.assertRaisesRegex(ValueError, "mixed-node topology changed"):
            verify.verify_report()

    def test_failed_provider_result_fails_after_rehash(self):
        path = self.capture / "events.log"
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        next(row for row in rows if row["ev"] == "provider-result")["ok"] = False
        path.write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows))
        self.update_checksum("events.log")
        with self.assertRaisesRegex(ValueError, "query/subscription result failed"):
            verify.verify_report()

    def test_provider_events_cannot_be_rebound_to_one_node(self):
        path = self.capture / "events.log"
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        for row in rows:
            if row["ev"] in {"provider-result", "provider-broadcast"}:
                row["node"] = 0
                row["node_kind"] = "go"
        path.write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows))
        self.update_checksum("events.log")
        with self.assertRaisesRegex(ValueError, "per-node operation matrix"):
            verify.verify_report()

    def test_noncontiguous_block_hashes_fail_after_rehash(self):
        path = self.capture / "hashes.jsonl"
        path.write_text("\n".join(path.read_text().splitlines()[1:]) + "\n")
        self.update_checksum("hashes.jsonl")
        with self.assertRaisesRegex(ValueError, "not contiguous"):
            verify.verify_report()

    def test_full_node_cannot_be_recorded_as_block_proposer(self):
        def change(report):
            report["proposed_by_node"]["0"] -= 1
            report["proposed_by_node"]["4"] = 1

        self.mutate_json("report.json", change)
        with self.assertRaisesRegex(ValueError, "proposer counts"):
            verify.verify_report()

    def test_candidate_commit_rebinding_fails_after_rehash(self):
        self.mutate_json(
            "provider-provenance.json",
            lambda provenance: provenance.__setitem__("source_commit", "0" * 40),
        )
        with self.assertRaisesRegex(ValueError, "source commit mismatch"):
            verify.verify_provenance()

    def test_go_build_recipe_module_rebinding_fails_after_rehash(self):
        self.mutate_json(
            "provider-provenance.json",
            lambda provenance: provenance["go_build_recipe"].__setitem__(
                "module", "evil.invalid/module@v1"
            ),
        )
        with self.assertRaisesRegex(ValueError, "Go oracle build receipt mismatch"):
            verify.verify_provenance()

    def test_capture_source_inventory_cannot_be_removed_after_rehash(self):
        self.mutate_json(
            "provider-provenance.json",
            lambda provenance: provenance.__setitem__("capture_sources_sha256", {}),
        )
        with self.assertRaisesRegex(ValueError, "capture source inventory"):
            verify.verify_provenance()

    def test_per_node_operation_counts_cannot_be_redistributed(self):
        def change(report):
            operations = report["metrics"]["provider"]["operations"]
            for field in ("requested", "completed"):
                operations["n0:comet-http-status"][field] = 10
                operations["n1:comet-http-status"][field] = 12

        self.mutate_json("report.json", change)
        with self.assertRaisesRegex(ValueError, "exactly 11 times"):
            verify.verify_report()

    def test_transaction_cutoff_cannot_be_hidden_after_rehash(self):
        def change(report):
            report["metrics"]["bank"]["included"] = 330
        self.mutate_json("report.json", change)
        with self.assertRaisesRegex(ValueError, "cutoff counts changed"):
            verify.verify_report()

    def test_bank_errors_cannot_be_hidden_after_rehash(self):
        self.mutate_json(
            "report.json", lambda report: report["metrics"]["bank"].__setitem__("errors", 99)
        )
        with self.assertRaisesRegex(ValueError, "rejections, errors"):
            verify.verify_report()

    def test_staking_rejections_cannot_be_removed_after_rehash(self):
        self.mutate_json(
            "report.json", lambda report: report["metrics"]["staking"].__setitem__("rejected", {})
        )
        with self.assertRaisesRegex(ValueError, "rejections, errors"):
            verify.verify_report()

    def test_status_protocol_cannot_disagree_with_app_version(self):
        def change(observation):
            response = observation["responses"]["status"]
            body = json.loads(response["body"])
            body["result"]["node_info"]["protocol_version"]["app"] = "9"
            response["body"] = json.dumps(body, separators=(",", ":"))
            response["sha256"] = verify.hashlib.sha256(response["body"].encode()).hexdigest()

        self.mutate_observation(change)
        with self.assertRaisesRegex(ValueError, "wrong application protocol"):
            verify.verify_provenance()

    def test_observation_timestamp_must_be_timezone_aware(self):
        self.mutate_observation(
            lambda observation: observation.__setitem__("observed_at", "2026-10-31T02:39:29")
        )
        with self.assertRaisesRegex(ValueError, "timezone-aware"):
            verify.verify_provenance()

    def test_expected_go_websocket_noise_is_frozen(self):
        path = self.capture / "node-logs/n0.log"
        path.write_text(path.read_text().replace("close 1006", "close 1001", 1))
        self.update_checksum("node-logs/n0.log")
        with self.assertRaisesRegex(ValueError, "cutoff-noise count changed"):
            verify.verify_report()


if __name__ == "__main__":
    unittest.main()
