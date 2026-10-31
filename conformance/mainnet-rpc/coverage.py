#!/usr/bin/env python3
"""Offline RPC evidence ledger. A passing ledger check is not readiness."""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timedelta
import gzip
import hashlib
import importlib.util
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
REVIEW_OUTCOMES = {
    "normalized-runtime-comparison", "runtime-exact-unrecorded", "schema-only",
    "rust-with-stand-in-only", "go-transports-only",
}
ADMISSION_BLOCKERS = {
    "missing-exact-oracle-identity", "missing-exact-dependency-commits",
    "missing-go-binary-hash", "missing-rs-binary-hash",
    "missing-go-matched-state", "missing-rs-matched-state",
    "missing-oracle-build-receipt", "missing-dependency-receipts",
    "missing-committed-go-response", "missing-committed-rs-response",
    "missing-committed-genesis", "missing-committed-workload",
    "missing-capture-source", "missing-capture-time", "missing-complete-mainnet-observation",
    "comparison-not-exact",
}
REVIEW_CONTRACTS = {
    182: {
        "bindings": {
            "oracle_build": ("conformance/grpc-reflection/v10.5.0/go-build.json",),
            "dependency_receipts": ("conformance/grpc-reflection/v10.5.0/meta.json",),
            "genesis": (), "workload": (),
            "capture_source": ("conformance/grpc-reflection/capture.go",),
            "mainnet_observation": (),
            "go_responses": (
                "conformance/grpc-reflection/v10.5.0/app-limits.json.gz",
                "conformance/grpc-reflection/v10.5.0/app.json.gz",
                "conformance/grpc-reflection/v10.5.0/core.json.gz",
            ),
            "rs_responses": (),
        },
        "endpoints": (
            ("core-grpc", "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo"),
            ("core-grpc", "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"),
            ("cosmos-grpc", "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"),
        ),
        "regressions": (
            ("cargo", "grpc-reflection", "transport", "core_v1_and_v1alpha_match_go_socket_streams", "crates/tier-4-runtime/grpc-reflection/tests/transport.rs"),
            ("cargo", "grpc-reflection", "transport", "sdk_v1alpha_matches_go_socket_streams", "crates/tier-4-runtime/grpc-reflection/tests/transport.rs"),
        ),
    },
    184: {
        "bindings": {
            "oracle_build": ("conformance/node-metadata/v10.5.0/consensus.json",),
            "dependency_receipts": (), "genesis": (), "workload": (),
            "capture_source": ("conformance/node-metadata/record.py",),
            "mainnet_observation": (),
            "go_responses": ("conformance/node-metadata/v10.5.0/consensus.json",),
            "rs_responses": (),
        },
        "endpoints": (
            ("comet-uri", "abci_info"), ("comet-uri", "abci_query"),
            ("comet-uri", "consensus_state"), ("comet-uri", "dump_consensus_state"),
            ("comet-uri", "net_info"), ("comet-uri", "status"),
            ("cosmos-grpc", "/cosmos.base.tendermint.v1beta1.Service/GetNodeInfo"),
            ("rest", "GET /cosmos/base/tendermint/v1beta1/node_info"),
        ),
        "regressions": (
            ("cargo", "celestia-rs", "runtime_metadata", "production_listeners_report_build_and_compatibility_separately", "crates/tier-5-node/celestia-rs/tests/runtime_metadata.rs"),
        ),
    },
    185: {
        "bindings": {
            "oracle_build": ("conformance/mainnet-rpc/simulation/oracle-build.json",),
            "dependency_receipts": (
                "conformance/mainnet-rpc/simulation/dependencies.json",
                "conformance/mainnet-rpc/simulation/metadata.json",
            ),
            "genesis": ("conformance/fixtures/devnet/empty/genesis.json",),
            "workload": ("conformance/fixtures/devnet/empty/abci.rec",),
            "capture_source": ("conformance/mainnet-rpc/simulation/record.sh",),
            "mainnet_observation": ("conformance/mainnet-rpc/simulation/mainnet-observation.json",),
            "go_responses": ("conformance/mainnet-rpc/simulation/go-errors.json",),
            "rs_responses": (),
        },
        "endpoints": (("cosmos-grpc", "/cosmos.tx.v1beta1.Service/Simulate"),),
        "regressions": (
            ("cargo", "devnet-conformance", "simulation_errors", "current_release_simulation_errors", "crates/devnet-conformance/tests/simulation_errors.rs"),
        ),
    },
    186: {
        "bindings": {
            "oracle_build": ("conformance/mainnet-rpc/v10.5.0/empty-query-meta.json",),
            "dependency_receipts": ("conformance/mainnet-rpc/v10.5.0/empty-query-meta.json",),
            "genesis": (), "workload": (),
            "capture_source": ("conformance/mainnet-rpc/empty-query/record.sh",),
            "mainnet_observation": (),
            "go_responses": (
                "conformance/mainnet-rpc/v10.5.0/empty-query-abci.json",
                "conformance/mainnet-rpc/v10.5.0/empty-query.json",
            ),
            "rs_responses": (),
        },
        "endpoints": (
            ("comet-jsonrpc", "abci_query"), ("comet-uri", "abci_query"),
            ("comet-websocket", "abci_query"),
        ),
        "regressions": (
            ("cargo", "rpc", "empty_query", "p2p_empty_query_matches_current_go_over_reusable_http_and_websocket", "crates/tier-4-runtime/rpc/tests/empty_query.rs"),
        ),
    },
    187: {
        "bindings": {name: () for name in (
            "oracle_build", "dependency_receipts", "genesis", "workload",
            "capture_source", "mainnet_observation", "go_responses", "rs_responses",
        )},
        "endpoints": (
            ("core-grpc", "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx"),
            ("cosmos-grpc", "/cosmos.tx.v1beta1.Service/BroadcastTx"),
        ),
        "regressions": (
            ("cargo", "devnet-conformance", "grpc_server", "broadcast_tx_reaches_check_tx", "crates/devnet-conformance/tests/grpc_server.rs"),
            ("cargo", "rpc", "lib", "core_broadcast_is_served_over_http2", "crates/tier-4-runtime/rpc/src/grpc.rs"),
            ("cargo", "rpc", "lib", "core_broadcast_projects_the_go_response_fields", "crates/tier-4-runtime/rpc/src/grpc.rs"),
        ),
    },
    188: {
        "bindings": {
            "oracle_build": ("conformance/fixtures/devnet/v105-populated-staking/capture/app-build.json",),
            "dependency_receipts": (
                "conformance/fixtures/devnet/v105-populated-staking/capture/app-build-info.txt",
                "conformance/fixtures/devnet/v105-populated-staking/capture/core-build.json",
                "conformance/fixtures/devnet/v105-populated-staking/capture/metadata.json",
            ),
            "genesis": ("conformance/fixtures/devnet/v105-populated-staking/genesis.json",),
            "workload": ("conformance/devnet/workloads/staking.sh",),
            "capture_source": ("conformance/mainnet-rpc/populated-staking/record.sh",),
            "mainnet_observation": ("conformance/fixtures/devnet/v105-populated-staking/capture/mainnet-observation.json",),
            "go_responses": ("conformance/fixtures/devnet/v105-populated-staking/capture/transports.json",),
            "rs_responses": (),
        },
        "endpoints": (
            ("comet-jsonrpc", "abci_query"),
            ("cosmos-grpc", "/cosmos.distribution.v1beta1.Query/DelegationTotalRewards"),
            ("cosmos-grpc", "/cosmos.staking.v1beta1.Query/DelegatorDelegations"),
            ("cosmos-grpc", "/cosmos.staking.v1beta1.Query/Validator"),
            ("cosmos-grpc", "/cosmos.staking.v1beta1.Query/Validators"),
            ("rest", "GET /cosmos/distribution/v1beta1/delegators/{delegator_address}/rewards"),
            ("rest", "GET /cosmos/staking/v1beta1/delegations/{delegator_addr}"),
            ("rest", "GET /cosmos/staking/v1beta1/validators"),
            ("rest", "GET /cosmos/staking/v1beta1/validators/{validator_addr}"),
        ),
        "regressions": (
            ("python-unittest", "conformance.mainnet-rpc.tests", "test_populated_staking", "PopulatedStakingEvidence.test_abci_comet_and_grpc_return_identical_protobuf", "conformance/mainnet-rpc/tests/test_populated_staking.py"),
        ),
    },
}


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


def validate_artifact(artifact, root=ROOT):
    if set(artifact) != {"path", "sha256"}:
        raise ValueError("artifact binding must contain only path and sha256")
    path = checked_path(root, artifact["path"])
    if not SHA256.fullmatch(artifact["sha256"]) or digest(path) != artifact["sha256"]:
        raise ValueError(f"review artifact hash mismatch: {artifact['path']}")
    return path


def artifact_text(artifact, root=ROOT):
    path = validate_artifact(artifact, root)
    try:
        value = read_json(path)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return path.read_text(errors="replace")

    def scalars(item):
        if isinstance(item, dict):
            for key, child in item.items():
                yield str(key)
                yield from scalars(child)
        elif isinstance(item, list):
            for child in item:
                yield from scalars(child)
        elif item is not None:
            yield str(item)

    return "\n".join(scalars(value))


def binding_paths(bindings):
    result = {}
    for name, value in bindings.items():
        if value is None:
            result[name] = ()
        elif isinstance(value, list):
            result[name] = tuple(sorted(item["path"] for item in value))
        else:
            result[name] = (value["path"],)
    return result


def validate_review_semantics(review, root=ROOT):
    """Parse the bound response and state receipts instead of trusting their paths."""
    issue = review["issue"]
    bindings = review["bindings"]
    response_paths = {item["path"]: checked_path(root, item["path"])
                      for item in bindings["go_responses"]}
    if issue == 182:
        expected = {
            "conformance/grpc-reflection/v10.5.0/app.json.gz": ("127.0.0.1:10090", 41),
            "conformance/grpc-reflection/v10.5.0/core.json.gz": ("127.0.0.1:27658", 5),
            "conformance/grpc-reflection/v10.5.0/app-limits.json.gz": ("127.0.0.1:14090", None),
        }
        for name, (endpoint, service_count) in expected.items():
            capture = json.loads(gzip.decompress(response_paths[name].read_bytes()))
            services = capture.get("services")
            if capture["endpoint"] != endpoint or (service_count is not None and len(services) != service_count):
                raise ValueError("reflection response binding has unexpected listener or services")
            if not capture["sessions"]:
                raise ValueError("reflection response binding has no captured sessions")
    elif issue == 184:
        capture = read_json(response_paths["conformance/node-metadata/v10.5.0/consensus.json"])
        expected = {"abci_info", "app_version", "consensus_state", "dump_consensus_state", "net_info", "status"}
        if set(capture["responses"]) != expected or capture["scope"].find("no matched-state equality") < 0:
            raise ValueError("runtime identity response binding changed scope or route set")
        for response in capture["responses"].values():
            if "result" not in json.loads(response["body"]):
                raise ValueError("runtime identity response binding is not a successful JSON-RPC result")
    elif issue == 185:
        cases = read_json(response_paths["conformance/mainnet-rpc/simulation/go-errors.json"])
        expected = {
            "missing-tx-bytes", "malformed-request", "decoder-wrong-wire-type",
            "decoder-truncated-body", "decoder-empty-tx", "ante-missing-signatures",
            "ante-wrong-sequence", "ante-long-memo", "execution-insufficient-funds",
            "successful-simulation", "successful-simulation-repeat", "missing-latest-height",
            "missing-historical-height", "future-height",
        }
        if len(cases) != 14 or {case["name"] for case in cases} != expected or any(
                "request_hex" not in case or "code" not in case or "message" not in case
                for case in cases):
            raise ValueError("simulation response binding does not match the captured request inventory")
    elif issue == 186:
        http = read_json(response_paths["conformance/mainnet-rpc/v10.5.0/empty-query.json"])
        abci = read_json(response_paths["conformance/mainnet-rpc/v10.5.0/empty-query-abci.json"])
        if len(http["cases"]) != 28 or {case["transport"] for case in http["cases"]} != {
                "uri", "jsonrpc", "jsonrpc_batch", "websocket"}:
            raise ValueError("empty-query response binding has unexpected transport inventory")
        if len(abci["cases"]) != 9 or not abci["echo_after"] or not abci["flush_after"]:
            raise ValueError("empty-query ABCI response binding lacks connection-survival controls")
        if http["binary_sha256"] != review["binary_sha256"]["go"] or abci["binary_sha256"] != review["binary_sha256"]["go"]:
            raise ValueError("empty-query response binding disagrees with Go binary identity")
    elif issue == 188:
        capture = read_json(response_paths[
            "conformance/fixtures/devnet/v105-populated-staking/capture/transports.json"])
        methods = {case["method"] for case in capture["cases"]}
        expected = {
            "/cosmos.distribution.v1beta1.Query/DelegationTotalRewards",
            "/cosmos.staking.v1beta1.Query/DelegatorDelegations",
            "/cosmos.staking.v1beta1.Query/Validator",
            "/cosmos.staking.v1beta1.Query/Validators",
        }
        if len(capture["cases"]) != 11 or methods != expected:
            raise ValueError("populated response binding has unexpected case or method inventory")
        if any(set(case["transports"]) != {"abci", "comet_rpc", "cosmos_grpc", "rest"}
               for case in capture["cases"]):
            raise ValueError("populated response binding lacks a recorded Go transport")

    state = review["matched_state"]["go"]
    if state is not None:
        parsed = read_json(validate_artifact(state["source"], root))
        recorded = parsed if issue == 185 else parsed["state_identity"]
        height_key = "height" if issue == 185 else "query_height"
        actual = (recorded["chain_id"], recorded[height_key], recorded["app_hash"].upper())
        expected = (state["chain_id"], state["height"], state["app_hash"].upper())
        if actual != expected:
            raise ValueError("reviewed Go matched state disagrees with its bound receipt")

    observation = bindings["mainnet_observation"]
    if observation is not None:
        parsed = read_json(validate_artifact(observation, root))
        if parsed["chain_id"] != "celestia" or parsed["version"] != "10.5.0":
            raise ValueError("reviewed mainnet observation disagrees with the selected oracle")
        for route in ("status", "abci_info"):
            response = parsed["responses"][route]
            if hashlib.sha256(response["body"].encode()).hexdigest() != response["sha256"]:
                raise ValueError("reviewed mainnet observation raw hash mismatch")


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


def validate_reviews(review_ledger, policy, root=ROOT):
    """Bind reviewed evidence without allowing a partial package into admission."""
    if review_ledger["schema_version"] != 1:
        raise ValueError("unknown evidence review schema")
    expected_issues = {182, 184, 185, 186, 187, 188}
    reviews = review_ledger["reviews"]
    if {review["issue"] for review in reviews} != expected_issues or len(reviews) != len(expected_issues):
        raise ValueError("reviews must account exactly once for issues 182, 184-188")

    profiles = review_ledger["oracle_profiles"]
    policy_oracle_keys = (
        "celestia_app", "celestia_app_commit", "celestia_core",
        "celestia_core_commit", "cosmos_sdk", "cosmos_sdk_commit",
    )
    for name, profile in profiles.items():
        if any(profile[key] != policy["oracle"][key] for key in policy_oracle_keys):
            raise ValueError(f"review oracle profile disagrees with policy: {name}")
        if not re.fullmatch(r"go1\.[0-9]+\.[0-9]+", profile["compiler"]):
            raise ValueError(f"review oracle profile lacks an actual compiler: {name}")
        if not isinstance(profile["build_tags"], list):
            raise ValueError(f"review oracle profile lacks explicit build tags: {name}")

    source_identities = {(item["transport"], item["endpoint"]) for item in endpoints()}
    for review in reviews:
        contract = REVIEW_CONTRACTS[review["issue"]]
        if review["status"] != "not-admitted":
            raise ValueError("reviewed partial evidence must remain not-admitted")
        if review["issue"] not in {item["issue"] for item in policy["behaviors"].values()}:
            raise ValueError("review refers to an untracked issue")
        if not review["scope"]:
            raise ValueError("review scope is required")
        if not review["capture_times"]:
            if "missing-capture-time" not in review["admission_blockers"]:
                raise ValueError("review omits capture time without a blocker")
        else:
            if "missing-capture-time" in review["admission_blockers"]:
                raise ValueError("review has inconsistent capture time provenance")
            for value in review["capture_times"]:
                if datetime.fromisoformat(value).utcoffset() is None:
                    raise ValueError("review capture time must include a timezone")

        bindings = review["bindings"]
        if set(bindings) != {"oracle_build", "dependency_receipts", "genesis", "workload",
                            "capture_source", "mainnet_observation", "go_responses", "rs_responses"}:
            raise ValueError("review bindings must explicitly account for every admission artifact class")
        for name in ("oracle_build", "genesis", "workload", "capture_source", "mainnet_observation"):
            if bindings[name] is not None:
                validate_artifact(bindings[name], root)
        for name in ("dependency_receipts", "go_responses", "rs_responses"):
            if not isinstance(bindings[name], list):
                raise ValueError(f"review binding must be a list: {name}")
            for artifact in bindings[name]:
                validate_artifact(artifact, root)
        if binding_paths(bindings) != contract["bindings"]:
            raise ValueError(f"review artifact classes changed for issue {review['issue']}")

        blockers = review["admission_blockers"]
        if not blockers or len(blockers) != len(set(blockers)) or not set(blockers) <= ADMISSION_BLOCKERS:
            raise ValueError("review has missing, duplicate or unknown admission blockers")
        artifact_blockers = {
            "oracle_build": "missing-oracle-build-receipt",
            "dependency_receipts": "missing-dependency-receipts",
            "genesis": "missing-committed-genesis",
            "workload": "missing-committed-workload",
            "capture_source": "missing-capture-source",
            "mainnet_observation": "missing-complete-mainnet-observation",
            "go_responses": "missing-committed-go-response",
            "rs_responses": "missing-committed-rs-response",
        }
        paths = binding_paths(bindings)
        for name, blocker in artifact_blockers.items():
            if bool(paths[name]) == (blocker in blockers):
                raise ValueError(f"review has inconsistent {name} presence/blocker")
        binary = review["binary_sha256"]
        for implementation in ("go", "rs"):
            value = binary[implementation]
            blocker = f"missing-{implementation}-binary-hash"
            if value is None:
                if blocker not in blockers:
                    raise ValueError(f"review omits {implementation} binary without a blocker")
            elif not SHA256.fullmatch(value) or blocker in blockers:
                raise ValueError(f"review has inconsistent {implementation} binary identity")
        if binary["go"] is not None and bindings["oracle_build"] is None:
            raise ValueError("Go binary identity needs a committed build receipt")
        if binary["rs"] is not None or bindings["rs_responses"]:
            raise ValueError("unadmitted reviews must not manufacture Rust capture artifacts")
        if "missing-rs-binary-hash" not in blockers or "missing-committed-rs-response" not in blockers:
            raise ValueError("review does not state its missing Rust capture provenance")

        receipts = [item for item in [bindings["oracle_build"]] if item is not None]
        receipts += bindings["dependency_receipts"]
        receipt_text = "\n".join(artifact_text(item, root) for item in receipts)
        capture_text = receipt_text + "\n" + "\n".join(
            artifact_text(item, root) for item in bindings["go_responses"]
        )
        for value in review["capture_times"]:
            if value not in capture_text:
                raise ValueError(f"capture time is not bound by an artifact for issue {review['issue']}")
        profile_name = review["oracle_profile"]
        if profile_name is not None:
            profile = profiles[profile_name]
            for value in profile.values():
                values = value if isinstance(value, list) else [value]
                if any(str(item) not in receipt_text for item in values):
                    raise ValueError(f"oracle profile is not bound by receipts for issue {review['issue']}")
        else:
            if "missing-exact-oracle-identity" not in blockers and "missing-exact-dependency-commits" not in blockers:
                raise ValueError("partial oracle identity needs an explicit blocker")
            observed = review.get("observed_release", {})
            for key, value in observed.items():
                if key in policy["oracle"] and policy["oracle"][key] != value:
                    raise ValueError(f"observed release disagrees with policy: {key}")
                values = value if isinstance(value, list) else [value]
                if any(str(item) not in receipt_text for item in values):
                    raise ValueError(f"observed release is not bound by receipts for issue {review['issue']}")
        if binary["go"] is not None and binary["go"] not in receipt_text:
            raise ValueError("Go binary hash is not bound by its receipt")

        state = review["matched_state"]
        for implementation in ("go", "rs"):
            value = state[implementation]
            blocker = f"missing-{implementation}-matched-state"
            if value is None:
                if blocker not in blockers:
                    raise ValueError(f"review omits {implementation} matched state without a blocker")
                continue
            if blocker in blockers or not value["chain_id"] or value["height"] < 1:
                raise ValueError(f"review has inconsistent {implementation} matched state")
            if not re.fullmatch(r"[0-9a-fA-F]{64}", value["app_hash"]):
                raise ValueError(f"review has invalid {implementation} app hash")
            validate_artifact(value["source"], root)
        if state["rs"] is not None or "missing-rs-matched-state" not in blockers:
            raise ValueError("review must not claim an unrecorded Rust matched state")

        comparison = review["comparison"]
        if comparison["outcome"] not in REVIEW_OUTCOMES:
            raise ValueError("unknown reviewed comparison outcome")
        if not isinstance(comparison["normalizations"], list) or not isinstance(comparison["exclusions"], list):
            raise ValueError("review comparison must list normalizations and exclusions explicitly")
        if comparison["outcome"] != "runtime-exact-unrecorded" and "comparison-not-exact" not in blockers:
            raise ValueError("non-exact review lacks its comparison blocker")

        if not review["endpoints"] or not review["regressions"]:
            raise ValueError("review must bind endpoints and exact regression selectors")
        endpoint_set = tuple(sorted((item["transport"], item["endpoint"])
                                    for item in review["endpoints"]))
        if endpoint_set != contract["endpoints"] or len(endpoint_set) != len(review["endpoints"]):
            raise ValueError(f"review endpoint set changed for issue {review['issue']}")
        for endpoint in review["endpoints"]:
            if (endpoint["transport"], endpoint["endpoint"]) not in source_identities:
                raise ValueError(f"review endpoint absent from source inventory: {endpoint}")
        regression_set = tuple(sorted((item["runner"], item["package"], item["target"],
                                       item["filter"], item["source"])
                                      for item in review["regressions"]))
        if regression_set != contract["regressions"] or len(regression_set) != len(review["regressions"]):
            raise ValueError(f"review regression selector set changed for issue {review['issue']}")
        for test in review["regressions"]:
            if set(test) != {"runner", "package", "target", "filter", "source", "source_sha256"}:
                raise ValueError("reviewed regression must bind its exact source hash")
            if test["runner"] not in {"cargo", "python-unittest"}:
                raise ValueError("unknown reviewed regression runner")
            if not all(re.fullmatch(r"[A-Za-z0-9_.:-]+", test[key])
                       for key in ("package", "target", "filter")):
                raise ValueError("invalid reviewed regression selector")
            test_source = validate_artifact(
                {"path": test["source"], "sha256": test["source_sha256"]}, root)
            function = re.split(r"::|\.", test["filter"])[-1]
            declaration = r"\bdef\s+" if test["runner"] == "python-unittest" else r"\b(?:async\s+)?fn\s+"
            if not re.search(declaration + re.escape(function) + r"\s*\(", test_source.read_text()):
                raise ValueError("reviewed regression selector function absent from source")
        validate_review_semantics(review, root)
    return reviews


def scope_module():
    spec = importlib.util.spec_from_file_location("rpc_scope_audit", POLICY.with_name("scope_audit.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def inventory(policy):
    if policy["schema_version"] != 1 or set(v["issue"] for v in policy["behaviors"].values()) != set(range(182, 193)):
        raise ValueError("policy must account for every tracked behavior issue #182–192")
    reviews = validate_reviews(read_json(checked_path(ROOT, policy["reviewed_evidence"])), policy)
    scoped = scope_module().validate(read_json(checked_path(ROOT, policy["scoped_evidence"])), ROOT)
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
            "reviewed_not_admitted": reviews,
            "scoped_current_release": scoped,
            "endpoints": declared, "behaviors": policy["behaviors"],
            "comparison_policies": policy["comparison_policies"], "known_mismatches": policy["known_mismatches"],
            "exclusions": policy["exclusions"], "legacy_fixtures": legacy_fixtures()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path, help="Write source/evidence inventory as JSON")
    parser.add_argument("--run-current", action="store_true", help="Run only admitted current-release regression selectors (no capture/network)")
    parser.add_argument("--verify-scoped", action="store_true", help="Run hash-bound offline verifiers for scoped current-release packages (no Cargo or capture)")
    args = parser.parse_args()
    try:
        report = inventory(read_json(POLICY))
        if args.json:
            args.json.write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['endpoints'])} source endpoint candidates; {report['current_evidence_manifests']} admitted current-release manifests; {len(report['legacy_fixtures'])} regression-only fixtures")
        print(f"{len(report.get('scoped_current_release', {}).get('packages', []))} scoped current-release packages; exact parity admission remains separate")
        print("Provider readiness: NOT ESTABLISHED (see #181 and policy.json)")
        if args.verify_scoped:
            scope_module().verify_scoped(report["scoped_current_release"], ROOT)
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
