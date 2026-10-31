#!/usr/bin/env python3
"""Freeze exact build, source, input and capture hashes after both campaigns."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess


APP_MODULE = "github.com/celestiaorg/celestia-app/v10@v10.5.0"
APP_COMMIT = "6013e758d5dceb75827ad191c8d739d7c43cfa04"
CORE_MODULE = "github.com/celestiaorg/celestia-core@v0.42.3"
CORE_COMMIT = "a5537d84a008d566abc503d2ea41e30f2301403d"

SOURCE_NAMES = [
    "conformance/grpc-lifecycle/README.md",
    "conformance/grpc-lifecycle/record.sh",
    "conformance/grpc-lifecycle/build-rs.sh",
    "conformance/grpc-lifecycle/resources.py",
    "conformance/grpc-lifecycle/freeze.py",
    "conformance/grpc-lifecycle/verify.py",
    "conformance/grpc-lifecycle/test_verify.py",
    "conformance/grpc-lifecycle/recorder/main.go",
    "conformance/grpc-lifecycle/recorder/main_test.go",
    "conformance/grpc-lifecycle/recorder/go.mod",
    "conformance/grpc-lifecycle/recorder/go.sum",
    "crates/tier-4-runtime/rpc/src/grpc.rs",
    "crates/tier-4-runtime/grpc-transport/src/lib.rs",
    "crates/tier-5-node/celestia-rs/src/rpc_service.rs",
    "Cargo.lock",
]
INPUT_NAMES = [
    "go-quota-0-config.toml", "go-quota-2-config.toml",
    "rs-quota-0-config.toml", "rs-quota-2-config.toml",
    "go-app.toml", "rs-app.toml", "genesis.json",
]
CORE_CAPTURE_NAMES = [
    "go-quota-0.json", "go-quota-2.json",
    "rs-quota-0.json", "rs-quota-2.json",
]
SDK_CAPTURE_NAMES = ["go-sdk.json", "rs-sdk.json"]
CAPTURE_NAMES = [
    "go-quota-0.json", "go-sdk.json", "go-quota-2.json",
    "rs-quota-0.json", "rs-sdk.json", "rs-quota-2.json",
]
RESOURCE_NAMES = [
    "go-quota-0-resources.json", "go-sdk-resources.json", "go-quota-2-resources.json",
    "rs-quota-0-resources.json", "rs-sdk-resources.json", "rs-quota-2-resources.json",
]
BUILD_NAMES = [
    "go-build-info.txt", "go-core-build-info.txt", "recorder-build-info.txt",
    "rs-build-info.json", "go-app-build.json", "go-core-build.json",
    "go-gzip-symbols.txt",
]
OBSERVATION_NAMES = ["mainnet-observation.json"]
SOURCE_SUPPORT_PREFIX = "capture-source"
SOURCE_SNAPSHOT_PREFIX = f"{SOURCE_SUPPORT_PREFIX}/snapshot"
SOURCE_COMMIT_NAME = f"{SOURCE_SUPPORT_PREFIX}/commit.object"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked(*args: str) -> str:
    return subprocess.check_output(args, text=True).strip()


def git_bytes(repo: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(repo), *args])


def freeze_capture_source(repo: Path, capture: Path, source_commit: str) -> list[str]:
    """Export the signed commit graph needed by the offline verifier."""
    support = capture / SOURCE_SUPPORT_PREFIX
    snapshot = capture / SOURCE_SNAPSHOT_PREFIX
    trees = support / "trees"
    assert not support.exists(), f"capture source support already exists: {support}"
    snapshot.mkdir(parents=True)
    trees.mkdir()

    commit_object = git_bytes(repo, "cat-file", "commit", source_commit)
    (capture / SOURCE_COMMIT_NAME).write_bytes(commit_object)

    tree_oids = {checked("git", "-C", str(repo), "rev-parse", f"{source_commit}^{{tree}}")}
    for name in SOURCE_NAMES:
        contents = git_bytes(repo, "show", f"{source_commit}:{name}")
        destination = snapshot / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(contents)
        parts = Path(name).parts[:-1]
        for depth in range(1, len(parts) + 1):
            directory = "/".join(parts[:depth])
            oid = checked("git", "-C", str(repo), "rev-parse",
                          f"{source_commit}:{directory}")
            assert checked("git", "-C", str(repo), "cat-file", "-t", oid) == "tree"
            tree_oids.add(oid)

    tree_names = []
    for oid in sorted(tree_oids):
        name = f"{SOURCE_SUPPORT_PREFIX}/trees/{oid}.tree"
        (capture / name).write_bytes(git_bytes(repo, "cat-file", "tree", oid))
        tree_names.append(name)
    snapshot_names = [f"{SOURCE_SNAPSHOT_PREFIX}/{name}" for name in SOURCE_NAMES]
    return [SOURCE_COMMIT_NAME, *snapshot_names, *tree_names]


def sanitized_go_build_info(binary: Path, label: str) -> str:
    lines = checked("go", "version", "-m", str(binary)).splitlines()
    assert lines and ": go1." in lines[0] and "/" not in label
    lines[0] = f"{label}: {lines[0].rsplit(': ', 1)[1]}"
    return "\n".join(lines) + "\n"


def go_source_audit(repo: Path) -> dict:
    cache = Path(checked("go", "env", "GOMODCACHE"))
    files = {
        "go_core_keepalive": cache / "github.com/celestiaorg/celestia-core@v0.42.3/rpc/grpc/client_server.go",
        "grpc_go_setup_timeout": cache / "google.golang.org/grpc@v1.83.2/server.go",
        "grpc_go_initial_settings": cache / "google.golang.org/grpc@v1.83.2/internal/transport/http2_server.go",
        "grpc_go_keepalive_defaults": cache / "google.golang.org/grpc@v1.83.2/internal/transport/defaults.go",
        "cosmos_sdk_grpc_server": cache / "github.com/celestiaorg/cosmos-sdk@v0.52.12/server/grpc/server.go",
    }
    for path in files.values():
        assert path.is_file(), path
    rpc_source = (repo / "crates/tier-4-runtime/rpc/src/grpc.rs").read_text()
    service_source = (repo / "crates/tier-5-node/celestia-rs/src/rpc_service.rs").read_text()
    assert "keep_alive_interval" not in rpc_source
    assert "keep_alive_timeout" not in rpc_source
    assert "axum::serve(listener, router).await" in service_source
    return {
        "go_core_keepalive": {
            "module": CORE_MODULE, "source_commit": CORE_COMMIT,
            "path": "rpc/grpc/client_server.go", "sha256": digest(files["go_core_keepalive"]),
            "time_seconds": 30, "timeout_seconds": 10,
            "permit_without_stream": True, "minimum_client_ping_seconds": 10,
        },
        "grpc_go_setup_timeout": {
            "module": "google.golang.org/grpc@v1.83.2",
            "path": "server.go", "sha256": digest(files["grpc_go_setup_timeout"]),
            "default_seconds": 120,
        },
        "grpc_go_initial_settings": {
            "module": "google.golang.org/grpc@v1.83.2",
            "path": "internal/transport/http2_server.go",
            "sha256": digest(files["grpc_go_initial_settings"]),
            "first_frame": "SETTINGS", "max_frame_size": 16_384,
        },
        "grpc_go_keepalive_defaults": {
            "module": "google.golang.org/grpc@v1.83.2",
            "path": "internal/transport/defaults.go",
            "sha256": digest(files["grpc_go_keepalive_defaults"]),
            "server_time_seconds": 7200, "server_timeout_seconds": 20,
            "minimum_client_ping_seconds": 300,
        },
        "cosmos_sdk_grpc_server": {
            "module": "github.com/celestiaorg/cosmos-sdk@v0.52.12",
            "path": "server/grpc/server.go", "sha256": digest(files["cosmos_sdk_grpc_server"]),
            "only_operator_transport_settings": ["max-recv-msg-size", "max-send-msg-size"],
        },
        "rs_core_keepalive": {
            "path": "crates/tier-4-runtime/rpc/src/grpc.rs",
            "sha256": digest(repo / "crates/tier-4-runtime/rpc/src/grpc.rs"),
            "finding": "absent: HTTP/2 builder configures no keepalive interval or timeout",
        },
        "rs_setup_deadline": {
            "paths": [
                "crates/tier-4-runtime/rpc/src/grpc.rs",
                "crates/tier-5-node/celestia-rs/src/rpc_service.rs",
            ],
            "sha256": {
                name: digest(repo / name) for name in [
                    "crates/tier-4-runtime/rpc/src/grpc.rs",
                    "crates/tier-5-node/celestia-rs/src/rpc_service.rs",
                ]
            },
            "finding": "absent: core and SDK listeners configure no HTTP/2 setup deadline",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--observation", type=Path, required=True)
    parser.add_argument("--go-bin", type=Path, required=True)
    parser.add_argument("--go-core-bin", type=Path, required=True)
    parser.add_argument("--go-app-build", type=Path, required=True)
    parser.add_argument("--go-core-build", type=Path, required=True)
    parser.add_argument("--rs-bin", type=Path, required=True)
    parser.add_argument("--rs-build-receipt", type=Path, required=True)
    parser.add_argument("--recorder-bin", type=Path, required=True)
    args = parser.parse_args()
    repo = args.repo.resolve()
    capture = args.capture.resolve()
    assert capture.is_dir()

    app = json.loads(args.go_app_build.read_text())
    core = json.loads(args.go_core_build.read_text())
    assert app["module"] == APP_MODULE and app["source_commit"] == APP_COMMIT
    assert core["module"] == CORE_MODULE and core["source_commit"] == CORE_COMMIT
    assert app["compiler"] == core["compiler"] == "go version go1.26.9 linux/amd64"
    assert app["binary_sha256"] == digest(args.go_bin)
    assert core["binary_sha256"] == digest(args.go_core_bin)

    for name in SOURCE_NAMES:
        assert (repo / name).is_file(), name
    for name in INPUT_NAMES + CAPTURE_NAMES + RESOURCE_NAMES:
        assert (capture / name).is_file(), name

    observation_name = "mainnet-observation.json"
    (capture / observation_name).write_bytes(args.observation.read_bytes())
    (capture / "go-app-build.json").write_bytes(args.go_app_build.read_bytes())
    (capture / "go-core-build.json").write_bytes(args.go_core_build.read_bytes())
    go_build_info = sanitized_go_build_info(args.go_bin, "celestia-appd")
    go_core_build_info = sanitized_go_build_info(args.go_core_bin, "cometbft")
    recorder_build_info = sanitized_go_build_info(args.recorder_bin, "grpc-lifecycle-recorder")
    (capture / "go-build-info.txt").write_text(go_build_info)
    (capture / "go-core-build-info.txt").write_text(go_core_build_info)
    (capture / "recorder-build-info.txt").write_text(recorder_build_info)
    symbols = [line for line in checked("go", "tool", "nm", str(args.go_bin)).splitlines()
               if "google.golang.org/grpc/encoding/gzip" in line]
    assert any("gzip.init" in line for line in symbols)
    (capture / "go-gzip-symbols.txt").write_text("\n".join(symbols) + "\n")
    source_commit = checked("git", "-C", str(repo), "rev-parse", "HEAD")
    subprocess.run(["git", "-C", str(repo), "verify-commit", source_commit], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    assert checked("git", "-C", str(repo), "status", "--porcelain") == ""
    source_support_names = freeze_capture_source(repo, capture, source_commit)
    rs_info = json.loads(args.rs_build_receipt.read_text())
    assert args.rs_build_receipt.resolve() == Path(str(args.rs_bin.resolve()) + ".build.json")
    expected_rs_keys = {
        "schema_version", "built_at", "binary_sha256", "source_commit",
        "source_commit_signature_verified", "cargo_lock_sha256", "rustc_vv",
        "cargo_version", "target", "build_command", "profile", "package",
        "binary", "locked", "default_features", "features", "version_long",
    }
    assert set(rs_info) == expected_rs_keys
    assert rs_info["schema_version"] == 1
    built_at = datetime.fromisoformat(rs_info["built_at"])
    assert built_at.tzinfo is not None and built_at <= datetime.now(timezone.utc)
    assert rs_info["binary_sha256"] == digest(args.rs_bin)
    assert rs_info["source_commit"] == source_commit
    assert rs_info["source_commit_signature_verified"] is True
    assert rs_info["cargo_lock_sha256"] == digest(repo / "Cargo.lock")
    assert rs_info["build_command"] == [
        "cargo", "build", "--locked", "--release", "--package",
        "celestia-rs", "--bin", "celestia-rs",
    ]
    assert rs_info["profile"] == "release" and rs_info["locked"] is True
    assert rs_info["package"] == rs_info["binary"] == "celestia-rs"
    assert rs_info["default_features"] is True
    assert rs_info["features"] == ["rocksdb", "jemalloc"]
    assert rs_info["target"] == "x86_64-unknown-linux-gnu"
    assert rs_info["version_long"] == checked(str(args.rs_bin), "version", "--long")
    assert f"commit: {source_commit}" in rs_info["version_long"].splitlines()
    (capture / "rs-build-info.json").write_bytes(args.rs_build_receipt.read_bytes())
    for name in BUILD_NAMES:
        assert (capture / name).is_file(), name

    captures = {name: json.loads((capture / name).read_text()) for name in CAPTURE_NAMES}
    expected_order = CAPTURE_NAMES
    starts = [datetime.fromisoformat(captures[name]["started_at"]) for name in expected_order]
    completes = [datetime.fromisoformat(captures[name]["completed_at"]) for name in expected_order]
    for index, name in enumerate(expected_order):
        assert starts[index] <= completes[index], name
        if index:
            assert completes[index - 1] <= starts[index], name
    assert built_at <= starts[0]
    campaigns = {}
    for name in CAPTURE_NAMES:
        value = captures[name]
        implementation = name.split("-", 1)[0]
        assert value["implementation"] == implementation
        if name in SDK_CAPTURE_NAMES:
            listener, quota, config = "sdk", None, f"{implementation}-app.toml"
        else:
            _, _, quota_text = name.removesuffix(".json").split("-")
            listener, quota = "core", int(quota_text)
            config = f"{implementation}-quota-{quota}-config.toml"
            assert value["quota"] == quota
        campaigns[name] = {
            "implementation": implementation,
            "listener": listener,
            "quota": quota,
            "endpoint": value["endpoint"],
            "config": config,
            "started_at": value["started_at"],
            "completed_at": value["completed_at"],
        }

    meta = {
        "schema_version": 1,
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "mainnet_observation": observation_name,
        "go_builds": {"app": app, "core": core},
        "rs_build": rs_info,
        "source_commit": source_commit,
        "recorder_binary_sha256": digest(args.recorder_bin),
        "inventory": {
            "sources": SOURCE_NAMES,
            "inputs": INPUT_NAMES,
            "captures": CAPTURE_NAMES,
            "resources": RESOURCE_NAMES,
            "build_receipts": BUILD_NAMES,
            "observations": OBSERVATION_NAMES,
            "source_support": source_support_names,
        },
        "campaign_order": CAPTURE_NAMES,
        "campaigns": campaigns,
        "source_audit": go_source_audit(repo),
        "source_sha256": {
            name: digest(capture / SOURCE_SNAPSHOT_PREFIX / name)
            for name in SOURCE_NAMES
        },
        "input_sha256": {name: digest(capture / name) for name in INPUT_NAMES},
        "artifact_sha256": {
            name: digest(capture / name)
            for name in (CAPTURE_NAMES + RESOURCE_NAMES + BUILD_NAMES + INPUT_NAMES
                         + OBSERVATION_NAMES + source_support_names)
        },
        "deviations": {
            "rs_request_compression": (
                "unsupported on core and SDK: RS returns Unimplemented before decompression; "
                "Go accepts gzip and applies each listener's receive limit to the decompressed message"
            ),
            "sdk_connection_quota": (
                "unsupported by both production app listeners: app.toml has no connection-quota setting"
            ),
            "core_message_size_configuration": (
                "unsupported by both core listeners: receive is fixed at 4 MiB and send at MaxInt32"
            ),
            "rs_core_keepalive": (
                "absent: Go core sends idle PING at 30s and closes after 10s without ACK; "
                "RS configures no HTTP/2 keepalive"
            ),
            "rs_handshake_deadline": (
                "absent: grpc-go closes a silent HTTP/2 setup at 120s; RS has no gRPC setup deadline"
            ),
            "transport_metadata": (
                "intentional: Go core uses application/grpc+proto; Go SDK and RS use "
                "application/grpc; RS adds RFC 1123 Date and advertises identity when rejecting gzip"
            ),
        },
    }
    (capture / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")


if __name__ == "__main__":
    main()
