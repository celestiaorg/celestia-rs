#!/usr/bin/env python3
"""Verify bounded gRPC lifecycle captures and their exact provenance."""

from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
from email.utils import format_datetime, parsedate_to_datetime
import hashlib
import ipaddress
import json
from pathlib import Path
import re
import subprocess
import tomllib


GO_APP = "github.com/celestiaorg/celestia-app/v10@v10.5.0"
GO_APP_COMMIT = "6013e758d5dceb75827ad191c8d739d7c43cfa04"
GO_CORE = "github.com/celestiaorg/celestia-core@v0.42.3"
GO_CORE_COMMIT = "a5537d84a008d566abc503d2ea41e30f2301403d"
APP_SUM = "h1:CEKeiFSFRclu85ozJo5Gdk71H9am3wyEJJ8pMWqBNkc="
CORE_SUM = "h1:kdXKlSiEqEWl87EyxNB9ryx1aQJq/8RyBdgfYy2rdYM="
SDK_SUM = "h1:muIDlBLLXeuKiIrGA/t+k9ZLpjjRVs460XRzwh1dgqU="
GRPC_SUM = "h1:EManeRomTObA0BU7I8vXgg/78uE5MJ9M8B39EX2WscU="

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
INVENTORY = {
    "inputs": INPUT_NAMES,
    "captures": CAPTURE_NAMES,
    "resources": RESOURCE_NAMES,
    "build_receipts": BUILD_NAMES,
    "observations": OBSERVATION_NAMES,
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_bytes(root: Path, name: str, commit: str | None = None) -> bytes:
    if commit is None:
        return (root / name).read_bytes()
    assert re.fullmatch(r"[0-9a-f]{40}", commit)
    return subprocess.check_output(
        ["git", "-C", str(root), "show", f"{commit}:{name}"],
        stderr=subprocess.DEVNULL,
    )


def source_digest(root: Path, name: str, commit: str | None = None) -> str:
    return hashlib.sha256(source_bytes(root, name, commit)).hexdigest()


def load(path: Path) -> dict:
    value = json.loads(path.read_text())
    assert isinstance(value, dict), f"{path}: expected object"
    return value


def parse_go_build_info(path: Path) -> dict:
    lines = path.read_text().splitlines()
    assert lines and ": go1." in lines[0]
    binary_label, separator, compiler = lines[0].partition(": ")
    assert separator and binary_label and "/" not in binary_label and "\\" not in binary_label
    result: dict[str, object] = {
        "binary_label": binary_label,
        "compiler": compiler,
        "deps": {},
        "replacements": {},
        "build": {},
    }
    previous_dep = None
    for raw in lines[1:]:
        parts = raw.lstrip("\t").split("\t")
        if not parts or not parts[0]:
            continue
        kind = parts[0]
        if kind == "path":
            result["path"] = parts[1]
        elif kind == "mod":
            result["module"] = parts[1:]
        elif kind == "dep":
            assert len(parts) >= 3
            previous_dep = parts[1]
            result["deps"][previous_dep] = parts[2:]
        elif kind == "=>":
            assert previous_dep is not None and len(parts) >= 3
            result["replacements"][previous_dep] = parts[1:]
        elif kind == "build":
            key, separator, value = parts[1].partition("=")
            assert separator
            result["build"][key] = value
    assert "path" in result and "module" in result
    return result


def verify_rs_receipt(
    receipt: dict,
    source_commit: str,
    cargo_lock_sha256: str,
    binary_sha256: str | None = None,
    version_long: str | None = None,
) -> None:
    assert set(receipt) == {
        "schema_version", "built_at", "binary_sha256", "source_commit",
        "source_commit_signature_verified", "cargo_lock_sha256", "rustc_vv",
        "cargo_version", "target", "build_command", "profile", "package",
        "binary", "locked", "default_features", "features", "version_long",
    }
    assert receipt["schema_version"] == 1
    built_at = datetime.fromisoformat(receipt["built_at"])
    assert built_at.tzinfo is not None and built_at <= datetime.now(timezone.utc)
    assert receipt["source_commit"] == source_commit
    assert receipt["source_commit_signature_verified"] is True
    assert receipt["cargo_lock_sha256"] == cargo_lock_sha256
    assert re.fullmatch(r"[0-9a-f]{40}", source_commit)
    assert re.fullmatch(r"[0-9a-f]{64}", receipt["binary_sha256"])
    if binary_sha256 is not None:
        assert receipt["binary_sha256"] == binary_sha256
    assert "release: 1.98.0" in receipt["rustc_vv"]
    assert "host: x86_64-unknown-linux-gnu" in receipt["rustc_vv"]
    assert receipt["cargo_version"].startswith("cargo 1.98.0 ")
    assert receipt["target"] == "x86_64-unknown-linux-gnu"
    assert receipt["build_command"] == [
        "cargo", "build", "--locked", "--release", "--package",
        "celestia-rs", "--bin", "celestia-rs",
    ]
    assert receipt["profile"] == "release"
    assert receipt["package"] == receipt["binary"] == "celestia-rs"
    assert receipt["locked"] is True and receipt["default_features"] is True
    assert receipt["features"] == ["rocksdb", "jemalloc"]
    lines = receipt["version_long"].splitlines()
    assert f"commit: {source_commit}" in lines
    assert "build_profile: release" in lines
    assert "name: celestia-rs" in lines
    rustc_version = receipt["rustc_vv"].splitlines()[0]
    assert f"toolchain: {rustc_version} {receipt['target']}" in lines
    if version_long is not None:
        assert receipt["version_long"] == version_long


def verify_builds(root: Path, meta: dict, source_root: Path,
                  source_commit: str | None = None) -> None:
    app = meta["go_builds"]["app"]
    core = meta["go_builds"]["core"]
    receipt_keys = {
        "schema_version", "module", "source_commit", "module_sum", "compiler",
        "build_tags", "ldflags", "package", "buildvcs", "goflags", "target",
        "binary_sha256",
    }
    assert set(app) == set(core) == receipt_keys
    assert app == load(root / "go-app-build.json")
    assert core == load(root / "go-core-build.json")
    assert app == {
        **app,
        "module": GO_APP,
        "source_commit": GO_APP_COMMIT,
        "module_sum": APP_SUM,
        "compiler": "go version go1.26.9 linux/amd64",
        "build_tags": ["ledger"],
        "package": "./cmd/celestia-appd",
        "buildvcs": False,
        "goflags": "-mod=mod",
    }
    assert core == {
        **core,
        "module": GO_CORE,
        "source_commit": GO_CORE_COMMIT,
        "module_sum": CORE_SUM,
        "compiler": "go version go1.26.9 linux/amd64",
        "build_tags": [],
        "package": "./cmd/cometbft",
        "buildvcs": False,
        "goflags": "-mod=mod",
    }
    for build in (app, core):
        assert build["schema_version"] == 1
        assert re.fullmatch(r"[0-9a-f]{64}", build["binary_sha256"])
        assert build["target"] == {
            "CGO_ENABLED": "1", "GOAMD64": "v1", "GOARCH": "amd64",
            "GOARM": "", "GOEXPERIMENT": "", "GOFIPS140": "off", "GOOS": "linux",
        }
    assert "Version=10.5.0" in app["ldflags"]
    assert f"Commit={GO_APP_COMMIT}" in app["ldflags"]
    assert "BuildTags=ledger" in app["ldflags"]
    assert core["ldflags"] == ""

    app_info = parse_go_build_info(root / "go-build-info.txt")
    assert app_info["binary_label"] == "celestia-appd"
    assert app_info["compiler"] == "go1.26.9"
    assert app_info["path"] == "github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd"
    assert app_info["module"][:2] == ["github.com/celestiaorg/celestia-app/v10", "(devel)"]
    assert app_info["replacements"]["github.com/cometbft/cometbft"] == [
        "github.com/celestiaorg/celestia-core", "v0.42.3", CORE_SUM,
    ]
    assert app_info["replacements"]["github.com/cosmos/cosmos-sdk"] == [
        "github.com/celestiaorg/cosmos-sdk", "v0.52.12", SDK_SUM,
    ]
    assert app_info["deps"]["google.golang.org/grpc"] == ["v1.83.2", GRPC_SUM]
    assert app_info["build"]["GOOS"] == "linux" and app_info["build"]["GOARCH"] == "amd64"

    core_info = parse_go_build_info(root / "go-core-build-info.txt")
    assert core_info["binary_label"] == "cometbft"
    assert core_info["compiler"] == "go1.26.9"
    assert core_info["path"] == "github.com/cometbft/cometbft/cmd/cometbft"
    assert core_info["module"][:2] == ["github.com/cometbft/cometbft", "(devel)"]
    assert core_info["deps"]["google.golang.org/grpc"] == ["v1.83.2", GRPC_SUM]

    recorder = parse_go_build_info(root / "recorder-build-info.txt")
    assert recorder["binary_label"] == "grpc-lifecycle-recorder"
    assert recorder["compiler"] == "go1.26.9"
    assert recorder["path"] == (
        "github.com/celestiaorg/celestia-rs/conformance/grpc-lifecycle"
    )
    assert recorder["deps"]["google.golang.org/grpc"] == ["v1.83.2", GRPC_SUM]
    assert re.fullmatch(r"[0-9a-f]{64}", meta["recorder_binary_sha256"])

    rs = load(root / "rs-build-info.json")
    assert rs == meta["rs_build"]
    verify_rs_receipt(rs, meta["source_commit"],
                      source_digest(source_root, "Cargo.lock", source_commit))
    subprocess.run(["git", "-C", str(source_root), "verify-commit", rs["source_commit"]],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    gzip_symbols = (root / "go-gzip-symbols.txt").read_text().splitlines()
    assert any("google.golang.org/grpc/encoding/gzip.init" in line
               for line in gzip_symbols)


def verify_inventory(root: Path, meta: dict, source_root: Path,
                     source_commit: str | None = None) -> None:
    expected = {"sources": SOURCE_NAMES, **INVENTORY}
    assert meta["inventory"] == expected
    classes = list(expected.values())
    flat = [name for names in classes for name in names]
    assert len(flat) == len(set(flat)), "inventory classes overlap"
    artifacts = INPUT_NAMES + CAPTURE_NAMES + RESOURCE_NAMES + BUILD_NAMES + OBSERVATION_NAMES
    assert set(meta["artifact_sha256"]) == set(artifacts)
    assert set(meta["input_sha256"]) == set(INPUT_NAMES)
    assert set(meta["source_sha256"]) == set(SOURCE_NAMES)
    actual = {str(path.relative_to(root)) for path in root.rglob("*") if path.is_file()}
    assert actual == set(artifacts) | {"meta.json"}
    for name, expected_digest in meta["artifact_sha256"].items():
        assert digest(root / name) == expected_digest, f"artifact hash mismatch: {name}"
    for name, expected_digest in meta["source_sha256"].items():
        assert source_digest(source_root, name, source_commit) == expected_digest, (
            f"source hash mismatch: {name}"
        )
    for name, expected_digest in meta["input_sha256"].items():
        assert digest(root / name) == expected_digest, f"input hash mismatch: {name}"


def endpoint(config: dict) -> str:
    value = config["rpc"]["grpc_laddr"]
    assert isinstance(value, str) and value.startswith("tcp://")
    address = value.removeprefix("tcp://")
    host, separator, port = address.rpartition(":")
    assert separator and 0 < int(port) <= 65535
    assert ipaddress.ip_address(host.strip("[]")).is_loopback
    return address


def loopback_listener(value: str) -> None:
    assert isinstance(value, str)
    address = value.removeprefix("tcp://")
    host, separator, port = address.rpartition(":")
    assert separator and 0 < int(port) <= 65535
    assert host and ipaddress.ip_address(host.strip("[]")).is_loopback, value


def sdk_endpoint(app: dict) -> str:
    value = app["grpc"]["address"]
    loopback_listener(value)
    return value.removeprefix("tcp://")


def verify_loopback_configs(config: dict, app: dict) -> None:
    assert config["p2p"]["pex"] is False
    listeners = [config["rpc"]["laddr"], config["rpc"]["grpc_laddr"],
                 config["p2p"]["laddr"]]
    if config["rpc"].get("pprof_laddr"):
        listeners.append(config["rpc"]["pprof_laddr"])
    for key in ("priv_validator_laddr", "priv_validator_grpc_laddr"):
        if config.get(key):
            listeners.append(config[key])
    instrumentation = config.get("instrumentation", {})
    if instrumentation.get("prometheus") is True:
        listeners.append(instrumentation["prometheus_listen_addr"])
    for section in ("api", "grpc", "grpc-web"):
        value = app.get(section, {})
        if value.get("enable") is True:
            listeners.append(value["address"])
    for value in listeners:
        loopback_listener(value)

    # Fail closed for wildcard bind strings anywhere, including a future
    # listener section that this schema-specific list does not yet know.
    def strings(value: object):
        if isinstance(value, dict):
            for child in value.values():
                yield from strings(child)
        elif isinstance(value, list):
            for child in value:
                yield from strings(child)
        elif isinstance(value, str):
            yield value
    for value in strings(config), strings(app):
        for item in value:
            assert not re.search(r"(?:^|://)0\.0\.0\.0:\d+$", item), item
            assert not re.search(r"(?:^|://)(?:\[::\]|::):\d+$", item), item


def verify_configs_and_order(root: Path, meta: dict, observation_time: datetime) -> None:
    assert meta["campaign_order"] == CAPTURE_NAMES
    assert set(meta["campaigns"]) == set(CAPTURE_NAMES)
    previous = observation_time
    rs_built_at = datetime.fromisoformat(meta["rs_build"]["built_at"])
    for name in CAPTURE_NAMES:
        implementation = name.split("-", 1)[0]
        app = tomllib.loads((root / f"{implementation}-app.toml").read_text())
        if name in SDK_CAPTURE_NAMES:
            listener, quota = "sdk", None
            config_name = f"{implementation}-app.toml"
            address = sdk_endpoint(app)
        else:
            _, _, quota_text = name.removesuffix(".json").split("-")
            listener, quota = "core", int(quota_text)
            config_name = f"{implementation}-quota-{quota}-config.toml"
            config = tomllib.loads((root / config_name).read_text())
            verify_loopback_configs(config, app)
            assert config["rpc"]["grpc_max_open_connections"] == quota
            address = endpoint(config)
        capture = load(root / name)
        campaign = meta["campaigns"][name]
        assert campaign == {
            "implementation": implementation,
            "listener": listener,
            "quota": quota,
            "endpoint": address,
            "config": config_name,
            "started_at": capture["started_at"],
            "completed_at": capture["completed_at"],
        }
        assert capture["endpoint"] == address
        started = datetime.fromisoformat(capture["started_at"])
        completed = datetime.fromisoformat(capture["completed_at"])
        assert rs_built_at <= started
        assert previous <= started <= completed
        assert started - observation_time <= timedelta(hours=24)
        maximum = timedelta(minutes=4 if name == "go-quota-2.json" else 2)
        assert completed - started <= maximum
        previous = completed
    captured = datetime.fromisoformat(meta["captured_at"])
    assert previous <= captured <= previous + timedelta(minutes=10)
    assert captured <= datetime.now(timezone.utc) + timedelta(minutes=5)

    for name in ("go-app.toml", "rs-app.toml"):
        app = tomllib.loads((root / name).read_text())["grpc"]
        assert app["enable"] is True
        assert int(app["max-recv-msg-size"]) == 64
        assert int(app["max-send-msg-size"]) == 4_096
    genesis = load(root / "genesis.json")
    assert genesis["chain_id"] == "rs-grpc-limits-1"


def verify_resources(
    resources: dict,
    capture: dict,
    implementation: str,
    quota: int | None,
    executable_sha256: str,
) -> None:
    assert set(resources) == {
        "schema_version", "implementation", "listener", "quota", "started_at",
        "completed_at", "poll_ms", "settle_ms", "monitor", "process_identity",
        "samples",
    }
    assert resources["schema_version"] == 1
    assert resources["implementation"] == implementation
    assert resources["listener"] == ("sdk" if quota is None else "core")
    assert resources["quota"] == quota
    assert resources["poll_ms"] == 250 and resources["settle_ms"] == 2_000
    monitor = resources["monitor"]
    assert set(monitor) == {
        "python_version", "system", "kernel_release", "clock_ticks_per_second",
        "page_size",
    }
    assert re.fullmatch(r"3\.\d+\.\d+", monitor["python_version"])
    assert monitor["system"] == "Linux" and monitor["kernel_release"]
    assert monitor["clock_ticks_per_second"] > 0 and monitor["page_size"] > 0
    identity = resources["process_identity"]
    assert set(identity) == {"initial", "final"}
    assert identity["initial"] == identity["final"]
    assert set(identity["initial"]) == {"pid", "start_ticks", "executable_sha256"}
    assert identity["initial"]["pid"] > 1 and identity["initial"]["start_ticks"] > 0
    assert identity["initial"]["executable_sha256"] == executable_sha256
    started = datetime.fromisoformat(resources["started_at"])
    completed = datetime.fromisoformat(resources["completed_at"])
    capture_started = datetime.fromisoformat(capture["started_at"])
    capture_completed = datetime.fromisoformat(capture["completed_at"])
    assert started.tzinfo and completed.tzinfo
    assert started <= capture_started <= capture_completed <= completed
    assert completed - started <= timedelta(minutes=5)
    samples = resources["samples"]
    assert isinstance(samples, list) and 3 <= len(samples) <= 2_000
    previous = -1
    for index, item in enumerate(samples):
        assert set(item) == {"elapsed_ms", "rss_kib", "threads", "fd_count"}
        assert previous <= item["elapsed_ms"] <= 300_000
        assert 0 < item["rss_kib"] <= 2 * 1024 * 1024
        assert 0 < item["threads"] <= 256
        assert 2 < item["fd_count"] <= 256
        if index and index < len(samples) - 1:
            # 250 ms requested cadence with 1.25 s scheduler tolerance.
            assert item["elapsed_ms"] - previous <= 1_500
        previous = item["elapsed_ms"]
    assert samples[0]["elapsed_ms"] <= 100
    elapsed = round((completed - started).total_seconds() * 1_000)
    assert abs(samples[-1]["elapsed_ms"] - elapsed) <= 1_000
    completed_offset = round((capture_completed - started).total_seconds() * 1_000)
    assert abs(samples[-2]["elapsed_ms"] - completed_offset) <= 1_000
    assert completed_offset + resources["settle_ms"] - 500 <= samples[-1]["elapsed_ms"]
    assert samples[-1]["elapsed_ms"] <= completed_offset + resources["settle_ms"] + 1_000
    first, final = samples[0], samples[-1]
    assert final["fd_count"] <= first["fd_count"] + 8
    assert final["threads"] <= first["threads"] + 8
    assert final["rss_kib"] <= first["rss_kib"] + 256 * 1024


def verify_resource_artifacts(root: Path, meta: dict) -> None:
    for capture_name in CAPTURE_NAMES:
        implementation = capture_name.split("-", 1)[0]
        quota = None
        if capture_name not in SDK_CAPTURE_NAMES:
            _, _, quota_text = capture_name.removesuffix(".json").split("-")
            quota = int(quota_text)
        resource_name = capture_name.removesuffix(".json") + "-resources.json"
        binary = (meta["go_builds"]["app"] if implementation == "go"
                  else meta["rs_build"])["binary_sha256"]
        verify_resources(load(root / resource_name), load(root / capture_name),
                         implementation, quota, binary)


def verify_source_audit(meta: dict, source_root: Path,
                        source_commit: str | None = None) -> None:
    audit = meta["source_audit"]
    assert set(audit) == {
        "go_core_keepalive", "grpc_go_setup_timeout", "grpc_go_initial_settings",
        "grpc_go_keepalive_defaults",
        "cosmos_sdk_grpc_server", "rs_core_keepalive", "rs_setup_deadline",
    }
    assert audit["go_core_keepalive"] == {
        "module": GO_CORE, "source_commit": GO_CORE_COMMIT,
        "path": "rpc/grpc/client_server.go",
        "sha256": "edc13288085bf9bec39efdc6ba22d866a484064b4ae7417e92a2a93a972b1d44",
        "time_seconds": 30, "timeout_seconds": 10,
        "permit_without_stream": True, "minimum_client_ping_seconds": 10,
    }
    assert audit["grpc_go_setup_timeout"] == {
        "module": "google.golang.org/grpc@v1.83.2", "path": "server.go",
        "sha256": "a09f192cb707172d7b9004813f300697e0eab8eb781389978683460ed0ddc49e",
        "default_seconds": 120,
    }
    assert audit["grpc_go_initial_settings"] == {
        "module": "google.golang.org/grpc@v1.83.2",
        "path": "internal/transport/http2_server.go",
        "sha256": "84ad3bde6a86be6ab86af02adc250ae04418fb24aafa628292141fb6fc9ed093",
        "first_frame": "SETTINGS", "max_frame_size": 16_384,
    }
    assert audit["grpc_go_keepalive_defaults"] == {
        "module": "google.golang.org/grpc@v1.83.2",
        "path": "internal/transport/defaults.go",
        "sha256": "ca8b8cee5cb10ea5e3396a4e058f31d3a7d7f8e46ce95a82d84895fc34465888",
        "server_time_seconds": 7200, "server_timeout_seconds": 20,
        "minimum_client_ping_seconds": 300,
    }
    assert audit["cosmos_sdk_grpc_server"] == {
        "module": "github.com/celestiaorg/cosmos-sdk@v0.52.12",
        "path": "server/grpc/server.go",
        "sha256": "f24db7f5e784a4b40a63fbdd9b4a45ebb3e16d61952e62b87abcfa4d3657df5d",
        "only_operator_transport_settings": ["max-recv-msg-size", "max-send-msg-size"],
    }
    rpc_name = "crates/tier-4-runtime/rpc/src/grpc.rs"
    service_name = "crates/tier-5-node/celestia-rs/src/rpc_service.rs"
    assert audit["rs_core_keepalive"] == {
        "path": rpc_name, "sha256": source_digest(source_root, rpc_name, source_commit),
        "finding": "absent: HTTP/2 builder configures no keepalive interval or timeout",
    }
    assert audit["rs_setup_deadline"] == {
        "paths": [rpc_name, service_name],
        "sha256": {
            rpc_name: source_digest(source_root, rpc_name, source_commit),
            service_name: source_digest(source_root, service_name, source_commit),
        },
        "finding": "absent: core and SDK listeners configure no HTTP/2 setup deadline",
    }
    rpc_source = source_bytes(source_root, rpc_name, source_commit).decode()
    service_source = source_bytes(source_root, service_name, source_commit).decode()
    assert "keep_alive_interval" not in rpc_source
    assert "keep_alive_timeout" not in rpc_source
    assert "axum::serve(listener, router).await" in service_source


def verify_metadata(value: dict, implementation: str, listener: str,
                    started: datetime, completed: datetime,
                    *, rejected_compression: bool = False) -> None:
    content_type = ("application/grpc+proto"
                    if implementation == "go" and listener == "core"
                    else "application/grpc")
    expected = {"content-type": [content_type]}
    if implementation == "rs":
        assert set(value) == ({"content-type", "date", "grpc-accept-encoding"}
                              if rejected_compression else {"content-type", "date"})
        assert len(value["date"]) == 1
        date = parsedate_to_datetime(value["date"][0])
        assert date.tzinfo is not None and date.utcoffset() == timedelta(0)
        assert format_datetime(date.astimezone(timezone.utc), usegmt=True) == value["date"][0]
        assert started - timedelta(seconds=1) <= date <= completed + timedelta(seconds=1)
        expected["date"] = value["date"]
        if rejected_compression:
            expected["grpc-accept-encoding"] = ["identity"]
    assert value == expected


def ok(call: dict, implementation: str, started: datetime,
       completed: datetime, label: str) -> None:
    assert set(call) == {"code", "message", "response_hex", "headers", "trailers", "elapsed_ms"}
    assert call["code"] == 0, f"{label}: {call}"
    assert call["message"] == "", f"{label}: {call}"
    assert call.get("response_hex", "") == "", f"{label}: {call}"
    verify_metadata(call["headers"], implementation, "core", started, completed)
    assert call["trailers"] == {}, f"{label}: {call}"
    assert 0 <= call["elapsed_ms"] <= 10_000, f"{label}: {call}"


def deadline(call: dict, label: str) -> None:
    assert set(call) == {"code", "message", "response_hex", "headers", "trailers", "elapsed_ms"}
    assert call["code"] == 4, f"{label}: {call}"
    assert call["message"] == (
        "context deadline exceeded while waiting for connections to become ready"
    ), f"{label}: {call}"
    assert call.get("response_hex", "") == "", f"{label}: {call}"
    assert call.get("headers") is None and call.get("trailers") is None, f"{label}: {call}"
    assert 300 <= call["elapsed_ms"] <= 1_000, f"{label}: {call}"


def frames(data: bytes, preface: bool = False) -> list[tuple[int, int, int, bytes]]:
    if preface:
        marker = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
        assert data.startswith(marker)
        data = data[len(marker):]
    out = []
    while data:
        assert len(data) >= 9
        length = int.from_bytes(data[:3], "big")
        assert len(data) >= 9 + length
        frame_type, flags = data[3], data[4]
        stream = int.from_bytes(data[5:9], "big") & 0x7fffffff
        raw = data[:9 + length]
        out.append((frame_type, flags, stream, raw))
        data = data[9 + length:]
    return out


def varint(data: bytes, offset: int) -> tuple[int, int]:
    value = shift = 0
    while offset < len(data) and shift < 70:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if byte < 0x80:
            return value, offset
        shift += 7
    raise ValueError("invalid protobuf varint")


def proto_fields(data: bytes) -> dict[int, list[int | bytes]]:
    out: dict[int, list[int | bytes]] = {}
    offset = 0
    while offset < len(data):
        tag, offset = varint(data, offset)
        number, wire = tag >> 3, tag & 7
        assert number > 0
        if wire == 0:
            value, offset = varint(data, offset)
        elif wire == 2:
            length, offset = varint(data, offset)
            assert length <= len(data) - offset
            value = data[offset:offset + length]
            offset += length
        else:
            raise AssertionError(f"unsupported protobuf wire type {wire}")
        out.setdefault(number, []).append(value)
    return out


def verify_height_response(value: str, expected_height: int) -> None:
    message = bytes.fromhex(value)
    assert 0 < len(message) <= 1 << 20
    fields = proto_fields(message)
    assert fields[1] == [expected_height] and expected_height > 0
    assert len(fields[2]) == 1 and isinstance(fields[2][0], bytes)
    assert len(fields[2][0]) == 32 and fields[2][0] != bytes(32)
    assert len(fields[3]) == 1 and isinstance(fields[3][0], bytes)
    timestamp = proto_fields(fields[3][0])
    assert len(timestamp[1]) == 1 and timestamp[1][0] > 0


def verify_wire(wire: dict, slow_reader: bool) -> None:
    assert set(wire) == {"client_bytes_hex", "server_frames"}
    client = frames(bytes.fromhex(wire["client_bytes_hex"]), preface=True)
    assert client[0][0:3] == (4, 0, 0)  # initial SETTINGS
    assert any(kind == 4 and flags & 1 and stream == 0 for kind, flags, stream, _ in client)
    if slow_reader:
        assert any(kind == 1 and stream == 1 for kind, _, stream, _ in client)
        assert any(kind == 0 and flags & 1 and stream == 1 for kind, flags, stream, _ in client)
    assert len(client) <= 8
    server = wire["server_frames"]
    assert server and len(server) <= 64
    for item in server:
        assert set(item) == {
            "phase", "elapsed_ms", "type", "flags", "stream_id", "length", "raw_hex",
        }
        raw = bytes.fromhex(item["raw_hex"])
        decoded = frames(raw)
        assert len(decoded) == 1
        kind, flags, stream, _ = decoded[0]
        assert (kind, flags, stream, len(raw) - 9) == (
            item["type"], item["flags"], item["stream_id"], item["length"]
        )
        assert 0 <= item["elapsed_ms"] <= 55_000
    assert server[0]["type"] == 4 and server[0]["stream_id"] == 0


def verify_grpc_envelope(wire: dict, expected_message: bytes) -> None:
    data = bytearray()
    for item in wire["server_frames"]:
        raw = bytes.fromhex(item["raw_hex"])
        decoded = frames(raw)
        assert len(decoded) == 1
        kind, flags, stream, exact = decoded[0]
        if kind != 0 or stream != 1:
            continue
        payload = exact[9:]
        if flags & 0x8:  # PADDED
            assert payload
            padding = payload[0]
            assert padding <= len(payload) - 1
            payload = payload[1:len(payload) - padding]
        data.extend(payload)
    assert len(data) >= 5
    assert data[0] == 0
    declared = int.from_bytes(data[1:5], "big")
    assert declared <= 1 << 20
    assert len(data) == 5 + declared
    assert bytes(data[5:]) == expected_message


def verify_silent_handshake(handshake: dict) -> None:
    assert set(handshake) == {
        "closed_ms", "close_error", "client_bytes_hex", "received_bytes_hex",
    }
    assert 115_000 <= handshake["closed_ms"] <= 130_000
    assert handshake["close_error"]
    assert handshake["client_bytes_hex"] == ""
    raw = bytes.fromhex(handshake["received_bytes_hex"])
    assert 0 < len(raw) <= 1 << 20
    decoded = frames(raw)
    assert 1 <= len(decoded) <= 64
    first_type, first_flags, first_stream, first_raw = decoded[0]
    assert (first_type, first_flags, first_stream) == (4, 0, 0)
    payload = first_raw[9:]
    assert len(payload) % 6 == 0
    settings = {
        int.from_bytes(payload[offset:offset + 2], "big"):
        int.from_bytes(payload[offset + 2:offset + 6], "big")
        for offset in range(0, len(payload), 6)
    }
    assert settings[5] == 16_384  # SETTINGS_MAX_FRAME_SIZE
    for kind, _, stream, _ in decoded:
        assert stream == 0 and kind in {4, 6, 7, 8}


def verify_capture(capture: dict, implementation: str, quota: int) -> None:
    expected_keys = {
        "schema_version", "implementation", "endpoint", "quota", "started_at",
        "completed_at", "input_sha256", "saturation", "cancellation", "slow_reader",
        "compression", "notes",
    }
    if implementation == "go" and quota == 2:
        expected_keys |= {"keepalive_no_pong", "silent_handshake"}
    assert set(capture) == expected_keys
    assert capture["schema_version"] == 1
    assert capture["implementation"] == implementation
    assert capture["quota"] == quota
    assert capture["endpoint"].startswith(("127.0.0.1:", "[::1]:"))
    empty_digest = hashlib.sha256(b"").hexdigest()
    assert capture["input_sha256"] == {
        "ping-empty-protobuf": empty_digest,
        "subscribe-empty-protobuf": empty_digest,
        "gzip-uncompressed-4194305-zero-bytes": hashlib.sha256(
            bytes((4 * 1024 * 1024) + 1)
        ).hexdigest(),
    }
    started = datetime.fromisoformat(capture["started_at"])
    completed = datetime.fromisoformat(capture["completed_at"])
    assert started <= completed <= datetime.now(timezone.utc) + timedelta(minutes=5)

    saturation = capture["saturation"]
    expected_saturation = {
        "configured_quota", "opened", "concurrent_probe", "recovery_action",
    }
    if quota == 2:
        expected_saturation.add("recovered")
    assert set(saturation) == expected_saturation
    assert saturation["configured_quota"] == quota
    assert saturation["opened"] == (4 if quota == 0 else 2)
    if quota == 2:
        deadline(saturation["concurrent_probe"], f"{implementation} quota two")
        ok(saturation["recovered"], implementation, started, completed,
           f"{implementation} saturation recovery")
        assert saturation["recovery_action"] == "closed first established HTTP/2 connection"
    else:
        ok(saturation["concurrent_probe"], implementation, started, completed,
           f"{implementation} quota zero")
        assert saturation.get("recovered") is None
        assert saturation["recovery_action"] == (
            "none: zero is unlimited and the concurrent probe succeeded"
        )

    cancellation = capture["cancellation"]
    assert set(cancellation) == {
        "first_response_hex", "first_height", "buffered_after_cancel_hex", "cancel_code",
        "cancel_message", "same_client_after_cancel",
    }
    assert cancellation["first_response_hex"]
    verify_height_response(cancellation["first_response_hex"], cancellation["first_height"])
    assert cancellation["buffered_after_cancel_hex"] is None
    assert cancellation["cancel_code"] == 1
    assert cancellation["cancel_message"] == "context canceled"
    ok(cancellation["same_client_after_cancel"], implementation, started, completed,
       f"{implementation} same grpc.ClientConn after cancel")

    slow = capture["slow_reader"]
    assert set(slow) == {
        "connections_opened", "connections", "blocked", "recovered_after_close", "held_ms",
    }
    assert slow["connections_opened"] == (2 if quota == 0 else quota)
    assert len(slow["connections"]) == slow["connections_opened"]
    for connection in slow["connections"]:
        assert set(connection) == {"first_response_hex", "height", "wire"}
        message = bytes.fromhex(connection["first_response_hex"])
        verify_height_response(connection["first_response_hex"], connection["height"])
        verify_wire(connection["wire"], slow_reader=True)
        verify_grpc_envelope(connection["wire"], message)
        assert any(item["type"] == 0 and item["stream_id"] == 1
                   for item in connection["wire"]["server_frames"])
    if quota == 2:
        deadline(slow["blocked"], f"{implementation} slow-reader quota")
    else:
        ok(slow["blocked"], implementation, started, completed,
           f"{implementation} unlimited slow-reader probe")
    ok(slow["recovered_after_close"], implementation, started, completed,
       f"{implementation} slow-reader recovery")
    assert 0 <= slow["held_ms"] <= 30_000

    compressed = capture["compression"]
    assert set(compressed) == {"encoding", "uncompressed_len", "result"}
    assert set(compressed["result"]) == {
        "code", "message", "response_hex", "headers", "trailers", "elapsed_ms",
    }
    assert compressed["encoding"] == "gzip"
    assert compressed["uncompressed_len"] == 4 * 1024 * 1024 + 1
    result = compressed["result"]
    assert result["response_hex"] == ""
    assert 0 <= result["elapsed_ms"] <= 10_000
    if implementation == "go":
        assert result["code"] == 8
        assert result["message"] == (
            "grpc: received message after decompression larger than max 4194304"
        )
        assert result.get("headers") is None
        verify_metadata(result["trailers"], implementation, "core", started, completed,
                        rejected_compression=True)
    else:
        # RS currently declines request compression. This is an explicit,
        # bounded behavior difference rather than an allocation escape.
        assert result["code"] == 12
        assert result["message"] == "Content is compressed with `gzip` which isn't supported"
        assert result.get("headers") is None
        verify_metadata(result["trailers"], implementation, "core", started, completed,
                        rejected_compression=True)

    if implementation == "go" and quota == 2:
        keepalive = capture["keepalive_no_pong"]
        assert set(keepalive) == {
            "first_ping_ms", "ping_data_hex", "closed_ms", "close_error", "wire",
        }
        assert 25_000 <= keepalive["first_ping_ms"] <= 36_000
        assert 35_000 <= keepalive["closed_ms"] <= 50_000
        assert keepalive["closed_ms"] > keepalive["first_ping_ms"]
        assert len(bytes.fromhex(keepalive["ping_data_hex"])) == 8
        assert keepalive["close_error"]
        verify_wire(keepalive["wire"], slow_reader=False)
        ping_frames = [item for item in keepalive["wire"]["server_frames"]
                       if item["type"] == 6 and not item["flags"] & 1]
        assert len(ping_frames) == 1
        assert bytes.fromhex(ping_frames[0]["raw_hex"])[9:].hex() == keepalive["ping_data_hex"]
        client_frames = frames(bytes.fromhex(keepalive["wire"]["client_bytes_hex"]), preface=True)
        assert not any(kind == 6 and flags & 1 for kind, flags, _, _ in client_frames)
        verify_silent_handshake(capture["silent_handshake"])
    else:
        assert "keepalive_no_pong" not in capture
        assert "silent_handshake" not in capture

    assert capture["notes"] == [
        "all connections and request bodies are bounded and target explicit loopback addresses",
        "quota zero is the Go compatibility unlimited sentinel; this campaign opens four simultaneous clients",
        "each slow reader decodes one valid height response, then stops reading until its socket is closed",
    ]


def reflection_services(response_hex: str) -> list[str]:
    raw = bytes.fromhex(response_hex)
    assert 0 < len(raw) <= 1 << 20
    response = proto_fields(raw)
    assert len(response[6]) == 1 and isinstance(response[6][0], bytes)
    listed = proto_fields(response[6][0])
    services = []
    for encoded in listed.get(1, []):
        assert isinstance(encoded, bytes)
        service = proto_fields(encoded)
        assert len(service[1]) == 1 and isinstance(service[1][0], bytes)
        services.append(service[1][0].decode())
    assert services and len(services) <= 256
    return sorted(services)


def verify_sdk_capture(capture: dict, implementation: str) -> None:
    assert set(capture) == {
        "schema_version", "implementation", "endpoint", "started_at", "completed_at",
        "input_sha256", "cancellation", "compression", "notes",
    }
    assert capture["schema_version"] == 1 and capture["implementation"] == implementation
    host, separator, port = capture["endpoint"].rpartition(":")
    assert separator and ipaddress.ip_address(host.strip("[]")).is_loopback
    assert 0 < int(port) <= 65535
    raw_request = bytes([0x0a, 61]) + b"x" * 61 + bytes([0x3a, 0])
    assert len(raw_request) == 65
    assert capture["input_sha256"] == {
        "gzip-reflection-list-services-65-bytes": hashlib.sha256(raw_request).hexdigest(),
    }
    started = datetime.fromisoformat(capture["started_at"])
    completed = datetime.fromisoformat(capture["completed_at"])
    assert started.tzinfo and started <= completed

    cancellation = capture["cancellation"]
    assert set(cancellation) == {
        "first_response_hex", "first_services", "first_headers", "cancel_code",
        "cancel_message", "cancel_trailers", "recovered_response_hex",
        "recovered_services", "recovered_headers",
    }
    first_raw = bytes.fromhex(cancellation["first_response_hex"])
    recovered_raw = bytes.fromhex(cancellation["recovered_response_hex"])
    assert 0 < len(first_raw) <= 4_096
    assert 0 < len(recovered_raw) <= 4_096
    first = reflection_services(cancellation["first_response_hex"])
    recovered = reflection_services(cancellation["recovered_response_hex"])
    assert cancellation["first_services"] == cancellation["recovered_services"] == first == recovered
    assert first == sorted(set(first))
    for required in (
        "cosmos.base.node.v1beta1.Service",
        "grpc.reflection.v1alpha.ServerReflection",
    ):
        assert required in first
    verify_metadata(cancellation["first_headers"], implementation, "sdk", started, completed)
    assert cancellation["cancel_code"] == 1
    assert cancellation["cancel_message"] == "context canceled"
    assert cancellation["cancel_trailers"] == {}
    verify_metadata(cancellation["recovered_headers"], implementation, "sdk",
                    started, completed)

    compressed = capture["compression"]
    assert set(compressed) == {"encoding", "uncompressed_len", "result"}
    assert compressed["encoding"] == "gzip" and compressed["uncompressed_len"] == 65
    result = compressed["result"]
    assert set(result) == {
        "code", "message", "response_hex", "headers", "trailers", "elapsed_ms",
    }
    assert result["response_hex"] == "" and 0 <= result["elapsed_ms"] <= 10_000
    if implementation == "go":
        assert result["code"] == 8
        assert result["message"] == "grpc: received message after decompression larger than max 64"
    else:
        assert result["code"] == 12
        assert result["message"] == "Content is compressed with `gzip` which isn't supported"
    assert result["headers"] is None
    verify_metadata(result["trailers"], implementation, "sdk", started, completed,
                    rejected_compression=True)
    assert capture["notes"] == [
        "SDK connection quota is unsupported; cancellation recovery uses the same grpc.ClientConn",
        "the compressed reflection request is exactly 65 uncompressed protobuf bytes against a 64-byte receive limit",
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    root = args.capture.resolve()
    meta = load(root / "meta.json")
    assert set(meta) == {
        "schema_version", "captured_at", "mainnet_observation", "go_builds", "rs_build",
        "source_commit", "recorder_binary_sha256", "inventory", "campaign_order",
        "campaigns", "source_audit", "source_sha256", "input_sha256",
        "artifact_sha256", "deviations",
    }
    assert meta["schema_version"] == 1
    assert meta["mainnet_observation"] == "mainnet-observation.json"
    source_root = Path(__file__).resolve().parents[2]
    source_commit = meta["source_commit"]
    subprocess.run(["git", "-C", str(source_root), "verify-commit", source_commit],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    verify_inventory(root, meta, source_root, source_commit)
    verify_builds(root, meta, source_root, source_commit)
    verify_source_audit(meta, source_root, source_commit)

    observation = load(root / meta["mainnet_observation"])
    assert set(observation) == {
        "endpoint", "observed_at", "version", "chain_id", "responses",
    }
    assert observation["chain_id"] == "celestia"
    assert observation["version"] == "10.5.0"
    observed = datetime.fromisoformat(observation["observed_at"])
    captured = datetime.fromisoformat(meta["captured_at"])
    assert observed <= captured and captured - observed <= timedelta(hours=24)
    assert observed.tzinfo is not None and captured.tzinfo is not None
    assert observation["endpoint"].startswith("https://")
    assert set(observation["responses"]) == {"status", "abci_info"}
    parsed = {}
    for name in ("status", "abci_info"):
        assert set(observation["responses"][name]) == {"body", "sha256"}
        raw = observation["responses"][name]["body"].encode()
        assert hashlib.sha256(raw).hexdigest() == observation["responses"][name]["sha256"]
        parsed[name] = json.loads(raw)
    status = parsed["status"]["result"]
    info = parsed["abci_info"]["result"]["response"]
    assert status["node_info"]["network"] == "celestia"
    assert info["version"] == "10.5.0"
    assert status["node_info"]["protocol_version"]["app"] == info["app_version"]
    assert status["sync_info"]["latest_block_height"] == info["last_block_height"]
    verify_configs_and_order(root, meta, observed)
    verify_resource_artifacts(root, meta)

    deviations = meta["deviations"]
    assert set(deviations) == {
        "rs_request_compression", "sdk_connection_quota",
        "core_message_size_configuration", "rs_core_keepalive", "rs_handshake_deadline",
        "transport_metadata",
    }
    assert deviations["rs_request_compression"] == (
        "unsupported on core and SDK: RS returns Unimplemented before decompression; "
        "Go accepts gzip and applies each listener's receive limit to the decompressed message"
    )
    assert deviations["sdk_connection_quota"] == (
        "unsupported by both production app listeners: app.toml has no connection-quota setting"
    )
    assert deviations["core_message_size_configuration"] == (
        "unsupported by both core listeners: receive is fixed at 4 MiB and send at MaxInt32"
    )
    assert deviations["rs_core_keepalive"] == (
        "absent: Go core sends idle PING at 30s and closes after 10s without ACK; "
        "RS configures no HTTP/2 keepalive"
    )
    assert deviations["rs_handshake_deadline"] == (
        "absent: grpc-go closes a silent HTTP/2 setup at 120s; RS has no gRPC setup deadline"
    )
    assert deviations["transport_metadata"] == (
        "intentional: Go core uses application/grpc+proto; Go SDK and RS use "
        "application/grpc; RS adds RFC 1123 Date and advertises identity when rejecting gzip"
    )

    for implementation in ("go", "rs"):
        for quota in (0, 2):
            verify_capture(load(root / f"{implementation}-quota-{quota}.json"),
                           implementation, quota)
        verify_sdk_capture(load(root / f"{implementation}-sdk.json"), implementation)


if __name__ == "__main__":
    main()
