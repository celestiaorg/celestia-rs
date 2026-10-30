#!/usr/bin/env python3
"""Offline RPC evidence ledger. A passing ledger check is not readiness."""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timedelta
import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("policy.json")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
TRANSPORTS = {"comet-uri", "comet-jsonrpc", "comet-websocket", "core-grpc", "cosmos-grpc", "rest"}


def read_json(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source(path):
    return (ROOT / path).read_text()


def checked_path(root, name):
    path = (root / name).resolve()
    if not path.is_relative_to(root.resolve()) or not path.is_file():
        raise ValueError(f"missing or out-of-tree file: {name}")
    return path


def endpoints():
    """Source declarations are candidates, never proof of Go registration/parity."""
    result = []
    env = source("crates/tier-4-runtime/rpc/src/env.rs")
    routes = env.split("pub const ROUTES:", 1)[1].split("impl Route", 1)[0]
    for name in re.findall(r'route\(\s*"([a-z_0-9]+)"', routes):
        transports = ["comet-websocket"]
        if name not in {"subscribe", "unsubscribe", "unsubscribe_all"}:
            transports += ["comet-uri", "comet-jsonrpc"]
        unsafe = name in {"dial_seeds", "dial_peers", "unsafe_flush_mempool"}
        for transport in transports:
            result.append({"transport": transport, "endpoint": name,
                           "declaration": "unsafe-route" if unsafe else "route"})
    core = source("crates/tier-4-runtime/rpc/src/grpc.rs")
    methods = core.split("pub const METHODS:", 1)[1].split("];", 1)[0]
    for path in re.findall(r'"(/tendermint\.rpc\.grpc\.[^" ]+/[^" ]+)"', methods):
        result.append({"transport": "core-grpc", "endpoint": path, "declaration": "handler"})
        result.append({"transport": "cosmos-grpc", "endpoint": path, "declaration": "merged-core-handler-candidate"})
    table = source("crates/tier-1-protocol-types/celestia-proto/src/gen/services.rs")
    rest = source("crates/tier-4-runtime/api-server/src/rest.rs")
    excluded = rest.split("const NOT_ON_GATEWAY:", 1)[1].split("];", 1)[0]
    excluded = re.findall(r'"([^"]+)"', excluded)
    for path, bindings in re.findall(r'Method\{path:"([^"]+)"[^\n]*?http:&\[(.*?)\]\}', table):
        result.append({"transport": "cosmos-grpc", "endpoint": path,
                       "declaration": "descriptor-candidate"})
        if not any(path.startswith(prefix) for prefix in excluded):
            for verb, pattern in re.findall(r'H\{method:"([^"]+)",pattern:"([^"]+)"', bindings):
                result.append({"transport": "rest", "endpoint": verb + " " + pattern,
                               "grpc_method": path, "declaration": "gateway-candidate"})
    # Streaming reflection is not represented by the unary generated method table.
    for transport in ("core-grpc", "cosmos-grpc"):
        for version in ("v1", "v1alpha"):
            result.append({"transport": transport,
                           "endpoint": f"/grpc.reflection.{version}.ServerReflection/ServerReflectionInfo",
                           "declaration": "required-registration-check"})
    if len(result) != len({(e["transport"], e["endpoint"]) for e in result}):
        raise ValueError("duplicate source endpoint")
    return sorted(result, key=lambda e: (e["transport"], e["endpoint"]))


def legacy_fixtures():
    result = []
    for path in sorted((ROOT / "conformance/rpccorpus").glob("*/meta.json")):
        meta = read_json(path)
        corpus = path.with_name("corpus.json.gz")
        data = json.loads(gzip.decompress(corpus.read_bytes()))
        result.append({"fixture": str(corpus.relative_to(ROOT)), "sha256": digest(corpus),
                       "provenance": meta, "classification": "regression-only",
                       "missing_provenance": ["release_commit", "compiler", "build_tags", "input_hashes", "capture_source_hashes"],
                       "cases": len(data["entries"]),
                       "explicit_normalizations": dict(Counter(e.get("norm", "") or "none-declared" for e in data["entries"]))})
    for path in sorted((ROOT / "conformance/querycorpus").glob("*.json.gz")):
        scenario = path.name.removesuffix(".json.gz")
        meta_path = ROOT / "conformance/fixtures/devnet" / scenario / "meta.json"
        data = json.loads(gzip.decompress(path.read_bytes()))
        result.append({"fixture": str(path.relative_to(ROOT)), "sha256": digest(path),
                       "provenance": read_json(meta_path) if meta_path.exists() else None,
                       "classification": "regression-only",
                       "missing_provenance": ["response_capture_release_commit", "response_capture_compiler", "build_tags", "capture_source_hashes"],
                       "abci_cases": len(data.get("abci", [])), "rest_cases": len(data.get("rest", []))})
    return result


def validate_evidence(manifest, policy, root=ROOT):
    """Admission is deliberately stricter than legacy metadata."""
    oracle = manifest["oracle"]
    for key in ("celestia_app", "celestia_app_commit", "celestia_core", "celestia_core_commit", "cosmos_sdk", "cosmos_sdk_commit"):
        if oracle[key] != policy["oracle"][key]:
            raise ValueError(f"current evidence oracle mismatch: {key}")
    for key in ("celestia_app_commit", "celestia_core_commit", "cosmos_sdk_commit"):
        if not COMMIT.fullmatch(oracle[key]):
            raise ValueError(f"invalid oracle commit: {key}")
    if not re.fullmatch(r"go1\.[0-9]+\.[0-9]+", oracle["compiler"]):
        raise ValueError("record the actual Go compiler")
    if not isinstance(oracle["build_tags"], list):
        raise ValueError("build_tags must be an explicit list, including when empty")
    for key in ("go", "rs"):
        if not SHA256.fullmatch(manifest["binary_sha256"][key]):
            raise ValueError(f"missing binary SHA-256: {key}")
    for key in ("genesis", "workload", "capture_source", "mainnet_observation"):
        artifact = manifest["inputs"][key]
        path = checked_path(root, artifact["path"])
        if not SHA256.fullmatch(artifact["sha256"]) or digest(path) != artifact["sha256"]:
            raise ValueError(f"input hash mismatch: {key}")
    observation = read_json(checked_path(root, manifest["inputs"]["mainnet_observation"]["path"]))
    if observation["version"] != oracle["celestia_app"].removeprefix("v"):
        raise ValueError("mainnet observation version does not match oracle")
    if not observation["observed_at"] or not observation["endpoint"].startswith("https://"):
        raise ValueError("mainnet observation needs time and public HTTPS endpoint")
    captured = datetime.fromisoformat(manifest["captured_at"])
    observed = datetime.fromisoformat(observation["observed_at"])
    if captured.utcoffset() is None or observed.utcoffset() is None or not timedelta(0) <= captured - observed <= timedelta(hours=24):
        raise ValueError("capture requires a preceding mainnet observation within 24 hours")
    for route in ("status", "abci_info"):
        response = observation["responses"][route]
        if hashlib.sha256(response["body"].encode()).hexdigest() != response["sha256"]:
            raise ValueError("mainnet raw response hash mismatch")
    status = json.loads(observation["responses"]["status"]["body"])["result"]
    info = json.loads(observation["responses"]["abci_info"]["body"])["result"]["response"]
    if status["node_info"]["network"] != "celestia" or info["version"] != observation["version"]:
        raise ValueError("raw mainnet responses disagree with oracle observation")
    state = manifest["matched_state"]
    if state["go"] != state["rs"] or not state["go"]["chain_id"] or state["go"]["height"] < 1:
        raise ValueError("Go/RS chain, height and app hash must match")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", state["go"]["app_hash"]):
        raise ValueError("record the matched committed app hash")
    if not manifest["capture_command"] or not manifest["captured_at"] or not manifest["cases"]:
        raise ValueError("evidence needs capture command, time and cases")
    for case in manifest["cases"]:
        if case["transport"] not in TRANSPORTS or not case["endpoint"]:
            raise ValueError("invalid evidence endpoint")
        if case["behavior"] not in policy["behaviors"]:
            raise ValueError("unknown behavior")
        if case["comparison"] not in policy["comparison_policies"]:
            raise ValueError("unknown comparison policy")
        if case["outcome"] not in {"exact", "normalized", "mismatch", "excluded"}:
            raise ValueError("unknown comparison outcome")
        if (case["outcome"] == "exact" and case["comparison"] != "exact") or (case["outcome"] == "normalized" and case["comparison"] == "exact"):
            raise ValueError("normalizations/exclusions must never count as exact")
        if case["outcome"] != "exact" and not case.get("reason"):
            raise ValueError("record why a comparison is not exact")
        for key in ("request", "go_response", "rs_response"):
            artifact = case[key]
            path = checked_path(root, artifact["path"])
            if digest(path) != artifact["sha256"]:
                raise ValueError(f"case hash mismatch: {key}")
        if case["outcome"] == "exact" and case["go_response"]["sha256"] != case["rs_response"]["sha256"]:
            raise ValueError("exact result has different Go/RS response artifacts")
        test = case["test"]
        checked_path(root, test["source"])
        if not all(re.fullmatch(r"[A-Za-z0-9_:-]+", test[k]) for k in ("package", "target", "filter")):
            raise ValueError("invalid cargo test selector")
        function = test["filter"].rsplit("::", 1)[-1]
        if not re.search(r"\bfn\s+" + re.escape(function) + r"\s*\(", checked_path(root, test["source"]).read_text()):
            raise ValueError("regression selector function absent from test source")
    return manifest


def inventory(policy):
    if policy["schema_version"] != 1 or set(v["issue"] for v in policy["behaviors"].values()) != set(range(182, 193)):
        raise ValueError("policy must account for every tracked behavior issue #182–192")
    admitted = []
    for name in policy["current_evidence"]:
        admitted.append(validate_evidence(read_json(checked_path(ROOT, name)), policy))
    declared = endpoints()
    identities = {(e["transport"], e["endpoint"]) for e in declared}
    for manifest in admitted:
        for case in manifest["cases"]:
            if (case["transport"], case["endpoint"]) not in identities:
                raise ValueError("evidence endpoint absent from source inventory")
    cases = [case for manifest in admitted for case in manifest["cases"]]
    for endpoint in declared:
        endpoint["unassessed_behaviors"] = sorted(set(policy["behaviors"]) - {c["behavior"] for c in cases if (c["transport"], c["endpoint"]) == (endpoint["transport"], endpoint["endpoint"])})
        endpoint["evidence"] = [case for case in cases if (case["transport"], case["endpoint"]) == (endpoint["transport"], endpoint["endpoint"])]
    query_source = source("crates/devnet-conformance/src/querycorpus.rs")
    skipped = query_source.split("pub const NEEDS_CONSENSUS:", 1)[1].split("];", 1)[0]
    inputs = ["crates/tier-4-runtime/rpc/src/env.rs", "crates/tier-4-runtime/rpc/src/grpc.rs",
              "crates/tier-4-runtime/api-server/src/rest.rs", "crates/tier-5-node/celestia-rs/src/rpc_service.rs",
              "crates/tier-1-protocol-types/celestia-proto/src/gen/services.rs",
              "crates/devnet-conformance/src/querycorpus.rs", "crates/devnet-conformance/tests/query_corpus.rs",
              "crates/tier-4-runtime/rpc/tests/corpus.rs"]
    return {"schema_version": 1, "oracle": policy["oracle"],
            "inventory_source_sha256": {name: digest(ROOT / name) for name in inputs},
            "legacy_skipped_query_prefixes": re.findall(r'"([^"]+)"', skipped),
            "provider_ready": False,
            "readiness_reason": "Requires complete behavior coverage, reviewed deviations and the bounded all-API soak; ledger validation alone never qualifies a provider.",
            "current_evidence_manifests": len(admitted), "current_cases": dict(Counter(c["outcome"] for c in cases)),
            "endpoints": declared, "behaviors": policy["behaviors"],
            "comparison_policies": policy["comparison_policies"], "known_mismatches": policy["known_mismatches"],
            "exclusions": policy["exclusions"], "legacy_fixtures": legacy_fixtures()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path, help="Write source/evidence inventory as JSON")
    parser.add_argument("--run-current", action="store_true", help="Run only admitted current-release regression selectors (no capture/network)")
    args = parser.parse_args()
    try:
        report = inventory(read_json(POLICY))
        if args.json:
            args.json.write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['endpoints'])} source endpoint candidates; {report['current_evidence_manifests']} admitted current-release manifests; {len(report['legacy_fixtures'])} regression-only fixtures")
        print("Provider readiness: NOT ESTABLISHED (see #181 and policy.json)")
        if args.run_current:
            tests = {tuple(c["test"][k] for k in ("package", "target", "filter"))
                     for endpoint in report["endpoints"] for c in endpoint["evidence"]}
            if not tests:
                print("No current-release selectors admitted; this is an inventory guard, not a current-release parity pass.")
            for package, target, test_filter in sorted(tests):
                command = ["cargo", "test", "--locked", "-p", package, "--test", target, "--"]
                listed = subprocess.run(command + ["--list"], cwd=ROOT, check=True, text=True, capture_output=True)
                if f"{test_filter}: test" not in listed.stdout.splitlines():
                    raise ValueError(f"current-release regression selected zero tests: {test_filter}")
                subprocess.run(command + ["--exact", test_filter], cwd=ROOT, check=True)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"RPC evidence validation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
