"""The frozen REST POST/body evidence must verify without network access."""

from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest


MODULE = (
    Path(__file__).resolve().parents[1]
    / "rest-post-boundaries"
    / "verify.py"
)
SPEC = importlib.util.spec_from_file_location("rest_post_boundary_verifier", MODULE)
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class RestPostBoundaryEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.provenance = json.loads(
            (verifier.HERE / "provenance.json").read_text()
        )
        cls.capture = json.loads(
            (verifier.HERE / "rest-post-boundaries.json").read_text()
        )
        cls.observation = json.loads(
            (verifier.HERE / "mainnet-observation.json").read_text()
        )

    def rebind_mainnet_response(self, observation, name, mutate):
        response = observation["responses"][name]
        decoded = json.loads(response["body"])
        mutate(decoded["result"])
        response["body"] = json.dumps(decoded, separators=(",", ":"))
        response["sha256"] = hashlib.sha256(response["body"].encode()).hexdigest()

    def test_frozen_fixture_and_provenance_verify(self):
        verifier.main()

    def test_raw_response_mutation_is_rejected(self):
        capture = deepcopy(self.capture)
        capture["cases"][0]["response"]["raw_sha256"] = "0" * 64
        with self.assertRaisesRegex(SystemExit, "response raw hash/size mismatch"):
            verifier.verify_capture(capture, self.provenance)

    def test_dynamic_header_cannot_be_admitted_as_stable(self):
        capture = deepcopy(self.capture)
        capture["cases"][0]["response"]["stable_headers"].append(
            ["Date", "not stable"]
        )
        with self.assertRaisesRegex(SystemExit, "stable response header projection"):
            verifier.verify_capture(capture, self.provenance)

    def test_rebound_mainnet_protocol_app_mismatch_is_rejected(self):
        observation = deepcopy(self.observation)
        self.rebind_mainnet_response(
            observation,
            "status",
            lambda result: result["node_info"]["protocol_version"].update(app="11"),
        )
        with self.assertRaisesRegex(SystemExit, "protocol app versions disagree"):
            verifier.verify_mainnet_observation(observation)

    def test_rebound_mainnet_height_mismatch_is_rejected(self):
        observation = deepcopy(self.observation)
        self.rebind_mainnet_response(
            observation,
            "status",
            lambda result: result["sync_info"].update(latest_block_height="15447158"),
        )
        with self.assertRaisesRegex(SystemExit, "status and ABCI heights disagree"):
            verifier.verify_mainnet_observation(observation)

    def test_unexpected_case_rename_is_rejected_by_complete_inventory(self):
        capture = deepcopy(self.capture)
        capture["cases"][-1]["name"] = "unexpected-renamed-case"
        with self.assertRaisesRegex(SystemExit, "complete REST case inventory"):
            verifier.verify_capture(capture, self.provenance)


if __name__ == "__main__":
    unittest.main()
