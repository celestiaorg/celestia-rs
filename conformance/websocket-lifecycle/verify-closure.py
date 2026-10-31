#!/usr/bin/env python3
"""Verify the composed #190 WebSocket lifecycle and soak evidence."""

from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]
REVIEW = ROOT / "closure-review.json"
SOAK = REPO / "conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca"


def load(path: Path):
    return json.loads(path.read_text())


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def repo_path(name: str) -> Path:
    relative = Path(name)
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise ValueError(f"unsafe or missing evidence path: {name}")
    path = REPO
    for part in relative.parts:
        path = path / part
        if path.is_symlink():
            raise ValueError(f"symlinked evidence path component: {name}")
    if not path.is_file():
        raise ValueError(f"unsafe or missing evidence path: {name}")
    resolved = path.resolve(strict=True)
    repo = REPO.resolve(strict=True)
    if repo not in resolved.parents:
        raise ValueError(f"out-of-tree evidence path: {name}")
    return resolved


def verify_review(review: dict) -> None:
    if review.get("schema_version") != 1 or review.get("issue") != 190:
        raise ValueError("unexpected closure review schema or issue")
    lifecycle = review["lifecycle_capture"]
    expected_lifecycle = {
        "merge_commit": "3f129c8d214fecae5aa1a942d7fa1a5304ef93cf",
        "manifest": "conformance/websocket-lifecycle/testdata/manifest.json",
        "manifest_sha256": "f3eba1a5843db79a44be178fe7f4d7730183c7ad995d9131302fab83d8917697",
        "cors_enabled": "conformance/websocket-lifecycle/testdata/go-v10.5.0.json",
        "cors_enabled_sha256": "fab1006a449583948a01609632d82e1083f4b6bd43cf179faf862c56f4bfc72e",
        "cors_disabled": "conformance/websocket-lifecycle/testdata/go-v10.5.0-cors-disabled.json",
        "cors_disabled_sha256": "4b71bca15982f8fef9d475f5ac39749a9c4ef6e053303bd10d42236da2fb3b0a",
    }
    if lifecycle != expected_lifecycle:
        raise ValueError("lifecycle evidence identity changed")
    lifecycle_hashes = {
        lifecycle["manifest"]: lifecycle["manifest_sha256"],
        lifecycle["cors_enabled"]: lifecycle["cors_enabled_sha256"],
        lifecycle["cors_disabled"]: lifecycle["cors_disabled_sha256"],
    }
    soak = review["mixed_node_soak"]
    if soak["capture_commit"] != "809bfca7b374bf7463604a851b227dc9452fceb2":
        raise ValueError("mixed-node candidate commit changed")
    directory = soak["directory"]
    if directory != "conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca":
        raise ValueError("mixed-node evidence directory changed")
    expected_soak_hashes = {
        "sha256sums_sha256": "1d8287797084b37ad435f9360e09baab142cf01b0484f0b6e720ec5a97b92764",
        "report_sha256": "b89f076c0bd18080293331a950c48e45118a0ea52dd3d9018ed8e66e33f3d10e",
        "events_sha256": "5c8c7ac48991b02a650cf246bdbd95d7a393d633fdf518d3fe36ffd0c099ec8e",
        "memory_sha256": "017a712e0a13c78b1e752b4e338deaef237ba6a9e27a864a7ef543c60b2c4b33",
        "provenance_sha256": "13ea54964fc445e3992b2fc92eac53c409f2b9ac159bd8ec8813ca7cd98ec10c",
    }
    if any(soak.get(name) != expected for name, expected in expected_soak_hashes.items()):
        raise ValueError("mixed-node evidence hashes changed")
    expected_ws = {"comet-ws-subscribe": 55, "comet-ws-slow-reconnect": 55}
    if soak.get("websocket_results") != expected_ws:
        raise ValueError("WebSocket operation inventory changed")
    soak_hashes = {
        f"{directory}/SHA256SUMS": soak["sha256sums_sha256"],
        f"{directory}/report.json": soak["report_sha256"],
        f"{directory}/events.log": soak["events_sha256"],
        f"{directory}/mem.jsonl": soak["memory_sha256"],
        f"{directory}/provider-provenance.json": soak["provenance_sha256"],
    }
    for name, expected in (lifecycle_hashes | soak_hashes).items():
        if digest(repo_path(name)) != expected:
            raise ValueError(f"reviewed evidence hash changed: {name}")

    expected_acceptance = [
        {"id": "subscription-lifecycle", "evidence": ["go-v10.5.0.json.sequence", "go-v10.5.0.json.events"]},
        {"id": "pressure-and-deadlines", "evidence": ["go-v10.5.0.json.quota", "go-v10.5.0.json.timing", "go-v10.5.0.json.pressure", "rpc::websocket_disconnects_reclaim_subscriptions_and_allow_reconnect"]},
        {"id": "reconnect-and-resource-bounds", "evidence": ["go-v10.5.0.json.events", "go-v10.5.0.json.quota", "provider-soak events.log", "provider-soak mem.jsonl"]},
        {"id": "cors-upgrade-lifetimes", "evidence": ["go-v10.5.0.json.upgrade", "go-v10.5.0-cors-disabled.json.upgrade", "rpc::websocket_origins_match_go_and_deliver_events"]},
        {"id": "cancellation-capacity-recovery", "evidence": ["rpc::websocket_disconnects_reclaim_subscriptions_and_allow_reconnect", "provider-soak events.log"]},
        {"id": "normalization-and-representative-clients", "evidence": ["go-v10.5.0.json.events.ordering_note", "go-v10.5.0.json.timing.normalization", "provider-soak provider-provenance.json"]},
    ]
    acceptance = review["acceptance"]
    if acceptance != expected_acceptance:
        raise ValueError("acceptance criterion inventory changed")
    expected_limitations = [
        "The mixed-node run is a bounded 310-second local capture, not an unbounded soak.",
        "Subscription quota and full outbound-pressure saturation were exercised in the focused lifecycle capture, not repeated during the mixed-node soak.",
        "The captures exclude TLS, reverse proxies, external provider SDKs and browser implementations.",
        "Exact matched-state Go/Rust event-payload equivalence is not claimed; only the stated lifecycle relations and stable upgrade metadata are compared.",
    ]
    if review["limitations"] != expected_limitations:
        raise ValueError("closure limitations changed or disappeared")
    expected_normalizations = [
        "The HTTP Connection upgrade token is compared ASCII-case-insensitively; the exact Go and Rust header values remain in events.log.",
        "Sec-WebSocket-Accept, Date and X-Server-Time are retained but excluded from stable header comparison because they depend on the request nonce or wall clock.",
        "Tx and NewBlock event order is relational because independent subscription goroutines may interleave; both events must identify the transaction height.",
    ]
    if review["normalizations"] != expected_normalizations:
        raise ValueError("closure normalizations changed or disappeared")


def verify_lifecycle() -> None:
    subprocess.run([sys.executable, str(ROOT / "verify.py")], check=True)
    enabled = load(ROOT / "testdata/go-v10.5.0.json")
    disabled = load(ROOT / "testdata/go-v10.5.0-cors-disabled.json")
    for capture in (enabled, disabled):
        upgrades = capture["upgrade"]
        if [row["origin"] for row in upgrades] != [
            "", "https://allowed.example", "https://denied.example"
        ]:
            raise ValueError("upgrade origin inventory changed")
        if not all(row["status"] == 101 and row["active_block_seen"] for row in upgrades):
            raise ValueError("successful upgrade did not remain active")
        if any("Access-Control-Allow-Origin" in row["headers"] or "Vary" in row["headers"] for row in upgrades):
            raise ValueError("upgrade unexpectedly advertised HTTP CORS metadata")
    source = (REPO / "crates/tier-4-runtime/rpc/tests/corpus.rs").read_text()
    for test in (
        "websocket_disconnects_reclaim_subscriptions_and_allow_reconnect",
        "websocket_origins_match_go_and_deliver_events",
    ):
        if f"async fn {test}()" not in source:
            raise ValueError(f"missing Rust lifecycle regression: {test}")


def numeric_summary(rows: list[dict]) -> dict:
    first, last = rows[0], rows[-1]
    summary = {
        "node": first["node"], "kind": first["kind"], "samples": len(rows),
        "rss_first_mb": first["rss_mb"], "rss_last_mb": last["rss_mb"],
        "rss_max_mb": max(row["rss_mb"] for row in rows),
        "threads_first": first["threads"], "threads_last": last["threads"],
        "threads_max": max(row["threads"] for row in rows),
        "fds_first": first["fds"], "fds_last": last["fds"],
        "fds_max": max(row["fds"] for row in rows),
    }
    if first["kind"] == "go":
        summary.update({
            "go_goroutines_first": first["go_goroutines"],
            "go_goroutines_last": last["go_goroutines"],
            "go_goroutines_max": max(row["go_goroutines"] for row in rows),
        })
    return summary


def verify_soak(review: dict) -> None:
    subprocess.run([sys.executable, str(SOAK / "verify.py")], check=True)
    report = load(SOAK / "report.json")
    soak = review["mixed_node_soak"]
    if (report["final"] is not True or report["elapsed_s"] != soak["duration_seconds"]
            or report["heights"] != soak["height_advances"]
            or report["blocks"]["txs"] != soak["transactions"]
            or report["failures"] or report["stalls"] or report["log_hits"]):
        raise ValueError("mixed-node result no longer matches the closure review")
    if report["metrics"]["rss"]["max_total_mb"] != soak["peak_aggregate_rss_mb"]:
        raise ValueError("aggregate RSS bound changed")
    operations = report["metrics"]["provider"]["operations"]
    for operation, total in soak["websocket_results"].items():
        rows = [operations[f"n{node}:{operation}"] for node in range(5)]
        if sum(row["completed"] for row in rows) != total:
            raise ValueError(f"{operation} completion total changed")
        if any(row != {"requested": 11, "completed": 11, "errors": 0, "admission_unknown": 0}
               for row in rows):
            raise ValueError(f"{operation} per-node result changed")

    events = [json.loads(line) for line in (SOAK / "events.log").read_text().splitlines()]
    results = [row for row in events if row.get("operation") in soak["websocket_results"]]
    counts = Counter((row["operation"], row["node"], row["node_kind"]) for row in results)
    node_kinds = {0: "go", 1: "go", 2: "rs", 3: "rs", 4: "rs"}
    expected = Counter({(operation, node, node_kinds[node]): 11
                        for operation in soak["websocket_results"] for node in range(5)})
    if counts != expected or not all(row.get("ok") is True and not row.get("error") for row in results):
        raise ValueError("WebSocket event coverage changed or failed")
    if not all(
        row.get("response_bytes", 0) > 0
        and [value.lower() for value in row["metadata"].get("Connection", [])] == ["upgrade"]
        and row["metadata"].get("Upgrade") == ["websocket"]
        and "Access-Control-Allow-Origin" not in row["metadata"]
        and "Vary" not in row["metadata"]
        for row in results
    ):
        raise ValueError("active WebSocket upgrade metadata changed")
    broadcasts = [row for row in events if row.get("ev") == "provider-broadcast"]
    if len(broadcasts) != 330 or not all(row["code"] == 0 for row in broadcasts):
        raise ValueError("concurrent transaction workload changed or failed")

    by_node = defaultdict(list)
    for line in (SOAK / "mem.jsonl").read_text().splitlines():
        row = json.loads(line)
        by_node[row["node"]].append(row)
    summaries = [numeric_summary(by_node[node]) for node in range(5)]
    if summaries != soak["nodes"]:
        raise ValueError("OS resource sample summary changed")
    if (max(row["threads_max"] for row in summaries) > 21
            or max(row["fds_max"] for row in summaries) > 71
            or max(row.get("go_goroutines_max", 0) for row in summaries) > 150):
        raise ValueError("recorded task/descriptor bound exceeded")

    provenance = load(SOAK / "provider-provenance.json")
    if provenance["source_commit"] != soak["capture_commit"]:
        raise ValueError("soak provenance source commit changed")
    version = provenance["rs_version"]
    if f"commit: {soak['capture_commit']}" not in version:
        raise ValueError("Rust binary identity is not bound to the soak candidate")


def main() -> None:
    review = load(REVIEW)
    verify_review(review)
    verify_lifecycle()
    verify_soak(review)
    print("verified #190 lifecycle closure: 110 WebSocket results across 5 nodes and 145 resource samples")


if __name__ == "__main__":
    main()
