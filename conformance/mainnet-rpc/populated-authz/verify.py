#!/usr/bin/env python3
"""Verify authz/feegrant provenance and scope; does not execute Rust replay."""
from datetime import datetime, timedelta
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "conformance/fixtures/devnet/v105-authz-feegrant"
CAPTURE_COMMIT = "cabc055deb9dcfa5e438adb2d67365b7a104d9b7"
# The squash merge preserves every receipted source byte and is reachable in
# CI's full main history, unlike the original pre-squash capture commit.
PUBLISHED_SOURCE_COMMIT = "8f9729c9a300751b09679e5f48133bd652a7553f"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_sources(metadata):
    require(metadata["source_commit"] == CAPTURE_COMMIT, "capture source commit changed")
    require("Cargo.lock" in metadata["source_sha256"], "missing lockfile source receipt")
    for name, expected in metadata["source_sha256"].items():
        path = (ROOT / name).resolve()
        require(path.is_relative_to(ROOT), "source escapes repository")
        try:
            recorded = subprocess.run(
                ["git", "show", f"{PUBLISHED_SOURCE_COMMIT}:{name}"],
                cwd=ROOT, check=True, capture_output=True,
            ).stdout
        except (OSError, subprocess.CalledProcessError) as error:
            raise ValueError(f"captured source unavailable: {name}") from error
        require(hashlib.sha256(recorded).hexdigest() == expected, f"source hash mismatch: {name}")


def verify():
    metadata = json.loads((OUT / "capture/metadata.json").read_text())
    require(metadata["scenario"] == "v105-authz-feegrant", "wrong scenario")
    require(metadata["comparison"] == "existing-querycorpus-canonicalization", "comparison scope changed")
    require(len(metadata["omissions"]) == 3 and len(metadata["normalizations"]) == 3, "missing comparison limits")
    verify_sources(metadata)
    for name, expected in metadata["artifact_sha256"].items():
        path = (ROOT / name).resolve()
        require(path.is_relative_to(ROOT), "artifact escapes repository")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, f"hash mismatch: {name}")
    for name in ("conformance/devnet/workloads/authz-feegrant.sh",
                 "crates/devnet-conformance/examples/record_queries.rs",
                 "crates/devnet-conformance/src/querycorpus.rs"):
        require(name in metadata["source_sha256"], f"missing source receipt: {name}")
    actual = {str(p.relative_to(ROOT)) for p in OUT.rglob("*") if p.is_file() and p.name != "metadata.json"}
    actual.add("conformance/querycorpus/v105-authz-feegrant.json.gz")
    require(set(metadata["artifact_sha256"]) == actual, "incomplete artifact inventory")
    for name, module, commit in (
        ("celestia-appd", "github.com/celestiaorg/celestia-app/v10@v10.5.0", "6013e758d5dceb75827ad191c8d739d7c43cfa04"),
        ("cometbft", "github.com/celestiaorg/celestia-core@v0.42.3", "a5537d84a008d566abc503d2ea41e30f2301403d"),
    ):
        receipt = json.loads((OUT / f"capture/{name}.build.json").read_text())
        require(receipt["module"] == module and receipt["source_commit"] == commit, "wrong oracle release")
        require(receipt["compiler"] == "go version go1.26.9 linux/amd64", "unexpected oracle compiler")
        if name == "celestia-appd":
            require(receipt["build_tags"] == ["ledger"], "missing ledger registration")
    observation = json.loads((OUT / "capture/mainnet-observation.json").read_text())
    elapsed = datetime.fromisoformat(metadata["captured_at"]) - datetime.fromisoformat(observation["observed_at"])
    require(timedelta(0) <= elapsed <= timedelta(hours=24), "stale mainnet observation")
    parsed = {}
    for route in ("status", "abci_info"):
        raw = observation["responses"][route]["body"].encode()
        require(hashlib.sha256(raw).hexdigest() == observation["responses"][route]["sha256"], "unbound observation")
        parsed[route] = json.loads(raw)["result"]
    status, info = parsed["status"], parsed["abci_info"]["response"]
    require(status["node_info"]["network"] == "celestia" and info["version"] == "10.5.0", "wrong active release")
    require(status["sync_info"]["latest_block_height"] == info["last_block_height"], "observation heights differ")
    with gzip.open(ROOT / "conformance/querycorpus/v105-authz-feegrant.json.gz", "rt") as stream:
        corpus = json.load(stream)
    require(corpus["scenario"] == metadata["scenario"], "corpus identity differs")
    require(len(corpus["abci"]) > 1000 and len(corpus["rest"]) > 500, "empty or truncated corpus")
    require(any(case["path"].startswith("/cosmos.authz.") and case["response"].get("value") for case in corpus["abci"]), "no authz query response")
    require(any(case["path"].startswith("/cosmos.feegrant.") and case["response"].get("value") for case in corpus["abci"]), "no feegrant query response")
    require(any(case.get("prove") and case["response"].get("proof_ops") for case in corpus["abci"]), "no proof response")
    state = json.loads((OUT / f"state/{corpus['last_height']}.json").read_text())
    require(state["height"] == corpus["last_height"] and len(state["app_hash"]) == 64, "missing final state root")
    print(f"Verified fresh authz/feegrant inputs: {len(corpus['abci'])} ABCI and {len(corpus['rest'])} REST requests; runtime replay remains separate")


if __name__ == "__main__":
    verify()
