"""Integrity and four-transport checks for the current populated slice."""
import base64
from datetime import datetime, timedelta, timezone
from decimal import Decimal
import gzip
import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "conformance/fixtures/devnet/v105-populated-staking"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_varint(data, offset):
    value = shift = 0
    while offset < len(data) and shift < 70:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if byte < 0x80:
            return value, offset
        shift += 7
    raise ValueError("invalid protobuf varint")


def proto_fields(data):
    fields = []
    offset = 0
    while offset < len(data):
        tag, offset = read_varint(data, offset)
        field, wire = tag >> 3, tag & 7
        if field == 0:
            raise ValueError("protobuf field zero")
        if wire == 0:
            value, offset = read_varint(data, offset)
        elif wire == 2:
            size, offset = read_varint(data, offset)
            if size > len(data) - offset:
                raise ValueError("truncated protobuf field")
            value = data[offset:offset + size]
            offset += size
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        fields.append((field, wire, value))
    return fields


def exact_message(data, allowed):
    result = {field: [] for field in allowed}
    for field, wire, value in proto_fields(data):
        if field not in allowed:
            raise ValueError(f"unexpected protobuf field {field}")
        if wire != allowed[field]:
            raise ValueError(f"field {field} has wire type {wire}")
        result[field].append(value)
    return result


def one(values, label):
    if len(values) != 1:
        raise ValueError(f"{label} occurs {len(values)} times")
    return values[0]


def dec_coin(data):
    fields = exact_message(data, {1: 2, 2: 2})
    return {
        "denom": one(fields[1], "DecCoin.denom").decode(),
        # The protobuf scalar is the fixed-precision integer coefficient;
        # grpc-gateway emits its decimal value with 18 fractional places.
        "amount": Decimal(one(fields[2], "DecCoin.amount").decode()).scaleb(-18),
    }


def delegation_total_rewards(data):
    fields = exact_message(data, {1: 2, 2: 2})
    rewards = []
    for encoded in fields[1]:
        reward = exact_message(encoded, {1: 2, 2: 2})
        rewards.append({
            "validator_address": one(
                reward[1], "DelegationDelegatorReward.validator_address"
            ).decode(),
            "reward": [dec_coin(coin) for coin in reward[2]],
        })
    # The response is produced from a Go map. Only this repeated field is
    # unordered; coin order and the total field retain their wire order.
    rewards.sort(key=lambda item: item["validator_address"])
    return {"rewards": rewards, "total": [dec_coin(coin) for coin in fields[2]]}


def rest_delegation_total_rewards(body):
    if set(body) != {"rewards", "total"}:
        raise ValueError(f"unexpected REST fields: {set(body)}")
    rewards = []
    for reward in body["rewards"]:
        if set(reward) != {"validator_address", "reward"}:
            raise ValueError(f"unexpected REST reward fields: {set(reward)}")
        coins = []
        for coin in reward["reward"]:
            if set(coin) != {"denom", "amount"}:
                raise ValueError(f"unexpected REST coin fields: {set(coin)}")
            coins.append({"denom": coin["denom"], "amount": Decimal(coin["amount"])})
        rewards.append({"validator_address": reward["validator_address"], "reward": coins})
    rewards.sort(key=lambda item: item["validator_address"])
    total = []
    for coin in body["total"]:
        if set(coin) != {"denom", "amount"}:
            raise ValueError(f"unexpected REST total fields: {set(coin)}")
        total.append({"denom": coin["denom"], "amount": Decimal(coin["amount"])})
    return {"rewards": rewards, "total": total}


def archive_frames(path):
    data = gzip.decompress(path.read_bytes())
    frames = []
    offset = 0
    while offset < len(data):
        size, offset = read_varint(data, offset)
        if size == 0 or size > len(data) - offset:
            raise ValueError("invalid block archive frame")
        frames.append(data[offset:offset + size])
        offset += size
    return frames


def block_header(data):
    block = exact_message(data, {1: 2, 2: 2, 3: 2, 4: 2})
    header = exact_message(one(block[1], "Block.header"), {
        1: 2, 2: 2, 3: 0, 4: 2, 5: 2, 6: 2, 7: 2,
        8: 2, 9: 2, 10: 2, 11: 2, 12: 2, 13: 2, 14: 2,
    })
    return {
        "chain_id": one(header[2], "Header.chain_id").decode(),
        "height": one(header[3], "Header.height"),
        "app_hash": one(header[11], "Header.app_hash").hex() if header[11] else "",
    }


def commit_height(data):
    commit = exact_message(data, {1: 0, 2: 0, 3: 2, 4: 2})
    return one(commit[1], "Commit.height")


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
        expected_generators = {
            "abci-recorder": "conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go",
            "populated-queries": "conformance/rpccorpus/recorder/cmd/populated-queries/main.go",
            "rpc-recorder": "conformance/rpccorpus/recorder/main.go",
            "devnet-oracle": "conformance/devnet-oracle/main.go",
        }
        self.assertEqual(set(self.metadata["generator_binaries"]), set(expected_generators))
        for name, source in expected_generators.items():
            generator = self.metadata["generator_binaries"][name]
            self.assertRegex(generator["sha256"], r"^[0-9a-f]{64}$")
            self.assertIn(source, generator["sources"])
            for path in generator["sources"]:
                self.assertEqual(digest(ROOT / path), self.metadata["source_sha256"][path])
            build_info = FIXTURE / generator["build_info"]
            self.assertEqual(digest(build_info), generator["build_info_sha256"])
            contents = build_info.read_text()
            self.assertIn("go1.26.9", contents)
            self.assertIn(generator["module"], contents)

    def test_observation_freshness_raw_hashes_and_agreement(self):
        path = FIXTURE / "capture/mainnet-observation.json"
        observation = json.loads(path.read_text())
        parsed = {}
        for name in ("status", "abci_info"):
            raw = observation["responses"][name]["body"].encode()
            self.assertEqual(hashlib.sha256(raw).hexdigest(),
                             observation["responses"][name]["sha256"])
            parsed[name] = json.loads(raw)
        status = parsed["status"]["result"]
        info = parsed["abci_info"]["result"]["response"]
        self.assertEqual(status["node_info"]["network"], observation["chain_id"])
        self.assertEqual(info["version"], observation["version"])
        self.assertEqual(status["node_info"]["protocol_version"]["app"],
                         info["app_version"])
        self.assertEqual(status["sync_info"]["latest_block_height"],
                         info["last_block_height"])
        observed = datetime.fromisoformat(observation["observed_at"])
        captured = datetime.fromisoformat(self.metadata["captured_at"])
        now = datetime.now(timezone.utc)
        self.assertIsNotNone(observed.utcoffset())
        self.assertIsNotNone(captured.utcoffset())
        self.assertLessEqual(observed, captured)
        self.assertLessEqual(captured - observed, timedelta(hours=24))
        self.assertLessEqual(captured, now + timedelta(minutes=5))

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
                abci = bytes.fromhex(transports["abci"].get("response_hex", ""))
                comet_value = base64.b64decode(comet.get("value") or "")
                grpc = bytes.fromhex(expected)
                if name == "delegation-rewards":
                    canonical = delegation_total_rewards(grpc)
                    self.assertEqual(delegation_total_rewards(abci), canonical)
                    self.assertEqual(delegation_total_rewards(comet_value), canonical)
                    self.assertEqual(
                        rest_delegation_total_rewards(transports["rest"]["body"]),
                        canonical,
                    )
                else:
                    self.assertEqual(abci, grpc)
                    self.assertEqual(comet_value, grpc)
                self.assertEqual(comet["height"], str(case["height"]))
                grpc_headers = transports["cosmos_grpc"].get("headers", {})
                self.assertEqual(grpc_headers.get("x-cosmos-block-height"), [str(case["height"])])
                rest = transports["rest"]
                self.assertEqual(rest["status"], 200)
                self.assertEqual(
                    rest["headers"].get("Grpc-Metadata-X-Cosmos-Block-Height"),
                    [str(case["height"])],
                )

    def test_block_archive_and_state_snapshots_are_structurally_verified(self):
        frames = archive_frames(FIXTURE / "blocks.bin.gz")
        self.assertEqual(len(frames), 80)
        headers = [block_header(frames[index]) for index in range(0, len(frames), 2)]
        self.assertEqual([header["height"] for header in headers], list(range(1, 41)))
        self.assertEqual([commit_height(frames[index]) for index in range(1, len(frames), 2)],
                         list(range(1, 41)))
        self.assertTrue(all(header["chain_id"] == self.capture["chain_id"]
                            for header in headers))
        self.assertEqual(headers[-1]["app_hash"].upper(), self.capture["app_hash"])
        snapshots = {}
        for height in (1, 40):
            state = json.loads((FIXTURE / f"state/{height}.json").read_text())
            self.assertEqual(state["height"], height)
            self.assertRegex(state["app_hash"], r"^[0-9a-f]{64}$")
            names = [store["name"] for store in state["stores"]]
            self.assertEqual(names, sorted(names))
            self.assertEqual(len(names), len(set(names)))
            for store in state["stores"]:
                self.assertRegex(store["hash"], r"^[0-9a-f]{64}$")
                kv = store.get("kv", [])
                self.assertEqual(kv, sorted(kv, key=lambda item: item[0]))
                self.assertEqual(len({item[0] for item in kv}), len(kv))
                for key, value in kv:
                    bytes.fromhex(key)
                    bytes.fromhex(value)
            snapshots[height] = state
        self.assertEqual(snapshots[1]["app_hash"], headers[1]["app_hash"])
        info = self.capture["state"]["abci_info"]["result"]["response"]
        self.assertEqual(info["last_block_height"], "40")
        self.assertEqual(snapshots[40]["app_hash"],
                         base64.b64decode(info["last_block_app_hash"]).hex())

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
