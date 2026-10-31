#!/usr/bin/env python3
"""Audit scoped current-release packages without upgrading them to exact parity."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
REVIEWED_AUDIT_SHA256 = "98be8b4296ac79071c935defc53dd6ba232dc36abb5980526b57c0a7620551fb"
AUDIT = Path(__file__).with_name("scope-audit.json")
# The exact package/receipt/verifier set is reviewed code. A data edit cannot
# silently remove a campaign, swap its receipt class or skip its verifier.
CONTRACTS = {
    "reflection": (182, "normalized-runtime-comparison", "conformance/grpc-reflection/v10.5.0/meta.json", "conformance/grpc-reflection/README.md", None),
    "grpc-limits": (183, "unmatched-runtime-comparison", "conformance/grpc-limits/v10.5.0/meta.json", "conformance/grpc-limits/README.md", "conformance/grpc-limits/verify.py"),
    "runtime-identity": (184, "schema-only", "conformance/node-metadata/v10.5.0/consensus.json", "docs/rpc-version-identity.md", None),
    "simulation": (185, "normalized-runtime-comparison", "conformance/mainnet-rpc/simulation/metadata.json", "conformance/mainnet-rpc/simulation/README.md", None),
    "empty-query": (186, "runtime-exact-unrecorded", "conformance/mainnet-rpc/v10.5.0/empty-query-meta.json", "conformance/mainnet-rpc/empty-query/README.md", None),
    "live-transactions": (187, "normalized-runtime-comparison", "conformance/fixtures/devnet/v105-live-tx/capture/metadata.json", "conformance/mainnet-rpc/live-tx/README.md", "conformance/mainnet-rpc/tests/test_live_tx.py"),
    "populated-staking": (188, "normalized-runtime-comparison", "conformance/fixtures/devnet/v105-populated-staking/capture/metadata.json", "conformance/mainnet-rpc/populated-staking/README.md", "conformance/mainnet-rpc/tests/test_populated_staking.py"),
    "websocket-lifecycle": (190, "relational-go-plus-rust-regressions", "conformance/websocket-lifecycle/closure-review.json", "conformance/websocket-lifecycle/README.md", "conformance/websocket-lifecycle/verify-closure.py"),
    "rest-heights": (191, "rust-with-stand-in-only", "conformance/mainnet-rpc/rest-boundaries/v10.5.0/provenance.json", "conformance/mainnet-rpc/rest-boundaries/README.md", "conformance/mainnet-rpc/rest-boundaries/verify.py"),
    "rest-post": (191, "go-transports-only", "conformance/mainnet-rpc/rest-post-boundaries/v10.5.0/provenance.json", "conformance/mainnet-rpc/rest-post-boundaries/README.md", "conformance/mainnet-rpc/rest-post-boundaries/verify.py"),
    "provider-soak": (192, "bounded-mixed-node-soak", "conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca/provider-provenance.json", "conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca/README.md", "conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca/verify.py"),
    "comet-http": (191, "normalized-runtime-comparison", "conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/provenance.json", "conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/README.md", "conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/verify.py"),
    "proxy-tls-framing": (191, "normalized-runtime-comparison", "conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/provenance.json", "conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/README.md", "conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/verify.py"),
}
RUNTIME_CONTRACTS = {
    "reflection": (("crates/tier-4-runtime/grpc-reflection/tests/transport.rs", "sdk_v1alpha_matches_go_socket_streams"), ("crates/tier-4-runtime/grpc-reflection/tests/transport.rs", "core_v1_and_v1alpha_match_go_socket_streams")),
    "grpc-limits": (("crates/tier-5-node/celestia-rs/tests/grpc_limits.rs", "custom_app_limits_reach_module_and_block_api_and_core_capacity_recovers"),),
    "runtime-identity": (("crates/tier-5-node/celestia-rs/tests/runtime_metadata.rs", "production_listeners_report_build_and_compatibility_separately"),),
    "simulation": (("crates/devnet-conformance/tests/simulation_errors.rs", "current_release_simulation_errors"),),
    "empty-query": (("crates/tier-4-runtime/rpc/tests/empty_query.rs", "p2p_empty_query_matches_current_go_over_reusable_http_and_websocket"),),
    "live-transactions": (("crates/tier-5-node/celestia-rs/tests/live_tx_transports.rs", "admitted_live_transactions_match_rust_production_transports"),),
    "populated-staking": (("crates/devnet-conformance/tests/populated_staking_transports.rs", "admitted_populated_inputs_match_all_rust_transports"),),
    "websocket-lifecycle": (
        ("crates/tier-4-runtime/rpc/src/server.rs", "websocket_write_is_bounded"),
        ("crates/tier-4-runtime/rpc/tests/corpus.rs", "websocket_disconnects_reclaim_subscriptions_and_allow_reconnect"),
        ("crates/tier-4-runtime/rpc/tests/corpus.rs", "websocket_origins_match_go_and_deliver_events"),
    ),
    "rest-heights": (("crates/tier-4-runtime/api-server/src/rest.rs", "go_rest_height_boundaries"),),
    "rest-post": (),
    "provider-soak": (),
    "comet-http": (("crates/tier-4-runtime/rpc/tests/http_boundaries.rs", "tagged_go_comet_http_boundaries_match_over_real_sockets"),),
    "proxy-tls-framing": (
        ("crates/tier-4-runtime/rpc/tests/http_boundaries.rs", "tagged_go_proxy_and_framing_boundaries_match_or_are_bounded"),
        ("crates/tier-4-runtime/rpc/tests/http_boundaries.rs", "tagged_go_tls_and_config_boundaries_match_or_are_bounded"),
        ("crates/tier-5-node/celestia-rs/src/rpc_service.rs", "tls_public_rpc_keeps_app_get_latest_block_on_private_loopback"),
    ),
}
TRANSPORT_CONTRACTS = {
    "reflection": {"core-grpc", "cosmos-grpc"}, "grpc-limits": {"core-grpc", "cosmos-grpc"},
    "runtime-identity": {"comet-uri", "cosmos-grpc", "rest"}, "simulation": {"cosmos-grpc"},
    "empty-query": {"comet-uri", "comet-jsonrpc", "comet-websocket"},
    "live-transactions": {"comet-jsonrpc", "core-grpc", "cosmos-grpc", "rest"},
    "populated-staking": {"comet-jsonrpc", "cosmos-grpc", "rest"},
    "websocket-lifecycle": {"comet-websocket"}, "rest-heights": {"rest"}, "rest-post": {"rest"},
    "comet-http": {"comet-uri", "comet-jsonrpc"},
    "proxy-tls-framing": {"comet-uri", "comet-jsonrpc", "comet-websocket", "rest"},
    "provider-soak": {"comet-uri", "comet-jsonrpc", "comet-websocket", "core-grpc", "cosmos-grpc", "rest"},
}
TRANSPORTS = {"comet-uri", "comet-jsonrpc", "comet-websocket", "core-grpc", "cosmos-grpc", "rest"}


def artifact(root, binding):
    if set(binding) != {"path", "sha256"}:
        raise ValueError("scoped artifact must bind path and SHA-256")
    path = root / binding["path"]
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()) or not path.is_file():
        raise ValueError("missing, symlinked or out-of-tree scoped artifact")
    if not re.fullmatch(r"[0-9a-f]{64}", binding["sha256"]) or hashlib.sha256(path.read_bytes()).hexdigest() != binding["sha256"]:
        raise ValueError(f"scoped artifact hash mismatch: {binding['path']}")
    return path


def validate(audit, root=ROOT):
    if set(audit) != {"schema_version", "audited_main", "provider_ready", "packages", "behaviors"}:
        raise ValueError("unknown scoped audit fields")
    if audit["schema_version"] != 1 or audit["provider_ready"] is not False:
        raise ValueError("scoped evidence cannot establish provider readiness")
    if not re.fullmatch(r"[0-9a-f]{40}", audit["audited_main"]):
        raise ValueError("scoped audit needs its exact merged source baseline")
    packages = audit["packages"]
    if len(packages) != len(CONTRACTS) or {row["id"] for row in packages} != set(CONTRACTS):
        raise ValueError("scoped package inventory must be complete without duplicates")
    for row in packages:
        if set(row) != {"id", "issue", "comparison", "receipt", "documentation", "verifier", "transports", "scope", "normalizations", "known_mismatches", "exclusions", "provenance_limits", "runtime_regressions"}:
            raise ValueError("unknown scoped package fields")
        issue, comparison, receipt, documentation, verifier = CONTRACTS[row["id"]]
        if row["issue"] != issue or row["comparison"] != comparison:
            raise ValueError("scoped package changed issue or comparison class")
        if row["receipt"]["path"] != receipt or row["documentation"]["path"] != documentation:
            raise ValueError("scoped receipt/documentation class changed")
        artifact(root, row["receipt"])
        artifact(root, row["documentation"])
        if (row["verifier"] is None) != (verifier is None):
            raise ValueError("scoped verifier cannot be added or omitted without review")
        if verifier is not None:
            if row["verifier"]["path"] != verifier:
                raise ValueError("scoped verifier path changed")
            artifact(root, row["verifier"])
        if not row["scope"] or not row["provenance_limits"] or not row["exclusions"]:
            raise ValueError("scoped package must state scope, provenance limits and exclusions")
        if not row["transports"] or len(row["transports"]) != len(set(row["transports"])) or set(row["transports"]) != TRANSPORT_CONTRACTS[row["id"]]:
            raise ValueError("invalid scoped transport inventory")
        if not isinstance(row["normalizations"], list) or not isinstance(row["known_mismatches"], list):
            raise ValueError("scoped comparison policies must remain separate")
        actual = [(item["source"]["path"], item["function"]) for item in row["runtime_regressions"]]
        if len(actual) != len(set(actual)) or set(actual) != set(RUNTIME_CONTRACTS[row["id"]]):
            raise ValueError("scoped runtime selector set changed")
        for regression in row["runtime_regressions"]:
            if set(regression) != {"source", "function"}:
                raise ValueError("unknown scoped runtime selector fields")
            path = artifact(root, regression["source"])
            function = regression["function"].rsplit("::", 1)[-1]
            if not re.fullmatch(r"[A-Za-z0-9_:]+", regression["function"]) or not re.search(r"\b(?:async\s+)?fn\s+" + re.escape(function) + r"\s*\(", path.read_text()):
                raise ValueError("scoped runtime selector does not name a source function")
    checklist = audit["behaviors"]
    if len(checklist) != 11 or {row["issue"] for row in checklist} != set(range(182, 193)):
        raise ValueError("scoped checklist must account for every issue #182–#192")
    for row in checklist:
        if set(row) != {"issue", "packages", "remaining_gaps"}:
            raise ValueError("unknown scoped behavior checklist fields")
        expected = sorted(p["id"] for p in packages if p["issue"] == row["issue"])
        if row["packages"] != expected or not row["remaining_gaps"]:
            raise ValueError("behavior checklist must bind all packages and remaining gaps")
    canonical = json.dumps(audit, sort_keys=True, separators=(",", ":")).encode()
    if hashlib.sha256(canonical).hexdigest() != REVIEWED_AUDIT_SHA256:
        raise ValueError("canonical reviewed audit content changed; review baseline, scope, deviations and gaps")
    return audit


def verify_scoped(audit, root=ROOT):
    # Sources have been hash-checked. Commands are fixed Python file invocations;
    # no node, Cargo build, new oracle capture or network request runs here.
    validate(audit, root)
    for row in audit["packages"]:
        if row["verifier"] is not None:
            subprocess.run([sys.executable, str(root / row["verifier"]["path"])], cwd=root, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="Run registered offline corpus verifiers")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    try:
        audit = validate(json.loads(AUDIT.read_text()))
        if args.verify:
            verify_scoped(audit)
        if args.json:
            args.json.write_text(json.dumps(audit, indent=2) + "\n")
        print(f"{len(audit['packages'])} scoped current-release packages audited; exact parity admission remains separate; provider readiness NOT ESTABLISHED")
    except (KeyError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Scoped RPC evidence audit failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
