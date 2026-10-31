"""Integrity and four-transport checks for the current populated slice."""
import base64
import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "conformance/fixtures/devnet/v105-populated-staking"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PopulatedStakingEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.capture = json.loads((FIXTURE / "capture/transports.json").read_text())
        cls.metadata = json.loads((FIXTURE / "capture/metadata.json").read_text())
        cls.cases = {case["name"]: case for case in cls.capture["cases"]}

    def test_sources_artifacts_and_oracles_are_bound(self):
        for name, expected in self.metadata["source_sha256"].items():
            self.assertEqual(digest(ROOT / name), expected, name)
        for name, expected in self.metadata["artifact_sha256"].items():
            self.assertEqual(digest(FIXTURE / name), expected, name)
        app = json.loads((FIXTURE / "capture/app-build.json").read_text())
        core = json.loads((FIXTURE / "capture/core-build.json").read_text())
        self.assertEqual(app["module"], "github.com/celestiaorg/celestia-app/v10@v10.5.0")
        self.assertEqual(app["source_commit"], "6013e758d5dceb75827ad191c8d739d7c43cfa04")
        self.assertEqual(app["build_tags"], ["ledger"])
        self.assertEqual(core["module"], "github.com/celestiaorg/celestia-core@v0.42.3")
        self.assertEqual(core["source_commit"], "a5537d84a008d566abc503d2ea41e30f2301403d")
        self.assertEqual(app["compiler"], "go version go1.26.9 linux/amd64")
        observation = json.loads((FIXTURE / "capture/mainnet-observation.json").read_text())
        self.assertEqual((observation["chain_id"], observation["version"]), ("celestia", "10.5.0"))

    def test_committing_root_and_inventory(self):
        self.assertEqual(self.capture["chain_id"], "rs-devnet-1")
        self.assertEqual(self.capture["tip_height"], 40)
        self.assertEqual(self.capture["query_height"], 39)
        block = self.capture["state"]["committing_block"]["result"]["block"]["header"]
        self.assertEqual(block["height"], "40")
        self.assertEqual(block["chain_id"], self.capture["chain_id"])
        self.assertEqual(block["app_hash"], self.capture["app_hash"])
        self.assertEqual(self.metadata["state_identity"]["app_hash"], self.capture["app_hash"])
        self.assertEqual(len(self.cases), 11)
        self.assertEqual(
            self.capture["normalizations"],
            [{"case": "delegation-rewards", "policy": "validator-address-order",
              "reason": "Go iterates the validator rewards map independently on each transport"}],
        )
        self.assertEqual(
            set(self.cases),
            {"validators-first", "validators-next-key", "validators-offset",
             "validators-reverse", "validators-empty-page", "validators-invalid-cursor",
             "validators-historical", "validator", "delegations-first",
             "delegations-historical", "delegation-rewards"},
        )

    def test_abci_comet_and_grpc_return_identical_protobuf(self):
        for name, case in self.cases.items():
            with self.subTest(name=name):
                transports = case["transports"]
                self.assertEqual(set(transports), {"abci", "comet_rpc", "cosmos_grpc", "rest"})
                expected = transports["cosmos_grpc"].get("response_hex", "")
                comet = transports["comet_rpc"]["body"]["result"]["response"]
                if name != "delegation-rewards":
                    self.assertEqual(transports["abci"].get("response_hex", ""), expected)
                    self.assertEqual(base64.b64decode(comet.get("value") or "").hex(), expected)
                else:
                    self.assertTrue(transports["abci"].get("response_hex"))
                    self.assertTrue(expected)
                self.assertEqual(comet["height"], str(case["height"]))
                grpc_headers = transports["cosmos_grpc"].get("headers", {})
                self.assertEqual(grpc_headers.get("x-cosmos-block-height"), [str(case["height"])])
                rest = transports["rest"]
                self.assertEqual(rest["status"], 200)
                self.assertEqual(
                    rest["headers"].get("Grpc-Metadata-X-Cosmos-Block-Height"),
                    [str(case["height"])],
                )

    def test_populated_pagination_and_historical_controls(self):
        body = lambda name: self.cases[name]["transports"]["rest"]["body"]
        first = body("validators-first")
        next_page = body("validators-next-key")
        offset = body("validators-offset")
        reverse = body("validators-reverse")
        self.assertEqual(len(first["validators"]), 1)
        self.assertEqual(first["pagination"]["total"], "2")
        self.assertTrue(first["pagination"]["next_key"])
        self.assertEqual(next_page["validators"], offset["validators"])
        self.assertNotEqual(first["validators"], next_page["validators"])
        self.assertEqual(reverse["validators"], next_page["validators"])
        for name in ("validators-empty-page", "validators-invalid-cursor"):
            self.assertEqual(body(name)["validators"], [], name)
        self.assertEqual(len(body("validators-historical")["validators"]), 1)
        self.assertEqual(len(body("delegations-first")["delegation_responses"]), 1)
        self.assertEqual(len(body("delegations-historical")["delegation_responses"]), 1)
        self.assertTrue(body("delegation-rewards")["total"])


if __name__ == "__main__":
    unittest.main()
