#!/usr/bin/env python3
"""Bind a fresh authz capture to its oracle, public inputs and recorder sources."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "conformance/fixtures/devnet/v105-authz-feegrant"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--observation", type=Path, required=True)
    parser.add_argument("--bin", type=Path, required=True)
    args = parser.parse_args()
    receipt = OUT / "capture"
    receipt.mkdir()
    shutil.copyfile(args.observation, receipt / "mainnet-observation.json")
    for name in ("celestia-appd", "cometbft"):
        shutil.copyfile(args.bin / (name + ".build.json"), receipt / (name + ".build.json"))
        shutil.copyfile(args.bin / (name + ".build-info.txt"), receipt / (name + ".build-info.txt"))
    sources = [
        "conformance/devnet/build-bins.sh", "conformance/devnet/devnet.sh",
        "conformance/devnet/workloads/authz-feegrant.sh", "conformance/devnet/workloads/lib.sh",
        "conformance/devnet/workloads/wait_tx.py", "conformance/querycorpus/record.sh",
        "conformance/mainnet-rpc/observe-mainnet.py", "Cargo.toml", "Cargo.lock",
        "crates/devnet-conformance/Cargo.toml", "crates/devnet-conformance/src/querycorpus.rs",
        "crates/devnet-conformance/examples/record_queries.rs",
        "conformance/devnet-oracle/main.go", "conformance/devnet-oracle/go.mod", "conformance/devnet-oracle/go.sum",
        "conformance/rpccorpus/recorder/go.mod", "conformance/rpccorpus/recorder/go.sum",
        "conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go",
    ]
    sources += [str(p.relative_to(ROOT)) for p in sorted((ROOT / "crates/devnet-conformance/src/querycorpus").rglob("*.rs"))]
    sources += [str(p.relative_to(ROOT)) for p in sorted((ROOT / "conformance/mainnet-rpc/populated-authz").glob("*")) if p.is_file()]
    artifacts = [str(p.relative_to(ROOT)) for p in sorted(OUT.rglob("*")) if p.is_file()]
    artifacts.append("conformance/querycorpus/v105-authz-feegrant.json.gz")
    data = {"schema_version": 1, "captured_at": datetime.now(timezone.utc).isoformat(),
            "scenario": "v105-authz-feegrant", "comparison": "existing-querycorpus-canonicalization",
            "source_commit": subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
            "recorder_compiler": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "source_sha256": {p: digest(ROOT / p) for p in sources},
            "artifact_sha256": {p: digest(ROOT / p) for p in artifacts},
            "generator_sha256": {name: digest(args.bin / name) for name in ("devnet-oracle", "abci-recorder", "rpc-recorder")},
            "query_recorder_sha256": digest(ROOT / "target/debug/examples/record_queries"),
            "scope": "fresh v10.5.0 authz/feegrant workload, exact ABCI replay roots and ABCI/REST corpus",
            "omissions": ["real Comet/Cosmos gRPC/REST Rust listeners (the existing query harness calls app/gateway handlers)",
                          "runtime build identity, consensus-backed query handlers and client socket lifecycle",
                          "other module campaigns, independent ICS23 proof validation and universal provider parity"],
            "normalizations": ["querycorpus canonical_value sorts selected map-order lists and removes GetNodeInfo application_version",
                               "querycorpus comparable strips Go diagnostic source frames",
                               "REST comparisons parse JSON, sort selected map-order lists and remove GetNodeInfo application_version"]}
    (receipt / "metadata.json").write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    main()
