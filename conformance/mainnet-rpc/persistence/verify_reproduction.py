#!/usr/bin/env python3
"""Verify the saved failed restart campaign; never report it as a parity pass."""
import base64
import hashlib
import importlib.util
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DATA = HERE / "restart-reproduction"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load(path):
    return json.loads(path.read_text())


def verify():
    manifest = load(DATA / "manifest.json")
    require(manifest["outcome"] == "incomplete-restart-campaign", "failed run was relabelled")
    actual = {str(p.relative_to(DATA)) for p in DATA.rglob("*") if p.is_file() and p.name != "manifest.json"}
    require(set(manifest["artifacts"]) == actual, "incomplete reproduction inventory")
    for name, expected in manifest["artifacts"].items():
        require(hashlib.sha256((DATA / name).read_bytes()).hexdigest() == expected, f"changed reproduction: {name}")
    receipts = load(DATA / "build-receipts.json")
    for path, archived in (
        ("conformance/devnet/rpc-persistence.sh", "capture-source.sh"),
        ("conformance/devnet/lib-mixed.sh", "mixed-source.sh"),
        ("conformance/mainnet-rpc/persistence/capture.py", "capture-source.py"),
    ):
        require(hashlib.sha256((DATA / archived).read_bytes()).hexdigest() == receipts["sources"][path], "capture source differs from receipt")
    capture = load(DATA / "retain-kv/before-restart.json")
    require(not capture["errors"] and len(capture["cases"]) == 40, "invalid pre-restart capture")
    for row in capture["cases"]:
        require(row["outcome"] == "match", "pre-restart mismatch")
        for role in ("go", "rs"):
            response = row[role]
            require(hashlib.sha256(response["body"].encode()).hexdigest() == response["body_sha256"], "unbound paired body")
        require(row["go"]["status"] == row["rs"]["status"] and
                row["go"]["content_type"] == row["rs"]["content_type"] and
                json.loads(row["go"]["body"]) == json.loads(row["rs"]["body"]), "unequal pre-restart responses")
    diagnostic = load(DATA / "retain-kv/restart-diagnostic.json")
    statuses = {}
    for name, node in diagnostic["nodes"].items():
        for response in node.values():
            require("body" in response and hashlib.sha256(response["body"].encode()).hexdigest() == response["sha256"], "missing diagnostic response")
        statuses[name] = json.loads(node["status"]["body"])["result"]["sync_info"]
    go, rs = statuses["go-full"], statuses["rs-full"]
    require(int(statuses["validator"]["latest_block_height"]) > int(go["latest_block_height"]), "validator did not advance")
    require(go["latest_block_height"] == rs["latest_block_height"] == "16", "follower stall changed")
    require(go["latest_app_hash"] == rs["latest_app_hash"], "followers have different app roots")
    for name in ("go-full", "rs-full"):
        state = json.loads(diagnostic["nodes"][name]["consensus_state"]["body"])["result"]["round_state"]
        require(state["height/round/step"] == "17/0/8" and not state["proposal_block_hash"], "unexpected stalled consensus state")
    transactions = set()
    blocks = load(DATA / "retain-kv/reference-blocks.json")
    require(len(blocks) == 16, "missing retained inputs")
    for height, response in enumerate(blocks, 1):
        require(hashlib.sha256(response["body"].encode()).hexdigest() == response["sha256"], "unbound retained block")
        block = json.loads(response["body"])["result"]["block"]
        require(int(block["header"]["height"]) == height, "retained block order changed")
        transactions.update(hashlib.sha256(base64.b64decode(tx)).hexdigest().upper() for tx in block["data"]["txs"] or [])
    for amount in (101, 102, 103):
        broadcast = load(DATA / f"retain-kv/tx-{amount}.json")
        inclusion = load(DATA / f"retain-kv/inclusion-{amount}.json")
        require(broadcast["txhash"] in transactions and inclusion["hash"] == broadcast["txhash"] and inclusion["result"]["code"] == 0, "missing successful workload input")
    parser = ROOT / "conformance/mainnet-rpc/populated-governance/verify.py"
    require(hashlib.sha256(parser.read_bytes()).hexdigest() == manifest["archive_parser_sha256"], "archive parser changed")
    spec = importlib.util.spec_from_file_location("archive_parser", parser)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    frames = module.archive_frames(DATA / "validator-blocks.bin.gz")
    require(len(frames) == 176, "incomplete validator archive")
    headers = []
    for index in range(0, len(frames), 2):
        header = module.block_header(frames[index])
        require(header["height"] == index // 2 + 1 and module.commit_height(frames[index + 1]) == header["height"], "archive height changed")
        require(header["chain_id"] == capture["chain_id"], "archive chain differs")
        headers.append(header)
    # Comet status exposes the latest block header's root (state at H-1).
    require(headers[15]["app_hash"].upper() == go["latest_app_hash"].upper(), "stalled header root is not on validator chain")
    handshake = re.search(r"ABCI Handshake App Info height=16 hash=([0-9A-F]+)", (DATA / "node-1-tail.log").read_text())
    require(handshake and headers[16]["app_hash"].upper() == handshake[1], "retained application root is not on validator chain")
    print("Verified incomplete campaign: 40 matched pre-restart pairs; validator advanced to 78 while Go/Rust followers remained at 16. Full restart/pruning matrix is NOT established.")


if __name__ == "__main__":
    verify()
