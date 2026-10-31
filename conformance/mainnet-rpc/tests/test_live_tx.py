"""Offline integrity and behavior checks for the v10.5 live-tx evidence."""

import base64
from datetime import datetime, timedelta, timezone
import gzip
import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "conformance/fixtures/devnet/v105-live-tx"
CAPTURE = FIXTURE / "capture"


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


def values(data, field, wire=None):
    found = []
    for number, actual_wire, value in proto_fields(data):
        if number == field:
            if wire is not None and actual_wire != wire:
                raise ValueError(f"field {field} has wire type {actual_wire}")
            found.append(value)
    return found


def one(items, label):
    if len(items) != 1:
        raise ValueError(f"{label} occurs {len(items)} times")
    return items[0]


def scalar(data, field, default=0):
    found = values(data, field, 0)
    if not found:
        return default
    return one(found, f"field {field}")


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
    header = one(values(data, 1, 2), "Block.header")
    return {
        "chain_id": one(values(header, 2, 2), "Header.chain_id").decode(),
        "height": one(values(header, 3, 0), "Header.height"),
        "app_hash": (values(header, 11, 2) or [b""])[0].hex(),
    }


def commit_height(data):
    return one(values(data, 1, 0), "Commit.height")


def response_result(exchange):
    body = exchange["body"]
    return body.get("result") or body.get("tx_response")


class LiveTransactionEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.capture = json.loads((CAPTURE / "transactions.json").read_text())
        cls.metadata = json.loads((CAPTURE / "metadata.json").read_text())
        cls.manifest = json.loads((CAPTURE / "inputs/manifest.json").read_text())
        cls.cases = {case["name"]: case for case in cls.capture["cases"]}

    def test_sources_artifacts_builds_and_inputs_are_bound(self):
        for name, expected in self.metadata["source_sha256"].items():
            self.assertEqual(digest(ROOT / name), expected, name)
        for name, expected in self.metadata["artifact_sha256"].items():
            self.assertEqual(digest(FIXTURE / name), expected, name)

        app = json.loads((CAPTURE / "app-build.json").read_text())
        core = json.loads((CAPTURE / "core-build.json").read_text())
        self.assertEqual(app["module"], "github.com/celestiaorg/celestia-app/v10@v10.5.0")
        self.assertEqual(app["source_commit"], "6013e758d5dceb75827ad191c8d739d7c43cfa04")
        self.assertEqual(app["build_tags"], ["ledger"])
        self.assertEqual(core["module"], "github.com/celestiaorg/celestia-core@v0.42.3")
        self.assertEqual(core["source_commit"], "a5537d84a008d566abc503d2ea41e30f2301403d")
        self.assertEqual(app["compiler"], "go version go1.26.9 linux/amd64")
        self.assertEqual(core["compiler"], app["compiler"])
        self.assertRegex(app["binary_sha256"], r"^[0-9a-f]{64}$")
        self.assertRegex(core["binary_sha256"], r"^[0-9a-f]{64}$")

        expected_generators = {
            "abci-recorder": "conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go",
            "live-tx": "conformance/rpccorpus/recorder/cmd/live-tx/main.go",
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
            receipt = FIXTURE / generator["build_info"]
            self.assertEqual(digest(receipt), generator["build_info_sha256"])
            self.assertIn("go1.26.9", receipt.read_text())

        for name, expected in self.manifest["transactions"].items():
            path = CAPTURE / "inputs" / name
            self.assertEqual(digest(path), expected, name)
            self.assertEqual(self.metadata["artifact_sha256"][f"capture/inputs/{name}"], expected)

    def test_mainnet_observation_is_fresh_ordered_and_self_consistent(self):
        observation = json.loads((CAPTURE / "mainnet-observation.json").read_text())
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
        self.assertEqual((observation["chain_id"], observation["version"]),
                         ("celestia", "10.5.0"))

        observed = datetime.fromisoformat(observation["observed_at"])
        started = datetime.fromisoformat(self.capture["started_at"])
        completed = datetime.fromisoformat(self.capture["completed_at"])
        captured = datetime.fromisoformat(self.metadata["captured_at"])
        now = datetime.now(timezone.utc)
        self.assertLessEqual(observed, started)
        self.assertLessEqual(started, completed)
        self.assertLessEqual(completed, captured)
        self.assertLessEqual(captured - observed, timedelta(hours=24))
        self.assertLessEqual(captured, now + timedelta(minutes=5))

    def test_case_inventory_transports_modes_and_exact_requests(self):
        expected = {
            "malformed-rest-encoding": ("cosmos-rest", "sync"),
            "no-message": ("cosmos-grpc", "sync"),
            "bad-signature": ("cosmos-rest", "sync"),
            "low-gas": ("comet-jsonrpc", "sync"),
            "future-sequence": ("core-grpc", "commit"),
            "duplicate": ("comet-jsonrpc", "sync"),
            "rest-async": ("cosmos-rest", "async"),
            "cosmos-sync": ("cosmos-grpc", "sync"),
            "cosmos-async": ("cosmos-grpc", "async"),
            "core-grpc": ("core-grpc", "commit"),
            "comet-async": ("comet-jsonrpc", "async"),
            "comet-commit": ("comet-jsonrpc", "commit"),
            "cancel": ("comet-jsonrpc", "commit-client-timeout-100ms"),
            "cancel-retry": ("comet-jsonrpc", "retry-after-inclusion"),
            "cleanup": ("comet-jsonrpc", "commit"),
            "execution-failure": ("cosmos-grpc", "block-unsupported"),
            "execution-failure-included": ("cosmos-rest", "sync"),
        }
        self.assertEqual(set(self.cases), set(expected))
        for name, (transport, mode) in expected.items():
            case = self.cases[name]
            self.assertEqual((case["transport"], case["mode"]), (transport, mode))
            if name == "malformed-rest-encoding":
                self.assertEqual(json.loads(case["http"]["request_body"])["tx_bytes"], "%%%")
                continue
            tx_name = case["tx_file"]
            tx = (CAPTURE / "inputs" / tx_name).read_bytes()
            self.assertEqual(case["tx_sha256"], hashlib.sha256(tx).hexdigest())
            self.assertEqual(case["tx_hash"], hashlib.sha256(tx).hexdigest().upper())
            if "grpc" in case:
                request = bytes.fromhex(case["grpc"]["request_hex"])
                self.assertEqual(one(values(request, 1, 2), "request.tx"), tx)
            elif "http" in case:
                request = json.loads(case["http"]["request_body"])
                encoded = request.get("tx_bytes") or request["params"]["tx"]
                self.assertEqual(base64.b64decode(encoded, validate=True), tx)

    def test_rejections_duplicates_and_protobuf_codes(self):
        malformed = self.cases["malformed-rest-encoding"]["http"]
        self.assertEqual((malformed["status"], malformed["body"]["code"]), (400, 3))
        self.assertIn("illegal base64", malformed["body"]["message"])

        no_message = self.cases["no-message"]["grpc"]
        self.assertEqual(no_message["code"], 0)
        tx_response = one(values(bytes.fromhex(no_message["response_hex"]), 1, 2),
                          "BroadcastTxResponse.tx_response")
        self.assertEqual(scalar(tx_response, 4), 18)
        self.assertIn(b"must contain at least one message", one(values(tx_response, 6, 2), "raw_log"))

        bad_signature = response_result(self.cases["bad-signature"]["http"])
        self.assertEqual(bad_signature["code"], 4)
        self.assertIn("signature verification failed", bad_signature["raw_log"])
        low_gas = response_result(self.cases["low-gas"]["http"])
        self.assertEqual(low_gas["code"], 11)
        self.assertIn("out of gas", low_gas["log"])

        future = self.cases["future-sequence"]["grpc"]
        self.assertEqual(future["code"], 0)
        check_tx = one(values(bytes.fromhex(future["response_hex"]), 1, 2), "core.check_tx")
        self.assertEqual(scalar(check_tx, 1), 32)
        self.assertIn(b"expected 0, got 50", one(values(check_tx, 3, 2), "check_tx.log"))

        duplicate = self.cases["duplicate"]
        self.assertEqual(response_result(duplicate["http"])["code"], 0)
        second = duplicate["duplicate_http"]["body"]["error"]
        self.assertEqual(second["code"], -32603)
        self.assertEqual(second["message"], "Internal error")
        retry = response_result(self.cases["cancel-retry"]["http"])
        self.assertEqual(retry["code"], 32)
        self.assertIn("expected 8, got 7", retry["log"])

    def test_success_inclusion_heights_gas_events_and_cleanup(self):
        inclusions = [
            ("duplicate", 0),
            ("rest-async", 0),
            ("cosmos-sync", 0),
            ("cosmos-async", 0),
            ("core-grpc", 0),
            ("comet-async", 0),
            ("comet-commit", 0),
            ("cancel", 0),
            ("cleanup", 0),
            ("execution-failure-included", 5),
        ]
        previous = self.capture["initial_height"]
        observed_heights = []
        for name, code in inclusions:
            case = self.cases[name]
            result = case["inclusion"]["body"]["result"]
            tx_result = result["tx_result"]
            height = int(result["height"])
            observed_heights.append(height)
            self.assertEqual(tx_result["code"], code, name)
            self.assertEqual(result["hash"], case["tx_hash"], name)
            self.assertGreater(int(tx_result["gas_wanted"]), 0, name)
            self.assertGreater(int(tx_result["gas_used"]), 0, name)
            self.assertTrue(tx_result["events"], name)
            self.assertEqual(case["before"]["height"], previous, name)
            self.assertEqual(case["after"]["height"], height, name)
            self.assertEqual(case["after"]["num_unconfirmed_txs"]["body"]["result"]["n_txs"], "0")
            previous = height
        self.assertGreater(observed_heights[0], self.capture["initial_height"])
        self.assertEqual(observed_heights[1:],
                         list(range(observed_heights[0] + 1, observed_heights[-1] + 1)))
        self.assertEqual(self.capture["final_height"], observed_heights[-1])

        cancel = self.cases["cancel"]
        self.assertIn("context deadline exceeded", cancel["http"]["error"])
        cancel_height = cancel["inclusion"]["body"]["result"]["height"]
        existing = self.cases["cancel-retry"]["existing_tx"]["body"]["result"]
        self.assertEqual(existing["height"], cancel_height)
        self.assertEqual(self.cases["execution-failure"]["grpc"]["code"], 2)
        self.assertIn("supported types: sync, async",
                      self.cases["execution-failure"]["grpc"]["message"])
        admitted = response_result(self.cases["execution-failure-included"]["http"])
        self.assertEqual(admitted["code"], 0)

    def test_grpc_success_payloads_metadata_and_http_transport_status(self):
        for name in ("no-message", "cosmos-sync", "cosmos-async"):
            exchange = self.cases[name]["grpc"]
            self.assertEqual(exchange["code"], 0)
            self.assertEqual(exchange["headers"]["content-type"], ["application/grpc+proto"])
            self.assertEqual(exchange["headers"]["x-cosmos-block-height"],
                             [str(self.cases[name]["before"]["height"])])
        for name in ("cosmos-sync", "cosmos-async"):
            exchange = self.cases[name]["grpc"]
            response = one(values(bytes.fromhex(exchange["response_hex"]), 1, 2),
                           "BroadcastTxResponse.tx_response")
            self.assertEqual(scalar(response, 4), 0)
            self.assertEqual(one(values(response, 2, 2), "txhash").decode(),
                             self.cases[name]["tx_hash"])
        core = self.cases["core-grpc"]["grpc"]
        self.assertEqual(core["code"], 0)
        self.assertEqual(core["headers"]["content-type"], ["application/grpc+proto"])
        core_response = bytes.fromhex(core["response_hex"])
        self.assertEqual(scalar(one(values(core_response, 1, 2), "check_tx"), 1), 0)

        for case in self.cases.values():
            if "http" in case and not case["http"].get("error"):
                expected = 400 if case["name"] == "malformed-rest-encoding" else 200
                self.assertEqual(case["http"]["status"], expected, case["name"])
                self.assertTrue(case["http"]["headers"].get("Content-Type"), case["name"])

    def test_blob_submission_and_final_capacity(self):
        blob = json.loads((CAPTURE / "blob-submission.json").read_text())
        tx = (CAPTURE / blob["tx_file"]).read_bytes()
        tx_hash = hashlib.sha256(tx).hexdigest().upper()
        self.assertEqual(blob["tx_sha256"], tx_hash.lower())
        self.assertEqual(blob["cli_response"]["txhash"], tx_hash)
        inclusion = blob["inclusion"]["result"]
        blob_height = int(inclusion["height"])
        self.assertEqual(inclusion["tx_result"]["code"], 0)
        self.assertEqual(inclusion["hash"], tx_hash)
        self.assertTrue(inclusion["tx_result"]["events"])
        before = int(blob["before"]["status"]["result"]["sync_info"]["latest_block_height"])
        after = int(blob["after"]["status"]["result"]["sync_info"]["latest_block_height"])
        self.assertEqual(before, self.capture["final_height"])
        self.assertGreater(blob_height, before)
        self.assertEqual(after, blob_height)
        self.assertEqual(blob["after"]["unconfirmed"]["result"]["n_txs"], "0")

    def test_blocks_and_state_snapshots_are_structurally_verified(self):
        meta = json.loads((FIXTURE / "meta.json").read_text())
        self.assertEqual((meta["chain_id"], meta["celestia_app"], meta["celestia_core"]),
                         ("rs-devnet-1", "v10.5.0", "v0.42.3"))
        committed = meta["committed_heights"]
        frames = archive_frames(FIXTURE / "blocks.bin.gz")
        self.assertEqual(len(frames), 2 * committed)
        headers = [block_header(frames[index]) for index in range(0, len(frames), 2)]
        commits = [commit_height(frames[index]) for index in range(1, len(frames), 2)]
        self.assertEqual([header["height"] for header in headers], list(range(1, committed + 1)))
        self.assertEqual(commits, list(range(1, committed + 1)))
        self.assertTrue(all(header["chain_id"] == "rs-devnet-1" for header in headers))

        snapshots = {}
        for height in (1, committed):
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
                self.assertEqual(len(kv), len({item[0] for item in kv}))
                for key, value in kv:
                    bytes.fromhex(key)
                    bytes.fromhex(value)
            snapshots[height] = state
        self.assertEqual(snapshots[1]["app_hash"], headers[1]["app_hash"])
        final_info = json.loads((CAPTURE / "final-abci-info.json").read_text())
        response = final_info["result"]["response"]
        self.assertEqual(response["last_block_height"], str(committed))
        self.assertEqual(snapshots[committed]["app_hash"],
                         base64.b64decode(response["last_block_app_hash"]).hex())


if __name__ == "__main__":
    unittest.main()
