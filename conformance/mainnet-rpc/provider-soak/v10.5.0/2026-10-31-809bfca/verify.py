#!/usr/bin/env python3
"""Fail-closed integrity and acceptance checks for the bounded provider soak."""

from collections import Counter
from datetime import datetime
import hashlib
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = Path(__file__).resolve().parents[5]
COMMIT = "809bfca7b374bf7463604a851b227dc9452fceb2"
RS_SHA256 = "e3a6dcb04a3f8f89ec5179f615ad5c42a94c9f44c0dc5090868c39ecd27eac55"
GO_SHA256 = "eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38"
CLIENT_SHA256 = "0c7f41799e6568af8896a39523ae5bc22af1ec701d03df9ddf21afacd37471ed"
QUERY_OPERATIONS = (
    "comet-http-status",
    "comet-jsonrpc-history",
    "comet-jsonrpc-proof",
    "comet-ws-subscribe",
    "comet-ws-slow-reconnect",
    "core-grpc-status",
    "core-grpc-history",
    "core-grpc-slow-subscribe",
    "cosmos-grpc-pagination-history",
    "cosmos-grpc-block",
    "rest-pagination-history",
    "rest-node-info",
)
BROADCAST_TRANSPORTS = (
    "jsonrpc",
    "uri",
    "websocket",
    "cosmos-grpc",
    "rest",
    "core-grpc",
)
LIMITATIONS = [
    "TLS and reverse proxies",
    "external provider client SDKs",
    "proof cryptographic validity",
    "multi-page cursor traversal",
    "all supported message families through every transport",
    "subscription quota saturation",
    "unbounded slow readers",
    "exact Go/Rust response equivalence at matched state",
]
PUBLIC_ARTIFACTS = {
    "disk.csv", "events.log", "hashes.jsonl", "mainnet-observation.json",
    "mem.jsonl", "provider-provenance.json", "report.json", "report.md",
    "rss.csv", *(f"node-logs/n{i}.log" for i in range(5)),
}
SUPPORT_FILES = {"README.md", "SHA256SUMS", "verify.py"}
CAPTURE_SOURCES = {
    "conformance/devnet/soak.sh",
    "conformance/devnet/soak/provider.py",
    "conformance/devnet/soak/soak.py",
    "conformance/provider-client/go.mod",
    "conformance/provider-client/go.sum",
    "conformance/provider-client/main.go",
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(name: str):
    return json.loads((HERE / name).read_text())


def verify_hashes() -> None:
    expected = {}
    for line in (HERE / "SHA256SUMS").read_text().splitlines():
        sha256, name = line.split("  ", 1)
        if name in expected or Path(name).is_absolute() or ".." in Path(name).parts:
            raise ValueError(f"invalid duplicate or non-local checksum path: {name}")
        expected[name] = sha256
    if set(expected) != PUBLIC_ARTIFACTS:
        raise ValueError("SHA256SUMS does not bind the exact public capture set")
    found = set()
    for path in HERE.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"public capture contains a symlink: {path.relative_to(HERE)}")
        if path.is_file():
            found.add(path.relative_to(HERE).as_posix())
    if found != PUBLIC_ARTIFACTS | SUPPORT_FILES:
        raise ValueError("capture directory does not contain the exact public file set")
    for name, sha256 in expected.items():
        if digest(HERE / name) != sha256:
            raise ValueError(f"capture hash mismatch: {name}")


def verify_observation(provenance: dict) -> None:
    observation = load("mainnet-observation.json")
    if provenance["mainnet_observation"] != observation:
        raise ValueError("provenance does not embed the published observation exactly")
    if (observation["chain_id"], observation["version"]) != ("celestia", "10.5.0"):
        raise ValueError("unexpected mainnet oracle identity")
    raw = {}
    for route in ("status", "abci_info"):
        response = observation["responses"][route]
        if hashlib.sha256(response["body"].encode()).hexdigest() != response["sha256"]:
            raise ValueError(f"raw mainnet {route} hash mismatch")
        raw[route] = json.loads(response["body"])["result"]
    status = raw["status"]
    info = raw["abci_info"]["response"]
    if status["node_info"]["network"] != "celestia" or status["sync_info"]["catching_up"]:
        raise ValueError("mainnet observation is not a synced celestia node")
    if status["node_info"]["protocol_version"]["app"] != "10":
        raise ValueError("mainnet status has the wrong application protocol")
    if info["version"] != "10.5.0" or info["app_version"] != "10":
        raise ValueError("mainnet observation has the wrong application version")
    if status["sync_info"]["latest_block_height"] != info["last_block_height"]:
        raise ValueError("mainnet status and ABCI heights differ")
    observed_at = datetime.fromisoformat(observation["observed_at"])
    if observed_at.utcoffset() is None:
        raise ValueError("mainnet observation timestamp is not timezone-aware")
    observed = observed_at.timestamp()
    captured = provenance["captured_at_unix"]
    if not 0 <= captured - observed <= 24 * 60 * 60:
        raise ValueError("mainnet observation did not precede the capture within 24 hours")


def verify_provenance() -> dict:
    provenance = load("provider-provenance.json")
    if provenance["source_commit"] != COMMIT:
        raise ValueError("capture source commit mismatch")
    binaries = provenance["binaries"]
    expected = {
        "celestia_rs": RS_SHA256,
        "celestia_app": GO_SHA256,
        "provider_client": CLIENT_SHA256,
    }
    if {name: value["sha256"] for name, value in binaries.items()} != expected:
        raise ValueError("binary identity mismatch")
    version = provenance["rs_version"]
    for text in (f"commit: {COMMIT}", "app_version: 10", "celestia-app v10.5.0", "celestia-core v0.42.3"):
        if text not in version:
            raise ValueError(f"Rust build identity omits {text!r}")
    recipe = provenance["go_build_recipe"]
    if (provenance["go_source_module"] != "github.com/celestiaorg/celestia-app/v10@v10.5.0"
            or recipe["module"] != "github.com/celestiaorg/celestia-app/v10@v10.5.0"
            or recipe["binary_sha256"] != GO_SHA256
            or recipe["source_commit"] != "6013e758d5dceb75827ad191c8d739d7c43cfa04"
            or "ledger" not in recipe["build_tags"]
            or recipe["compiler"] != "go version go1.26.9 linux/amd64"):
        raise ValueError("Go oracle build receipt mismatch")
    if provenance["coverage_limitations"] != LIMITATIONS:
        raise ValueError("coverage limitations changed or disappeared")
    sources = provenance["capture_sources_sha256"]
    if set(sources) != CAPTURE_SOURCES:
        raise ValueError("capture source inventory changed or disappeared")
    for name, sha256 in sources.items():
        path = (REPO / name).resolve()
        if REPO not in path.parents or digest(path) != sha256:
            raise ValueError(f"capture source mismatch: {name}")
    verify_observation(provenance)
    return provenance


def verify_blocks(report: dict) -> None:
    rows = [json.loads(line) for line in (HERE / "hashes.jsonl").read_text().splitlines()]
    heights = [row["h"] for row in rows]
    if heights != list(range(1, 114)):
        raise ValueError("block hash evidence is not contiguous through height 113")
    if (len(rows), sum(row["ntx"] for row in rows), max(row["size"] for row in rows)) != (113, 408, 2_045_362):
        raise ValueError("block count, transaction count, or maximum size changed")
    proposer_counts = Counter(row["p"] for row in rows)
    proposed_by_node = report["proposed_by_node"]
    if (proposed_by_node != {"0": 28, "1": 28, "2": 29, "3": 28}
            or len(proposer_counts) != 4
            or sorted(proposer_counts.values()) != sorted(proposed_by_node.values())):
        raise ValueError("reported proposer counts do not match the block evidence")
    if report["blocks"] != {"txs": 408, "max_bytes": 2_045_362, "over_8mb": 0, "count": 113}:
        raise ValueError("report block summary does not match the frozen run")


def verify_provider(report: dict) -> None:
    provider = report["metrics"]["provider"]
    if not provider["enabled"] or provider["disabled_scenarios"] != LIMITATIONS:
        raise ValueError("provider mode or its declared limitations changed")
    operations = provider["operations"]
    expected = {
        *(f"n{node}:{operation}" for node in range(5) for operation in QUERY_OPERATIONS),
        *(f"n{node}:broadcast:{transport}" for node in range(5) for transport in BROADCAST_TRANSPORTS),
    }
    if set(operations) != expected:
        raise ValueError("provider node/operation matrix is incomplete or duplicated")
    for name, row in operations.items():
        if row["requested"] != row["completed"] or not row["completed"]:
            raise ValueError(f"provider operation did not complete: {name}")
        if ":broadcast:" not in name and (row["requested"] != 11 or row["completed"] != 11):
            raise ValueError(f"provider operation did not complete exactly 11 times: {name}")
        if row["errors"] or row["admission_unknown"]:
            raise ValueError(f"provider operation recorded an error: {name}")
        if ":broadcast:" in name and row.get("accepted") != row["completed"]:
            raise ValueError(f"provider broadcast was not accepted: {name}")
    if sum(row["completed"] for row in operations.values()) != 990:
        raise ValueError("provider completion count changed")

    events = [json.loads(line) for line in (HERE / "events.log").read_text().splitlines()]
    event_counts = Counter(row["ev"] for row in events)
    if event_counts != {"start": 1, "provider-broadcast": 330, "provider-result": 660,
                        "gov-proposal": 2, "draining": 1}:
        raise ValueError("event inventory changed or contains a failure event")
    results = [row for row in events if row["ev"] == "provider-result"]
    broadcasts = [row for row in events if row["ev"] == "provider-broadcast"]
    if not all(row.get("ok") is True and not row.get("error") for row in results):
        raise ValueError("provider query/subscription result failed")
    if Counter(row["operation"] for row in results) != Counter({name: 55 for name in QUERY_OPERATIONS}):
        raise ValueError("provider query/subscription coverage changed")
    if Counter(row["transport"] for row in broadcasts) != Counter({name: 55 for name in BROADCAST_TRANSPORTS}):
        raise ValueError("provider broadcast coverage changed")
    node_kinds = {node["node"]: node["kind"] for node in report["nodes"]}
    expected_results = Counter({
        (node, node_kinds[node], operation): operations[f"n{node}:{operation}"]["completed"]
        for node in range(5) for operation in QUERY_OPERATIONS
    })
    actual_results = Counter((row.get("node"), row.get("node_kind"), row["operation"])
                             for row in results)
    if actual_results != expected_results:
        raise ValueError("provider result events do not match the per-node operation matrix")
    expected_broadcasts = Counter({
        (node, node_kinds[node], transport):
            operations[f"n{node}:broadcast:{transport}"]["completed"]
        for node in range(5) for transport in BROADCAST_TRANSPORTS
    })
    actual_broadcasts = Counter((row.get("node"), row.get("node_kind"), row["transport"])
                                for row in broadcasts)
    if actual_broadcasts != expected_broadcasts:
        raise ValueError("provider broadcast events do not match the per-node operation matrix")
    if not all(row["code"] == 0 and (row["transport"] != "core-grpc" or row.get("tx_result_code") == 0)
               for row in broadcasts):
        raise ValueError("provider broadcast admission or execution failed")


def verify_transactions(report: dict) -> None:
    metrics = report["metrics"]
    expected = {
        "bank": {"ok": 330, "rejected": {}, "errors": 0, "resyncs": 0,
                 "included": 322, "failed_exec": 0, "lost": 0},
        "pfb": {"submitted": 58, "rejected": {}, "included": 56,
                "failed_exec": 0, "lost": 0, "max_round_bytes": 2_086_434},
        "staking": {"submitted": 16,
                    "rejected": {"err:Error: no rewards to withdraw": 2, "32": 1},
                    "included": 15, "failed_exec": 0, "lost": 0},
        "gov": {"proposals": 2, "passed": 1, "rejected": 0, "other": 1, "votes": 8},
    }
    if {name: metrics[name] for name in expected} != expected:
        raise ValueError("transaction totals, rejections, errors, or cutoff counts changed")


def verify_report() -> None:
    report = load("report.json")
    if (report["final"] is not True or report["elapsed_s"] < 300 or report["heights"] < 80
            or report["failures"] or report["stalls"] or report["log_hits"]):
        raise ValueError("final duration/height/health gate failed")
    nodes = report["nodes"]
    if (len(nodes) != 5
            or [node["node"] for node in nodes] != list(range(5))
            or [node["kind"] for node in nodes] != ["go", "go", "rs", "rs", "rs"]
            or [node["validator"] for node in nodes] != [True, True, True, True, False]):
        raise ValueError("mixed-node topology changed")
    if min(node["checked_upto"] for node in report["nodes"]) < 110:
        raise ValueError("final consistency pass did not cover the completed run")
    if (report["metrics"]["rss"]["max_total_mb"], report["metrics"]["disk"]["max_total_mb"]) != (1955, 236):
        raise ValueError("resource summary changed")
    verify_blocks(report)
    verify_provider(report)
    verify_transactions(report)
    for name in ("mem.jsonl", "rss.csv", "disk.csv"):
        if not (HERE / name).read_text().strip():
            raise ValueError(f"resource sample is empty: {name}")
    for node in (0, 1):
        log = (HERE / f"node-logs/n{node}.log").read_text()
        if log.count("close 1006 (abnormal closure): unexpected EOF") != 40:
            raise ValueError(f"Go node {node} WebSocket cutoff-noise count changed")
        if log.count('error="already stopped" module=rpc-server protocol=websocket') != 40:
            raise ValueError(f"Go node {node} WebSocket stop-noise count changed")


def main() -> None:
    verify_hashes()
    verify_provenance()
    verify_report()
    print("provider soak evidence verified: 310s, 111 heights, 990/990 provider operations")


if __name__ == "__main__":
    main()
