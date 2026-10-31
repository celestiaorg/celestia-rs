#!/usr/bin/env python3
"""Offline verifier for the paired current-release live transaction campaign."""

from __future__ import annotations

import argparse
import base64
import copy
import datetime as dt
import hashlib
import ipaddress
import pathlib
import re
import sys
from typing import Any

import harness


PUBLIC_FILES = {
    "mainnet-observation.json",
    "build-receipts.json",
    "topology.json",
    "genesis.json",
    "configs.json",
    "inputs.json",
    "paired-responses.json",
    "inclusions.json",
    "cleanup.json",
    "resources.json",
    "blob-proxy.json",
}
SEMANTIC_FILES = (
    "mainnet-observation.json", "build-receipts.json", "topology.json", "genesis.json", "configs.json", "inputs.json",
    "paired-responses.json", "inclusions.json", "cleanup.json",
    "resources.json", "blob-proxy.json",
)
# Set to the independently reviewed digest after the real capture. Keeping this
# unset makes a source-only candidate fail closed rather than admitting mutable
# paired facts by checksum alone.
EXPECTED_SEMANTIC_DIGEST = "UNSET_UNTIL_REVIEWED_CAPTURE"
SENSITIVE = re.compile(r"(^|/)(data|keyring[^/]*|node_key\.json|priv_validator[^/]*|.*\.mnemonic|.*\.log)(/|$)", re.I)


class VerificationError(harness.HarnessError):
    pass


def require(condition: bool, code: str, detail: str) -> None:
    if not condition:
        raise VerificationError(f"{code}: {detail}")


def load(path: pathlib.Path) -> Any:
    try:
        return harness.strict_json(path.read_bytes())
    except (OSError, UnicodeError, harness.HarnessError, ValueError) as error:
        raise VerificationError(f"json.invalid: {path.name}: {error}") from error


def semantic_digest(root: pathlib.Path) -> str:
    digest = hashlib.sha256()
    for name in SEMANTIC_FILES:
        digest.update(name.encode() + b"\0")
        value = load(root / name)
        if name == "build-receipts.json":
            # Avoid a self-reference cycle: the reviewed digest lives in this
            # verifier, whose own file hash is independently checked below.
            value = copy_json(value)
            value["capture_sources"]["verify.py"] = "<self-checked-verifier>"
        digest.update(harness.canonical_json(value))
    return digest.hexdigest()


def copy_json(value: Any) -> Any:
    import copy
    return copy.deepcopy(value)


def parse_time(value: Any, label: str) -> dt.datetime:
    require(isinstance(value, str), "time.invalid", f"{label} is not a string")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise VerificationError(f"time.invalid: {label}") from error
    require(parsed.tzinfo is not None and parsed.utcoffset() is not None, "time.invalid", f"{label} lacks timezone")
    return parsed


def safe_inventory(root: pathlib.Path, manifest: dict[str, Any]) -> None:
    actual: set[str] = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        require(not path.is_symlink(), "inventory.symlink", relative)
        require(path.is_file() or path.is_dir(), "inventory.nonregular", relative)
        if path.is_file() and relative != "manifest.json":
            actual.add(relative)
    expected = set(manifest["files"])
    require(actual == expected, "inventory.exact", f"missing={sorted(expected-actual)} extra={sorted(actual-expected)}")
    require(actual == PUBLIC_FILES, "inventory.public", "public file inventory differs")
    for name, record in manifest["files"].items():
        require(not pathlib.PurePosixPath(name).is_absolute() and ".." not in pathlib.PurePosixPath(name).parts,
                "inventory.path", name)
        require(not SENSITIVE.search(name), "inventory.sensitive", name)
        harness.require_fields(record, {"sha256", "size"}, f"manifest file {name}")
        data = (root / name).read_bytes()
        require(record["size"] == len(data), "inventory.size", name)
        require(record["sha256"] == hashlib.sha256(data).hexdigest(), "inventory.hash", name)


def verify_observation(observation: dict[str, Any], capture_started: dt.datetime) -> None:
    fields = {"endpoint", "observed_at", "chain_id", "version", "app_protocol", "status_height", "abci_height", "responses"}
    harness.require_fields(observation, fields, "mainnet observation")
    endpoint = urllib_split(observation["endpoint"])
    require(endpoint[0] == "https", "observation.endpoint", "endpoint is not HTTPS")
    try:
        address = ipaddress.ip_address(endpoint[1])
    except ValueError:
        require(endpoint[1] not in {"localhost"} and not endpoint[1].endswith(".localhost"),
                "observation.endpoint", "endpoint is not public")
    else:
        require(address.is_global, "observation.endpoint", "endpoint is not public")
    observed = parse_time(observation["observed_at"], "observed_at")
    require(observed <= capture_started, "observation.order", "observation follows capture start")
    require((dt.datetime.now(dt.timezone.utc) - observed).total_seconds() >= -300,
            "observation.future", "observation is more than five minutes in the future")
    require((capture_started - observed).total_seconds() <= 3600, "observation.stale", "observation is older than one hour")
    require(observation["chain_id"] == "celestia" and observation["version"] == "10.5.0" and observation["app_protocol"] == 10,
            "observation.release", "active release identity differs")
    require(observation["status_height"] == observation["abci_height"], "observation.height", "status/ABCI height differs")
    responses = harness.require_fields(observation["responses"], {"status", "abci_info"}, "observation responses")
    parsed: dict[str, Any] = {}
    for name, record in responses.items():
        harness.require_fields(record, {"body", "sha256"}, f"observation {name}")
        require(hashlib.sha256(record["body"].encode()).hexdigest() == record["sha256"],
                "observation.hash", name)
        parsed[name] = harness.strict_json(record["body"])["result"]
    require(parsed["status"]["node_info"]["network"] == "celestia", "observation.raw", "raw chain differs")
    abci_response = harness.require_fields(parsed["abci_info"]["response"], harness.ABCI_INFO_RESPONSE_FIELDS,
                                           "observation ABCI response")
    require(abci_response["data"] == "celestia-app" and abci_response["version"] == "10.5.0",
            "observation.raw", "raw application identity differs")
    require(int(abci_response["app_version"]) == 10, "observation.raw", "raw app protocol differs")
    require(int(parsed["status"]["sync_info"]["latest_block_height"]) == observation["status_height"],
            "observation.raw", "raw status height differs")
    require(int(abci_response["last_block_height"]) == observation["abci_height"],
            "observation.raw", "raw ABCI height differs")
    try:
        last_app_hash = base64.b64decode(abci_response["last_block_app_hash"], validate=True)
    except (TypeError, ValueError) as error:
        raise VerificationError("observation.raw: raw last app hash is invalid") from error
    require(len(last_app_hash) == 32, "observation.raw", "raw last app hash length differs")
    try:
        freshness_bound_ms = harness.consensus_freshness_bound_ms(
            abci_response["timeout_info"]
        )
    except (KeyError, harness.HarnessError) as error:
        raise VerificationError(f"observation.timeouts: {error}") from error
    require(freshness_bound_ms == 5600, "observation.timeouts", "freshness derivation differs")


def urllib_split(value: Any) -> tuple[str, str]:
    require(isinstance(value, str), "observation.endpoint", "endpoint is not a string")
    from urllib.parse import urlsplit
    parsed = urlsplit(value)
    require(parsed.hostname is not None, "observation.endpoint", "endpoint lacks hostname")
    return parsed.scheme, parsed.hostname.lower()


def verify_receipts(receipts: dict[str, Any], support: dict[str, Any]) -> None:
    harness.require_fields(receipts, {"go", "rust", "transport_client", "capture_sources"}, "build receipts")
    go = harness.require_fields(receipts["go"], {"module", "source_commit", "core_commit", "compiler", "tags", "binary_sha256", "command", "build_info", "build_info_sha256", "cli_source_sha256"}, "Go receipt")
    require(go["module"] == "github.com/celestiaorg/celestia-app/v10@v10.5.0", "receipt.go", "Go module differs")
    require(go["source_commit"] == "6013e758d5dceb75827ad191c8d739d7c43cfa04", "receipt.go", "Go source differs")
    require(go["core_commit"] == "a5537d84a008d566abc503d2ea41e30f2301403d", "receipt.go", "core source differs")
    require(go["compiler"] == "go1.26.9" and go["tags"] == ["ledger"], "receipt.go", "Go toolchain/tags differ")
    require(re.fullmatch(r"[0-9a-f]{64}", go["binary_sha256"]) is not None and
            re.fullmatch(r"[0-9a-f]{64}", go["cli_source_sha256"]) is not None,
            "receipt.go", "Go binary/CLI source hash invalid")
    require(hashlib.sha256(go["build_info"].encode()).hexdigest() == go["build_info_sha256"],
            "receipt.go", "Go build-info hash differs")
    ldflags = (
        "-X github.com/cosmos/cosmos-sdk/version.Name=celestia-app "
        "-X github.com/cosmos/cosmos-sdk/version.AppName=celestia-appd "
        "-X github.com/cosmos/cosmos-sdk/version.Version=10.5.0 "
        f"-X github.com/cosmos/cosmos-sdk/version.Commit={go['source_commit']} "
        "-X github.com/cosmos/cosmos-sdk/version.BuildTags=ledger "
        "-X github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd/cmd.v2UpgradeHeight=0"
    )
    require(go["command"] == ["go", "build", "-buildvcs=false", "-tags=ledger", f"-ldflags={ldflags}", "-o", "celestia-appd", "./cmd/celestia-appd"],
            "receipt.go", "Go build command differs")
    require("go1.26.9" in go["build_info"] and
            "path\tgithub.com/celestiaorg/celestia-app/v10/cmd/celestia-appd" in go["build_info"] and
            "mod\tgithub.com/celestiaorg/celestia-app/v10\t(devel)" in go["build_info"] and
            "=>\tgithub.com/celestiaorg/celestia-core\tv0.42.3" in go["build_info"],
            "receipt.go", "Go build-info module/core/toolchain differs")
    rust = harness.require_fields(receipts["rust"], {"source_commit", "compiler", "binary_sha256", "command", "features", "version_long", "cargo_lock_sha256"}, "Rust receipt")
    require(rust["source_commit"] == support["candidate_commit"], "receipt.rust", "candidate commit differs")
    require(support["candidate_commit"] in rust["version_long"], "receipt.rust", "meaningful build identity lacks commit")
    require(rust["compiler"] == "rustc 1.98.0", "receipt.rust", "Rust compiler differs")
    require(rust["command"] == ["cargo", "build", "--locked", "--bin", "celestia-rs"],
            "receipt.rust", "Rust build command differs")
    require(rust["features"] == ["default"], "receipt.rust", "Rust features differ")
    require(re.fullmatch(r"[0-9a-f]{64}", rust["binary_sha256"]) is not None,
            "receipt.rust", "Rust binary hash invalid")
    repository = pathlib.Path(__file__).resolve().parents[3]
    require(hashlib.sha256((repository / "Cargo.lock").read_bytes()).hexdigest() == rust["cargo_lock_sha256"],
            "receipt.rust", "Cargo.lock hash differs")
    client = harness.require_fields(receipts["transport_client"], {"binary_sha256", "source_sha256", "go_mod_sha256", "go_sum_sha256", "compiler", "command"}, "transport client receipt")
    client_root = repository / "conformance/provider-client"
    require(re.fullmatch(r"[0-9a-f]{64}", client["binary_sha256"]) is not None,
            "receipt.client", "transport client binary hash invalid")
    require(client["source_sha256"] == hashlib.sha256((client_root / "main.go").read_bytes()).hexdigest() and
            client["go_mod_sha256"] == hashlib.sha256((client_root / "go.mod").read_bytes()).hexdigest() and
            client["go_sum_sha256"] == hashlib.sha256((client_root / "go.sum").read_bytes()).hexdigest(),
            "receipt.client", "transport client source/module hashes differ")
    require(client["compiler"] == "go1.26.9" and client["command"] == ["go", "build", "-trimpath", "-o", "provider-client", "."],
            "receipt.client", "transport client build recipe differs")
    sources = receipts["capture_sources"]
    require(isinstance(sources, dict) and set(sources) == {"harness.py", "verify.py", "capture.py", "driver.py", "blob_proxy.py", "campaign.py"},
            "receipt.sources", "capture source inventory differs")
    here = pathlib.Path(__file__).parent
    for name in ("harness.py", "verify.py", "capture.py", "driver.py", "blob_proxy.py", "campaign.py"):
        require(sources[name] == hashlib.sha256((here / name).read_bytes()).hexdigest(),
                "receipt.sources", f"{name} source hash differs")
    for digest in sources.values():
        require(isinstance(digest, str) and len(digest) == 64, "receipt.sources", "capture source hash invalid")


def verify_topology(topology: dict[str, Any], receipts: dict[str, Any]) -> None:
    fields = {"generations", "pex", "mempool_broadcast", "validator_power", "mempool_size", "max_txs", "max_txs_bytes", "max_subscription_clients", "max_subscriptions_per_client"}
    harness.require_fields(topology, fields, "topology")
    generations = harness.require_fields(topology["generations"], {"normal", "timeout", "restored"}, "topology generations")
    require(topology["pex"] is False and topology["mempool_broadcast"] is False,
            "topology.isolation", "PEX or mempool propagation enabled")
    require(topology["validator_power"] == {"go": "1", "rs": "1"}, "topology.power", "validator power differs")
    require(topology["max_subscription_clients"] == harness.SUBSCRIPTION_CLIENT_LIMITS and
            topology["max_subscriptions_per_client"] == 1,
            "topology.capacity", "subscription capacity differs")
    require(topology["mempool_size"] == 0 and topology["max_txs"] == 1 and
            topology["max_txs_bytes"] == harness.MEMPOOL_MAX_TXS_BYTES,
            "topology.capacity", "mempool capacity invalid")
    generation_ticks: set[tuple[str, int]] = set()
    reference_endpoints: dict[str, str] | None = None
    for generation in ("normal", "timeout", "restored"):
        row = harness.require_fields(generations[generation], {"endpoints", "listeners"}, f"topology {generation}")
        try:
            harness.assert_expected_listeners(row["endpoints"], row["listeners"])
        except harness.HarnessError as error:
            raise VerificationError(f"topology.listeners: {generation}: {error}") from error
        if reference_endpoints is None:
            reference_endpoints = row["endpoints"]
        else:
            require(row["endpoints"] == reference_endpoints, "topology.endpoints", "restart endpoints changed")
        for listener in row["listeners"]:
            role = "go" if listener["name"].startswith("go_") else "rs"
            expected_hash = receipts["go" if role == "go" else "rust"]["binary_sha256"]
            require(listener["executable_sha256"] == expected_hash, "topology.binary", f"{generation}/{listener['name']}")
        for role in ("go", "rs"):
            role_ticks = {listener["pid_start_ticks"] for listener in row["listeners"] if listener["name"].startswith(f"{role}_")}
            require(len(role_ticks) == 1, "topology.process", f"{generation}/{role} listeners do not share one process identity")
            identity = (role, next(iter(role_ticks)))
            require(identity not in generation_ticks, "topology.process", f"{role} PID start ticks reused across generations")
            generation_ticks.add(identity)


def verify_genesis(genesis: dict[str, Any]) -> None:
    fields = {"app_name", "app_version", "genesis_time", "chain_id", "initial_height", "app_hash", "app_state", "consensus"}
    harness.require_fields(genesis, fields, "genesis")
    require(genesis["app_name"] == "celestia-appd" and genesis["app_version"] == "10.5.0" and
            genesis["chain_id"] == "rs-devnet-187" and genesis["initial_height"] == 1,
            "genesis.identity", "app/chain/initial height differs")
    consensus = harness.require_fields(genesis["consensus"], {"params", "validators"}, "genesis consensus")
    require(isinstance(consensus["params"], dict) and consensus["params"],
            "genesis.consensus", "consensus parameters are empty")
    validators = consensus["validators"]
    require(isinstance(validators, list) and len(validators) == 2, "genesis.validators", "expected exactly two validators")
    addresses: set[str] = set()
    powers: list[int] = []
    for row in validators:
        harness.require_fields(row, {"address", "pub_key", "power", "name"}, "genesis validator")
        require(row["name"] in {"go", "rs"} and row["name"] not in addresses, "genesis.validators", "validator roles differ")
        addresses.add(row["name"])
        require(isinstance(row["address"], str) and row["address"], "genesis.validators", "validator address missing")
        require(isinstance(row["pub_key"], dict) and row["pub_key"], "genesis.validators", "validator public key missing")
        powers.append(int(row["power"]))
    require(powers[0] == powers[1] > 0, "genesis.validators", "validator power differs")
    require(isinstance(genesis["app_state"], dict) and genesis["app_state"], "genesis.app_state", "application genesis is empty")


def verify_configs(configs: dict[str, Any], topology: dict[str, Any]) -> None:
    harness.require_fields(configs, {"normal", "timeout", "restored", "full"}, "configs")
    try:
        harness.validate_config_delta(configs["normal"], configs["timeout"])
    except harness.HarnessError as error:
        raise VerificationError(f"config.delta: {error}") from error
    require(configs["restored"] == configs["normal"], "config.restore", "restored config differs from normal")
    config_fields = {
        "priv_validator_grpc_laddr",
        "rpc.timeout_broadcast_tx_commit", "rpc.laddr", "rpc.grpc_laddr",
        "rpc.max_subscription_clients", "rpc.max_subscriptions_per_client",
        "p2p.pex", "p2p.laddr", "mempool.broadcast", "mempool.size", "mempool.max_txs_bytes",
        "tx_index.indexer", "consensus.timeout_commit",
    }
    app_fields = {"grpc.enable", "grpc.address", "grpc-web.enable", "api.enable", "api.address"}
    full = harness.require_fields(configs["full"], {"normal", "timeout", "restored"}, "full configs")
    paths = {
        "config.toml": {
            "priv_validator_grpc_laddr": ("priv_validator_grpc_laddr",),
            "rpc.timeout_broadcast_tx_commit": ("rpc", "timeout_broadcast_tx_commit"),
            "rpc.laddr": ("rpc", "laddr"), "rpc.grpc_laddr": ("rpc", "grpc_laddr"),
            "rpc.max_subscription_clients": ("rpc", "max_subscription_clients"),
            "rpc.max_subscriptions_per_client": ("rpc", "max_subscriptions_per_client"),
            "p2p.pex": ("p2p", "pex"), "p2p.laddr": ("p2p", "laddr"),
            "mempool.broadcast": ("mempool", "broadcast"),
            "mempool.size": ("mempool", "size"),
            "mempool.max_txs_bytes": ("mempool", "max_txs_bytes"),
            "tx_index.indexer": ("tx_index", "indexer"),
            "consensus.timeout_commit": ("consensus", "timeout_commit"),
        },
        "app.toml": {"grpc.enable": ("grpc", "enable"), "grpc.address": ("grpc", "address"),
                     "grpc-web.enable": ("grpc-web", "enable"),
                     "api.enable": ("api", "enable"), "api.address": ("api", "address")},
    }

    def projected(value: dict[str, Any], filename: str) -> dict[str, Any]:
        result = {}
        for name, keys in paths[filename].items():
            item: Any = value
            for key in keys:
                require(isinstance(item, dict) and key in item, "config.full", f"{filename} lacks {name}")
                item = item[key]
            result[name] = item
        return result

    for phase in ("normal", "timeout", "restored"):
        endpoints = topology["generations"][phase]["endpoints"]
        require(set(configs[phase]) == {"go", "rs"}, "config.nodes", f"{phase} node inventory differs")
        full_phase = harness.require_fields(full[phase], {"go", "rs"}, f"{phase} full configs")
        for node in ("go", "rs"):
            files = harness.require_fields(configs[phase][node], {"config.toml", "app.toml"}, f"{phase} {node} configs")
            full_files = harness.require_fields(full_phase[node], {"config.toml", "app.toml"}, f"{phase} {node} full configs")
            config = harness.require_fields(files["config.toml"], config_fields, f"{phase} {node} config.toml")
            app = harness.require_fields(files["app.toml"], app_fields, f"{phase} {node} app.toml")
            require(projected(full_files["config.toml"], "config.toml") == config and
                    projected(full_files["app.toml"], "app.toml") == app,
                    "config.full", f"{phase}/{node} full config projection differs")
            require(config["priv_validator_grpc_laddr"] == "" and
                    config["p2p.pex"] is False and config["mempool.broadcast"] is False and
                    config["tx_index.indexer"] == "kv" and config["consensus.timeout_commit"] == "5s",
                    "config.isolation", f"{phase} {node}")
            require(config["mempool.size"] == topology["mempool_size"] and
                    config["mempool.max_txs_bytes"] == topology["max_txs_bytes"] and
                    config["rpc.max_subscription_clients"] == harness.SUBSCRIPTION_CLIENT_LIMITS[node] and
                    config["rpc.max_subscriptions_per_client"] == 1,
                    "config.capacity", f"{phase} {node}")
            require(app["grpc.enable"] is True and app["api.enable"] is True and app["grpc-web.enable"] is False,
                    "config.listeners", f"{phase} {node}")
            require(config["rpc.laddr"] == endpoints[f"{node}_rpc"] and
                    config["rpc.grpc_laddr"] == endpoints[f"{node}_core"] and
                    config["p2p.laddr"] == endpoints[f"{node}_p2p"] and
                    app["grpc.address"] == endpoints[f"{node}_sdk"] and
                    app["api.address"] == endpoints[f"{node}_rest"],
                    "config.listeners", f"{phase} {node}")
    for node in ("go", "rs"):
        require(full["restored"][node] == full["normal"][node], "config.full", f"{node} restored full config differs")
        normalized = copy.deepcopy(full["timeout"][node])
        normalized["config.toml"]["rpc"]["timeout_broadcast_tx_commit"] = \
            full["normal"][node]["config.toml"]["rpc"]["timeout_broadcast_tx_commit"]
        require(normalized == full["normal"][node], "config.full", f"{node} timeout full config has another delta")


def verify_response_record(record: dict[str, Any], label: str) -> dict[str, Any]:
    fields = {
        "transport", "wire", "http_status", "http_headers", "json", "grpc_status",
        "grpc_message", "initial_metadata", "trailing_metadata", "websocket_frames",
        "websocket_handshake", "client_error", "process_effect",
    }
    harness.require_fields(record, fields, label)
    wire = None if record["wire"] is None else harness.decode_blob(record["wire"], f"{label}: wire")
    http_headers = harness.stable_headers(record["http_headers"])
    initial = harness.stable_headers(record["initial_metadata"])
    trailing = harness.stable_headers(record["trailing_metadata"])
    frames = []
    for frame in record["websocket_frames"]:
        harness.require_fields(frame, {"opcode", "payload"}, f"{label}: WebSocket frame")
        require(frame["opcode"] in {"text", "binary", "close"}, "response.websocket", label)
        frames.append((frame["opcode"], harness.decode_blob(frame["payload"], f"{label}: WebSocket frame")))
    require(record["transport"] in {spec[1] for spec in harness.CASE_SPECS}, "response.transport", label)
    require(record["http_status"] is None or isinstance(record["http_status"], int), "response.status", label)
    require(record["grpc_status"] is None or isinstance(record["grpc_status"], int), "response.status", label)
    require(record["grpc_message"] is None or isinstance(record["grpc_message"], str), "response.status", label)
    require(record["client_error"] is None or isinstance(record["client_error"], str), "response.error", label)
    require(record["process_effect"] is None or isinstance(record["process_effect"], str), "response.effect", label)
    transport = record["transport"]
    if transport in {"rest", "comet-jsonrpc", "comet-uri"}:
        concrete_disconnect = (transport == "comet-jsonrpc" and
                               record["process_effect"] == "client-disconnected-after-complete-write" and
                               record["client_error"] == "client-disconnected-after-complete-write")
        require((isinstance(record["http_status"], int) and wire is not None and record["json"] is not None) or
                (concrete_disconnect and record["http_status"] is None and wire is None and record["json"] is None),
                "response.http", label)
        require(http_headers or concrete_disconnect, "response.http", f"{label}: stable HTTP headers empty")
        require(record["grpc_status"] is None and not frames and record["websocket_handshake"] is None,
                "response.http", label)
        if not concrete_disconnect:
            require(harness.strict_json(wire) == record["json"], "response.wire", f"{label}: HTTP wire/JSON differs")
    elif transport in {"cosmos-grpc", "core-grpc"}:
        concrete_socket_close = record["process_effect"] == "abci-connection-closed" and record["grpc_status"] is None
        concrete_cancel = record["process_effect"] == "client-canceled" and record["grpc_status"] is None
        require(record["http_status"] is None and wire is not None and
                (isinstance(record["grpc_status"], int) or concrete_socket_close or concrete_cancel),
                "response.grpc", label)
        require(initial or concrete_socket_close or concrete_cancel,
                "response.grpc", f"{label}: initial metadata empty")
        if initial:
            initial_map = {name: value for name, value in initial}
            require(initial_map.get("content-type") in {"application/grpc", "application/grpc+proto"},
                    "response.grpc", f"{label}: gRPC content-type metadata differs")
        require(not frames and record["websocket_handshake"] is None, "response.grpc", label)
    elif transport == "comet-websocket":
        handshake = harness.require_fields(record["websocket_handshake"], {"status", "headers", "request_id"}, f"{label}: WebSocket handshake")
        require(handshake["status"] == 101 and handshake["request_id"] == 187, "response.websocket", label)
        stable_handshake = harness.stable_headers(handshake["headers"])
        header_map = {name: value for name, value in stable_handshake}
        require(header_map.get("upgrade", "").lower() == "websocket" and "upgrade" in header_map.get("connection", "").lower(),
                "response.websocket", f"{label}: upgrade handshake differs")
        require(header_map.get("sec-websocket-accept") == harness.FIXED_WEBSOCKET_ACCEPT,
                "response.websocket", f"{label}: fixed-key handshake accept differs")
        require(frames and record["http_status"] is None and record["grpc_status"] is None,
                "response.websocket", label)
        require(len(frames) == 1 and frames[0][0] == "text" and wire == frames[0][1] and
                harness.strict_json(wire) == record["json"],
                "response.wire", f"{label}: WebSocket wire/frame/JSON differs")
    if transport in {"comet-jsonrpc", "comet-websocket"} and record["process_effect"] != "client-disconnected-after-complete-write":
        require(isinstance(record["json"], dict) and record["json"].get("id") == 187,
                "response.jsonrpc", f"{label}: JSON-RPC id differs")
    elif transport == "comet-uri":
        require(isinstance(record["json"], dict) and record["json"].get("id") == -1,
                "response.jsonrpc", f"{label}: URI response id differs")
    return {
        "transport": record["transport"], "wire": wire, "http_status": record["http_status"],
        "stable_http_headers": http_headers, "json": record["json"], "grpc_status": record["grpc_status"],
        "grpc_message": record["grpc_message"], "initial_metadata": initial, "trailing_metadata": trailing,
        "websocket_frames": frames,
        "websocket_handshake": (None if record["websocket_handshake"] is None else {
            "status": record["websocket_handshake"]["status"],
            "headers": stable_handshake,
            "request_id": record["websocket_handshake"]["request_id"],
        }),
        "client_error": record["client_error"], "process_effect": record["process_effect"],
    }


def verify_lifecycle_outcome(record: dict[str, Any], outcome: str, label: str) -> None:
    """Decode the concrete CheckTx outcome used by lifecycle operations."""
    transport = record["transport"]
    code: int
    log: str
    if transport == "comet-jsonrpc":
        result = record["json"].get("result") if isinstance(record["json"], dict) else None
        require(isinstance(result, dict) and isinstance(result.get("code"), int) and isinstance(result.get("log"), str),
                "cleanup.outcome", f"{label}: Comet result schema differs")
        code, log = result["code"], result["log"]
    elif transport == "core-grpc":
        raw = harness.decode_blob(record["wire"], f"{label}: core response")
        outer = harness.protobuf_fields(raw)
        check_fields = [value for number, wire, value in outer if number == 1 and wire == 2]
        require(len(check_fields) == 1 and isinstance(check_fields[0], bytes),
                "cleanup.outcome", f"{label}: core response lacks CheckTx")
        decoded = harness.protobuf_fields(check_fields[0])
        codes = [value for number, wire, value in decoded if number == 1 and wire == 0]
        logs = [value for number, wire, value in decoded if number == 3 and wire == 2]
        require(len(codes) <= 1 and (not codes or isinstance(codes[0], int)) and len(logs) <= 1,
                "cleanup.outcome", f"{label}: core CheckTx schema differs")
        code = 0 if not codes else codes[0]
        log = "" if not logs else bytes(logs[0]).decode("utf-8", "strict")
    else:
        raise VerificationError(f"cleanup.outcome: {label}: unsupported lifecycle transport")
    if outcome in {"admitted", "included"}:
        require(code == 0, "cleanup.outcome", f"{label}: success has nonzero code")
    elif outcome == "mempool-full":
        require(code != 0 and "mempool" in log.lower() and "full" in log.lower(),
                "cleanup.outcome", f"{label}: capacity rejection differs")
    elif outcome == "duplicate":
        require(code != 0 and any(marker in log.lower() for marker in ("already exists", "duplicate", "mempool")),
                "cleanup.outcome", f"{label}: duplicate rejection differs")
    elif outcome == "capacity-rejected":
        require(code != 0 and any(marker in log.lower() for marker in ("sequence", "capacity", "mempool")),
                "cleanup.outcome", f"{label}: capacity probe rejection differs")


def verify_lifecycle_inclusions(rows: Any, expected_hashes: list[str], previous: dict[str, Any], label: str) -> dict[str, Any]:
    require(isinstance(rows, list) and len(rows) == len(expected_hashes), "cleanup.inclusion", f"{label} inventory differs")
    state = dict(previous)
    previous_height = int(state["height"])
    for index, (pair, expected_hash) in enumerate(zip(rows, expected_hashes, strict=True)):
        inclusion_pair = harness.require_fields(pair, {"go", "rs"}, f"{label} inclusion {index}")
        require(inclusion_pair["go"] == inclusion_pair["rs"], "cleanup.inclusion", f"{label}/{index} pair differs")
        record = harness.require_fields(inclusion_pair["go"], {"included_count", "tx_hash", "indexed_tx_hash", "check_tx", "check_tx_source", "deliver_tx", "post_state"}, f"{label} inclusion {index}")
        require(record["included_count"] == 1 and record["tx_hash"] == expected_hash and
                record["indexed_tx_hash"] == expected_hash,
                "cleanup.inclusion", f"{label}/{index} hash/count differs")
        require(record["check_tx_source"] == "broadcast-response", "cleanup.inclusion",
                f"{label}/{index} lacks a concrete CheckTx response")
        for result_name in ("check_tx", "deliver_tx"):
            result = harness.require_fields(record[result_name], {"code", "gas_wanted", "gas_used", "events", "codespace", "log", "data"}, f"{label}/{index} {result_name}")
            require(result["code"] == 0 and all(isinstance(result[field], str) for field in ("gas_wanted", "gas_used", "codespace", "log", "data")) and
                    result["gas_wanted"].isdigit() and result["gas_used"].isdigit() and isinstance(result["events"], list),
                    "cleanup.inclusion", f"{label}/{index} result differs")
        post = harness.require_fields(record["post_state"], {"height", "app_hash", "balance", "sequence", "delegation"}, f"{label}/{index} post-state")
        require(isinstance(post["height"], int) and post["height"] > previous_height and
                post["sequence"] == state["sequence"] + 1 and isinstance(post["app_hash"], str) and post["app_hash"] and
                isinstance(post["balance"], str) and post["balance"].isdigit() and post["delegation"] == state["delegation"],
                "cleanup.inclusion", f"{label}/{index} state continuity differs")
        state = {"height": post["height"], "app_hash": post["app_hash"], "account_number": state["account_number"],
                 "sequence": post["sequence"], "balance": post["balance"], "delegation": post["delegation"]}
        previous_height = post["height"]
    return state


def verify_cases(inputs: dict[str, Any], responses: dict[str, Any], inclusions: dict[str, Any], cleanup: dict[str, Any],
                 max_txs: int, max_txs_bytes: int, receipts: dict[str, Any],
                 generation_intervals: dict[str, tuple[int, int]]) -> dict[str, Any]:
    harness.require_fields(inputs, {"cases", "capacity", "lifecycle_inputs"}, "inputs")
    harness.require_fields(responses, {"cases", "driver_sha256", "transport_client_sha256"}, "responses")
    harness.require_fields(inclusions, {"cases"}, "inclusions")
    require(responses["driver_sha256"] == receipts["capture_sources"]["driver.py"] and
            responses["transport_client_sha256"] == receipts["transport_client"]["binary_sha256"],
            "response.driver", "paired response executables differ from build receipts")
    capacity = harness.require_fields(inputs["capacity"], {"tx_a", "tx_b", "mempool_size", "max_txs", "max_txs_bytes"}, "capacity inputs")
    require(capacity["mempool_size"] == 0 and capacity["max_txs"] == max_txs == 1 and
            capacity["max_txs_bytes"] == max_txs_bytes,
            "capacity.config", "input/topology capacity differs")
    tx_a = harness.decode_blob(capacity["tx_a"], "capacity tx A")
    tx_b = harness.decode_blob(capacity["tx_b"], "capacity tx B")
    require(tx_a != tx_b and 0 < len(tx_a) <= max_txs_bytes and 0 < len(tx_b) <= max_txs_bytes,
            "capacity.boundary", "capacity inputs do not exercise the one-transaction limit")
    for document, label in ((inputs, "inputs"), (responses, "responses"), (inclusions, "inclusions")):
        require([row.get("name") for row in document["cases"]] == list(harness.CASE_NAMES),
                "case.inventory", f"{label} case inventory/order differs")
    input_by_name = {row["name"]: row for row in inputs["cases"]}
    response_by_name = {row["name"]: row for row in responses["cases"]}
    inclusion_by_name = {row["name"]: row for row in inclusions["cases"]}
    last_state: dict[str, Any] | None = None
    last_height = 0
    for spec in harness.CASE_SPECS:
        name, transport, mode, family, expected = spec
        source = harness.require_fields(input_by_name[name], {"name", "generation", "transport", "mode", "family", "expected_outcome", "pre_state", "boundary", "requests", "request_semantics"}, f"input {name}")
        require(tuple(source[field] for field in ("name", "transport", "mode", "family", "expected_outcome")) == spec,
                "case.mapping", name)
        expected_generation = "normal" if name not in {"server-timeout", "raw-malformed-protobuf"} else "timeout" if name == "server-timeout" else "restored"
        require(source["generation"] == expected_generation, "case.generation", name)
        sampling_started_ms, sampling_completed_ms = generation_intervals[expected_generation]
        pre_pair = harness.require_fields(source["pre_state"], {"go", "rs"}, f"pre-state {name}")
        pre_fields = {"height", "block_hash", "app_hash", "account_number", "sequence", "balance", "delegation"}
        pre = harness.require_fields(pre_pair["go"], pre_fields, f"Go pre-state {name}")
        rs_pre = harness.require_fields(pre_pair["rs"], pre_fields, f"Rust pre-state {name}")
        require(pre == rs_pre, "case.prestate", f"paired pre-state differs: {name}")
        require(isinstance(pre["height"], int) and pre["height"] > 0, "case.prestate", name)
        require(pre["height"] > last_height, "case.order", f"case heights do not increase: {name}")
        last_height = pre["height"]
        if last_state is not None:
            for field in ("app_hash", "account_number", "sequence", "balance", "delegation"):
                require(pre[field] == last_state[field], "case.continuity", f"{name}: {field} disconnect")
        try:
            harness.assert_boundary(source["boundary"], name)
            request_bytes = harness.assert_request_identity(source["requests"], name)
            if name == "rest-invalid-base64":
                harness.assert_invalid_rest_base64_request(request_bytes)
            elif name == "cosmos-grpc-no-message":
                harness.assert_txraw_no_messages(request_bytes)
            elif name == "raw-malformed-protobuf":
                harness.assert_malformed_protobuf(request_bytes)
        except harness.HarnessError as error:
            raise VerificationError(f"case.request: {name}: {error}") from error
        boundary_go = source["boundary"]["go"]
        require((boundary_go["height"], boundary_go["block_hash"], boundary_go["app_hash"]) ==
                (pre["height"], pre["block_hash"], pre["app_hash"]),
                "case.boundary", f"boundary/pre-state disconnect: {name}")
        require(sampling_started_ms <= source["boundary"]["go"]["received_monotonic_ms"] <= sampling_completed_ms and
                sampling_started_ms <= source["boundary"]["rs"]["received_monotonic_ms"] <= sampling_completed_ms and
                sampling_started_ms <= source["boundary"]["barrier_release_monotonic_ms"] <= sampling_completed_ms,
                "case.capture", f"case boundary is outside node sampling interval: {name}")
        if name == "mempool-capacity-recovery":
            require(harness.decode_blob(source["requests"]["go"], "capacity case request") == tx_a,
                    "capacity.request", "capacity tx A is not the submitted case request")
        if transport.startswith("comet-"):
            expected_method = {
                "async": "broadcast_tx_async", "sync": "broadcast_tx_sync", "commit": "broadcast_tx_commit",
                "commit-disconnect": "broadcast_tx_commit",
            }.get(mode)
            expected_id = None if transport == "comet-uri" else 187
            require(source["request_semantics"] == {"jsonrpc_id": expected_id, "method": expected_method},
                    "case.jsonrpc", name)
        else:
            require(source["request_semantics"] is None, "case.jsonrpc", name)
        response = harness.require_fields(response_by_name[name], {"name", "admission", "go", "rs", "excluded_headers", "intentional_difference", "driver", "raw_driver"}, f"response {name}")
        driver = harness.require_fields(response["driver"], {"barrier_release_monotonic_ns", "pair_completed_monotonic_ns", "go_invoked_monotonic_ns", "rs_invoked_monotonic_ns", "timeout_ms", "disconnect_after_write", "request_sha256", "driver_sha256", "transport_client_sha256", "go_result_sha256", "rs_result_sha256"}, f"driver receipt {name}")
        raw_driver = harness.strict_json(harness.decode_blob(response["raw_driver"], f"raw driver {name}"))
        raw_fields = {"name", "barrier_release_monotonic_ns", "pair_completed_monotonic_ns", "timeout_ms",
                      "disconnect_after_write", "go_invoked_monotonic_ns", "rs_invoked_monotonic_ns",
                      "request_sha256", "transport_client_sha256", "driver_sha256", "go_result_sha256",
                      "rs_result_sha256", "go", "rs"}
        harness.require_fields(raw_driver, raw_fields, f"raw driver {name}")
        require(raw_driver["name"] == name and raw_driver["go"] == response["go"] and raw_driver["rs"] == response["rs"] and
                all(raw_driver[key] == driver[key] for key in raw_fields - {"name", "go", "rs"}),
                "response.driver", f"complete raw driver document differs: {name}")
        release_ns = driver["barrier_release_monotonic_ns"]
        require(all(isinstance(driver[field], int) for field in ("barrier_release_monotonic_ns", "pair_completed_monotonic_ns", "go_invoked_monotonic_ns", "rs_invoked_monotonic_ns")) and
                release_ns <= driver["go_invoked_monotonic_ns"] <= driver["pair_completed_monotonic_ns"] and
                release_ns <= driver["rs_invoked_monotonic_ns"] <= driver["pair_completed_monotonic_ns"],
                "response.driver", f"driver chronology differs: {name}")
        require(sampling_started_ms * 1_000_000 <= release_ns and
                driver["pair_completed_monotonic_ns"] <= sampling_completed_ms * 1_000_000,
                "response.driver", f"driver chronology is outside node sampling interval: {name}")
        expected_timeout = 100 if name == "core-grpc-cancel" else 5000 if name == "server-timeout" else 30000
        require(driver["timeout_ms"] == expected_timeout and
                driver["disconnect_after_write"] is (name == "comet-commit-disconnect"),
                "response.driver", f"driver timeout/disconnect policy differs: {name}")
        require(abs(release_ns // 1_000_000 - source["boundary"]["barrier_release_monotonic_ms"]) <= 1 and
                driver["request_sha256"] == source["requests"]["go"]["sha256"] and
                driver["driver_sha256"] == responses["driver_sha256"] and
                driver["transport_client_sha256"] == responses["transport_client_sha256"],
                "response.driver", f"driver cross-artifact binding differs: {name}")
        require(driver["go_result_sha256"] == hashlib.sha256(harness.canonical_json(response["go"])).hexdigest() and
                driver["rs_result_sha256"] == hashlib.sha256(harness.canonical_json(response["rs"])).hexdigest(),
                "response.driver", f"driver result binding differs: {name}")
        require(response["excluded_headers"] == list(harness.HTTP_EXCLUDED_HEADERS), "response.exclusions", name)
        try:
            harness.assert_admission(response["admission"], pre["height"], expected, name)
        except harness.HarnessError as error:
            raise VerificationError(f"response.admission: {name}: {error}") from error
        require(response["admission"]["go"]["tx_hash"] == source["requests"]["go"]["sha256"],
                "response.admission", f"{name} admission hash is not the submitted request hash")
        for side in ("go", "rs"):
            require(source["boundary"]["barrier_release_monotonic_ms"] + response["admission"][side]["completed_after_barrier_ms"] <= sampling_completed_ms,
                    "response.admission", f"{name}/{side} admission is outside node sampling interval")
        go_projection = verify_response_record(response["go"], f"Go response {name}")
        rs_projection = verify_response_record(response["rs"], f"Rust response {name}")
        require(go_projection["transport"] == rs_projection["transport"] == transport,
                "response.transport", name)
        if name == "comet-commit-disconnect":
            require(go_projection["process_effect"] == rs_projection["process_effect"] == "client-disconnected-after-complete-write",
                    "response.lifecycle", "disconnect case lacks concrete completed-write disconnect")
        elif name == "core-grpc-cancel":
            require(go_projection["process_effect"] == rs_projection["process_effect"] == "client-canceled" and
                    go_projection["client_error"] is not None and rs_projection["client_error"] is not None,
                    "response.lifecycle", "gRPC cancel case lacks concrete cancellation")
        elif name == "server-timeout":
            require(go_projection["process_effect"] == rs_projection["process_effect"] == "server-timeout",
                    "response.lifecycle", "server timeout case lacks concrete timeout")
        if expected == "policy-difference":
            require(response["intentional_difference"] == {
                "policy": "rust-bounded-error-go-abci-socket-close",
                "go": "abci-connection-closed",
                "rs": "invalid-argument-malformed-protobuf",
            }, "response.policy", name)
            require(go_projection != rs_projection, "response.policy", "concrete malformed outcomes unexpectedly equal")
            require(go_projection["process_effect"] == "abci-connection-closed" and
                    go_projection["client_error"] == "grpc-transport-closed" and
                    rs_projection["process_effect"] == "continued" and rs_projection["grpc_status"] == 3 and
                    rs_projection["grpc_message"] == "invalid-argument-malformed-protobuf",
                    "response.policy", "malformed concrete process/status outcome differs")
        else:
            require(response["intentional_difference"] is None, "response.policy", name)
            require(go_projection == rs_projection, "response.exact", name)
        inclusion_pair = harness.require_fields(inclusion_by_name[name], {"name", "go", "rs"}, f"inclusion {name}")
        inclusion_fields = {"included_count", "tx_hash", "indexed_tx_hash", "check_tx", "check_tx_source", "deliver_tx", "post_state"}
        inclusion = harness.require_fields(inclusion_pair["go"], inclusion_fields, f"Go inclusion {name}")
        rs_inclusion = harness.require_fields(inclusion_pair["rs"], inclusion_fields, f"Rust inclusion {name}")
        require(inclusion == rs_inclusion, "inclusion.exact", name)
        for side in ("go", "rs"):
            admission = response["admission"][side]
            if admission["proof"] == "indexed-inclusion":
                require(inclusion_pair[side]["included_count"] == 1 and
                        isinstance(inclusion_pair[side]["post_state"], dict) and
                        inclusion_pair[side]["post_state"]["height"] == admission["indexed_height"],
                        "inclusion.admission", f"indexed admission proof differs: {name}/{side}")
        if expected in {"included", "execution-failed"}:
            require(inclusion["included_count"] == 1, "inclusion.count", name)
            require(inclusion["tx_hash"] == response["admission"]["go"]["tx_hash"], "inclusion.hash", name)
            request_bytes = harness.decode_blob(source["requests"]["go"], f"request {name}")
            expected_indexed_hash = (hashlib.sha256(harness.decode_canonical_blob_tx(request_bytes)["tx"]).hexdigest()
                                     if name == "blob-envelope" else inclusion["tx_hash"])
            require(inclusion["indexed_tx_hash"] == expected_indexed_hash,
                    "inclusion.indexed_hash", name)
            post = harness.require_fields(inclusion["post_state"], {"height", "app_hash", "balance", "sequence", "delegation"}, f"post-state {name}")
            require(post["height"] > pre["height"], "inclusion.height", name)
            require(inclusion["check_tx_source"] in {"broadcast-response", "mempool-admission", "indexed-inclusion"} and
                    isinstance(inclusion["deliver_tx"], dict), "inclusion.result", name)
            admission_only_names = {"comet-commit-disconnect", "core-grpc-cancel", "server-timeout"}
            admission_proofs = {response["admission"][side]["proof"] for side in ("go", "rs")}
            expected_source = ("indexed-inclusion" if name in admission_only_names and
                               "indexed-inclusion" in admission_proofs else
                               "mempool-admission" if name in admission_only_names else "broadcast-response")
            require(inclusion["check_tx_source"] == expected_source,
                    "inclusion.result", f"CheckTx evidence source differs: {name}")
            if inclusion["check_tx_source"] == "broadcast-response":
                require(isinstance(inclusion["check_tx"], dict), "inclusion.result", f"missing CheckTx response: {name}")
                check = harness.require_fields(inclusion["check_tx"], {"code", "gas_wanted", "gas_used", "events", "codespace", "log", "data"}, f"CheckTx {name}")
            else:
                require(name in admission_only_names and inclusion["check_tx_source"] in
                        {"mempool-admission", "indexed-inclusion"} and
                        inclusion["check_tx"] is None and response["admission"]["go"]["outcome"] == "admitted",
                        "inclusion.result", f"admission-only CheckTx is not allowed: {name}")
                check = None
            deliver = harness.require_fields(inclusion["deliver_tx"], {"code", "gas_wanted", "gas_used", "events", "codespace", "log", "data"}, f"DeliverTx {name}")
            if check is not None:
                require(check["code"] == 0, "inclusion.code", f"CheckTx failed: {name}")
            require(deliver["code"] == (5 if expected == "execution-failed" else 0), "inclusion.code", f"DeliverTx code differs: {name}")
            for result in (deliver,) if check is None else (check, deliver):
                require(isinstance(result["code"], int) and
                        all(isinstance(result[field], str) for field in ("codespace", "log", "data")),
                        "inclusion.result", name)
                require(all(isinstance(result[field], str) and result[field].isdigit() for field in ("gas_wanted", "gas_used")),
                        "inclusion.gas", name)
                require(isinstance(result["events"], list), "inclusion.events", name)
                for event in result["events"]:
                    require(isinstance(event, dict) and set(event) == {"type", "attributes"} and
                            isinstance(event["type"], str) and isinstance(event["attributes"], list),
                            "inclusion.events", name)
                    for attribute in event["attributes"]:
                        require(isinstance(attribute, dict) and set(attribute) in ({"key", "value"}, {"key", "value", "index"}) and
                                isinstance(attribute["key"], str) and isinstance(attribute["value"], str) and
                                ("index" not in attribute or isinstance(attribute["index"], bool)),
                                "inclusion.events", name)
            last_state = {
                "height": post["height"], "app_hash": post["app_hash"], "account_number": pre["account_number"],
                "sequence": post["sequence"], "balance": post["balance"], "delegation": post["delegation"],
            }
            lifecycle = {
                "mempool-capacity-recovery": ("mempool_capacity", [capacity["tx_b"]["sha256"], hashlib.sha256(harness.decode_blob(inputs["lifecycle_inputs"]["mempool_followup"], "mempool followup")).hexdigest()]),
                "comet-commit-disconnect": ("disconnect", [hashlib.sha256(harness.decode_blob(inputs["lifecycle_inputs"]["disconnect_followup"], "disconnect followup")).hexdigest()]),
                "core-grpc-cancel": ("grpc_cancel", [hashlib.sha256(harness.decode_blob(inputs["lifecycle_inputs"]["grpc_cancel_followup"], "cancel followup")).hexdigest()]),
                "server-timeout": ("server_timeout", [hashlib.sha256(harness.decode_blob(inputs["lifecycle_inputs"]["server_timeout_followup"], "timeout followup")).hexdigest()]),
            }.get(name)
            if lifecycle is not None:
                lifecycle_name, expected_hashes = lifecycle
                last_state = verify_lifecycle_inclusions(cleanup[lifecycle_name]["indexed_inclusions"], expected_hashes, last_state, lifecycle_name)
        else:
            require(inclusion == {"included_count": 0, "tx_hash": response["admission"]["go"]["tx_hash"],
                                  "indexed_tx_hash": None, "check_tx": None,
                                  "check_tx_source": None, "deliver_tx": None, "post_state": None},
                    "inclusion.rejection", name)
            if last_state is None:
                last_state = {field: pre[field] for field in ("height", "app_hash", "account_number", "sequence", "balance", "delegation")}
    require(last_state is not None, "case.continuity", "no final state")
    return last_state


def verify_cleanup(cleanup: dict[str, Any], inputs: dict[str, Any], responses: dict[str, Any],
                   inclusions: dict[str, Any], receipts: dict[str, Any], last_state: dict[str, Any],
                   generation_intervals: dict[str, tuple[int, int]]) -> None:
    fields = {"mempool_capacity", "disconnect", "grpc_cancel", "server_timeout", "final"}
    harness.require_fields(cleanup, fields, "cleanup")
    phases = {
        "mempool_capacity": ["a-admitted", "b-rejected", "a-included", "b-retry-admitted", "b-included", "followup-succeeded", "mempool-empty"],
        "disconnect": ["request-written", "client-disconnected", "tx-included", "retry-observed", "followup-succeeded", "mempool-empty"],
        "grpc_cancel": ["request-started", "client-canceled", "tx-included", "retry-observed", "followup-succeeded", "mempool-empty"],
        "server_timeout": ["request-started", "server-timeout", "tx-included", "retry-observed", "normal-config-restored", "followup-succeeded", "mempool-empty"],
    }
    outcomes = {
        "mempool_capacity": ["admitted", "mempool-full", "included", "admitted", "included", "included", "empty"],
        "disconnect": ["written", "disconnected", "included", "duplicate", "included", "empty"],
        "grpc_cancel": ["started", "canceled", "included", "duplicate", "included", "empty"],
        "server_timeout": ["started", "timeout", "included", "duplicate", "restored", "included", "empty"],
    }
    mempool_sizes = {
        "mempool_capacity": [1, 1, 0, 1, 0, 0, 0],
        "disconnect": [0, 1, 0, 0, 0, 0],
        "grpc_cancel": [0, 1, 0, 0, 0, 0],
        "server_timeout": [0, 1, 0, 0, 0, 0, 0],
    }
    response_absent = {
        "disconnect": {"request-written", "client-disconnected", "tx-included", "mempool-empty"},
        "grpc_cancel": {"request-started", "client-canceled", "tx-included", "mempool-empty"},
        "server_timeout": {"request-started", "tx-included", "normal-config-restored", "mempool-empty"},
        "mempool_capacity": {"a-included", "b-included", "mempool-empty"},
    }
    lifecycle_input_fields = {
        "mempool_followup", "disconnect_followup", "grpc_cancel_followup",
        "server_timeout_followup",
    }
    lifecycle_inputs = harness.require_fields(inputs["lifecycle_inputs"], lifecycle_input_fields, "lifecycle inputs")
    lifecycle_digests = {
        name: hashlib.sha256(harness.decode_blob(value, f"lifecycle input {name}")).hexdigest()
        for name, value in lifecycle_inputs.items()
    }
    case_requests = {row["name"]: row["requests"]["go"]["sha256"] for row in inputs["cases"]}
    capacity = inputs["capacity"]
    expected_hashes = {
        "mempool_capacity": [capacity["tx_a"]["sha256"], capacity["tx_b"]["sha256"], capacity["tx_a"]["sha256"], capacity["tx_b"]["sha256"], capacity["tx_b"]["sha256"], lifecycle_digests["mempool_followup"], None],
        "disconnect": [case_requests["comet-commit-disconnect"]] * 4 + [lifecycle_digests["disconnect_followup"], None],
        "grpc_cancel": [case_requests["core-grpc-cancel"]] * 4 + [lifecycle_digests["grpc_cancel_followup"], None],
        "server_timeout": [case_requests["server-timeout"], case_requests["server-timeout"], case_requests["server-timeout"], case_requests["server-timeout"], None, lifecycle_digests["server_timeout_followup"], None],
    }
    case_barriers = {row["name"]: row["boundary"]["barrier_release_monotonic_ms"] for row in inputs["cases"]}
    lifecycle_barriers = {
        "mempool_capacity": case_barriers["mempool-capacity-recovery"],
        "disconnect": case_barriers["comet-commit-disconnect"],
        "grpc_cancel": case_barriers["core-grpc-cancel"],
        "server_timeout": case_barriers["server-timeout"],
    }
    response_cases = {row["name"]: row for row in responses["cases"]}
    inclusion_cases = {row["name"]: row for row in inclusions["cases"]}
    lifecycle_cases = {
        "mempool_capacity": "mempool-capacity-recovery",
        "disconnect": "comet-commit-disconnect",
        "grpc_cancel": "core-grpc-cancel",
        "server_timeout": "server-timeout",
    }
    expected_transports = {
        "mempool_capacity": "comet-jsonrpc", "disconnect": "comet-jsonrpc",
        "grpc_cancel": "core-grpc", "server_timeout": "core-grpc",
    }
    event_generations = {
        "mempool_capacity": ["normal"] * 7,
        "disconnect": ["normal"] * 6,
        "grpc_cancel": ["normal"] * 6,
        "server_timeout": ["timeout"] * 4 + ["restored"] * 3,
    }
    event_fields = {"phase", "generation", "monotonic_ms", "outcome", "tx_hash", "mempool_size", "status_height", "response", "operation"}
    main_response_phases = {("mempool_capacity", "a-admitted"), ("server_timeout", "server-timeout")}
    for name, expected_phases in phases.items():
        pair = harness.require_fields(cleanup[name], {"go", "rs", "indexed_inclusions", "blocks", "block_intervals_ms"}, f"cleanup {name}")
        case_name = lifecycle_cases[name]
        main_response = response_cases[case_name]
        indexed_main_proof = (name in {"disconnect", "grpc_cancel"} and any(
            main_response["admission"][side]["proof"] == "indexed-inclusion" for side in ("go", "rs")
        ))
        projections: dict[str, list[Any]] = {}
        for side in ("go", "rs"):
            events = pair[side]
            require(isinstance(events, list) and [event.get("phase") for event in events] == expected_phases,
                    "cleanup.phases", f"{name}/{side}")
            require([event.get("outcome") for event in events] == outcomes[name],
                    "cleanup.transitions", f"{name}/{side}")
            observed_pools = [event.get("mempool_size") for event in events]
            if name == "mempool_capacity":
                require(observed_pools[1] in {0, 1} and
                        observed_pools[:1] + observed_pools[2:] ==
                        mempool_sizes[name][:1] + mempool_sizes[name][2:],
                        "cleanup.transitions", f"{name}/{side}")
            else:
                expected_mempools = list(mempool_sizes[name])
                if (name in {"disconnect", "grpc_cancel"} and
                        main_response["admission"][side]["proof"] == "indexed-inclusion"):
                    expected_mempools[1] = 0
                require(observed_pools == expected_mempools,
                        "cleanup.transitions", f"{name}/{side}")
            require([event.get("generation") for event in events] == event_generations[name],
                    "cleanup.generation", f"{name}/{side}")
            require(events[0]["monotonic_ms"] >= lifecycle_barriers[name],
                    "cleanup.order", f"{name}/{side} begins before its paired case barrier")
            previous_time = -1
            previous_height = -1
            projected = []
            for event in events:
                harness.require_fields(event, event_fields, f"cleanup {name}/{side} event")
                require(isinstance(event["monotonic_ms"], int) and event["monotonic_ms"] > previous_time,
                        "cleanup.order", f"{name}/{side}")
                generation_start, generation_end = generation_intervals[event["generation"]]
                require(generation_start <= event["monotonic_ms"] <= generation_end,
                        "cleanup.capture", f"{name}/{side} event is outside its generation interval")
                require(isinstance(event["status_height"], int) and event["status_height"] >= previous_height,
                        "cleanup.height", f"{name}/{side}")
                require(isinstance(event["mempool_size"], int) and event["mempool_size"] >= 0,
                        "cleanup.mempool", f"{name}/{side}")
                previous_time = event["monotonic_ms"]
                previous_height = event["status_height"]
                response = None if event["response"] is None else verify_response_record(event["response"], f"cleanup {name}/{side}/{event['phase']}")
                require((response is None) is (event["phase"] in response_absent[name]),
                        "cleanup.response", f"{name}/{side}/{event['phase']}")
                if response is not None:
                    require(response["transport"] == expected_transports[name],
                            "cleanup.response", f"{name}/{side}/{event['phase']} transport differs")
                    if event["outcome"] in {"admitted", "included", "mempool-full", "duplicate", "capacity-rejected"}:
                        verify_lifecycle_outcome(event["response"], event["outcome"], f"{name}/{side}/{event['phase']}")
                if response is None or (name, event["phase"]) in main_response_phases:
                    require(event["operation"] is None, "cleanup.operation", f"{name}/{side}/{event['phase']} unexpected operation receipt")
                else:
                    operation = harness.require_fields(event["operation"], {
                        "request_sha256", "campaign_sha256", "transport_client_sha256",
                        "barrier_release_monotonic_ns", "invoked_monotonic_ns", "completed_monotonic_ns", "result_sha256",
                    }, f"cleanup {name}/{side}/{event['phase']} operation")
                    require(operation["request_sha256"] == event["tx_hash"] and
                            operation["campaign_sha256"] == receipts["capture_sources"]["campaign.py"] and
                            operation["transport_client_sha256"] == responses["transport_client_sha256"] and
                            operation["result_sha256"] == hashlib.sha256(harness.canonical_json(event["response"])).hexdigest(),
                            "cleanup.operation", f"{name}/{side}/{event['phase']} receipt binding differs")
                    require(isinstance(operation["invoked_monotonic_ns"], int) and
                            isinstance(operation["completed_monotonic_ns"], int) and
                            isinstance(operation["barrier_release_monotonic_ns"], int) and
                            generation_start * 1_000_000 <= operation["barrier_release_monotonic_ns"] <= operation["invoked_monotonic_ns"] <= operation["completed_monotonic_ns"] <= event["monotonic_ms"] * 1_000_000,
                            "cleanup.operation", f"{name}/{side}/{event['phase']} receipt chronology differs")
                projected.append({key: event[key] for key in ("phase", "outcome", "tx_hash", "mempool_size", "status_height")} | {"response": response})
            require(events[-1]["mempool_size"] == 0 and events[-1]["outcome"] == "empty",
                    "cleanup.mempool", f"{name}/{side} final mempool")
            projections[side] = projected
        if indexed_main_proof:
            require(projections["go"][:1] + projections["go"][2:] ==
                    projections["rs"][:1] + projections["rs"][2:] and
                    {key: value for key, value in projections["go"][1].items()
                     if key not in {"mempool_size", "status_height"}} ==
                    {key: value for key, value in projections["rs"][1].items()
                     if key not in {"mempool_size", "status_height"}},
                    "cleanup.exact", name)
        else:
            require(projections["go"] == projections["rs"], "cleanup.exact", name)
        for go_event, rs_event in zip(pair["go"], pair["rs"], strict=True):
            if go_event["operation"] is not None:
                require(go_event["operation"]["barrier_release_monotonic_ns"] == rs_event["operation"]["barrier_release_monotonic_ns"] and
                        go_event["operation"]["request_sha256"] == rs_event["operation"]["request_sha256"],
                        "cleanup.operation", f"{name}/{go_event['phase']} pair barrier/request differs")
        driver = main_response["driver"]
        invocation_ms = max(driver["go_invoked_monotonic_ns"], driver["rs_invoked_monotonic_ns"]) // 1_000_000
        pair_completed_ms = (driver["pair_completed_monotonic_ns"] + 999_999) // 1_000_000
        admission_ms = case_barriers[case_name] + max(
            main_response["admission"]["go"]["completed_after_barrier_ms"],
            main_response["admission"]["rs"]["completed_after_barrier_ms"],
        )
        if name == "mempool_capacity":
            require(pair["go"][0]["monotonic_ms"] >= admission_ms and
                    projections["go"][0]["response"] == verify_response_record(main_response["go"], f"main response {case_name}"),
                    "cleanup.link", "capacity admission is not linked to main driver/admission response")
            inclusion_index = 2
        elif name in {"disconnect", "grpc_cancel"}:
            require(pair["go"][0]["monotonic_ms"] >= invocation_ms,
                    "cleanup.link", f"{name} chronology is not linked to the main driver")
            for side in ("go", "rs"):
                admission = main_response["admission"][side]
                side_admission_ms = case_barriers[case_name] + admission["completed_after_barrier_ms"]
                completion_floor = (max(pair_completed_ms, side_admission_ms)
                                    if admission["proof"] == "indexed-inclusion" else pair_completed_ms)
                require(pair[side][1]["monotonic_ms"] >= completion_floor and
                        pair[side][1]["status_height"] == admission["status_height"],
                        "cleanup.link", f"{name}/{side} completion contradicts its admission sample")
            inclusion_index = 2
        else:
            require(pair["go"][0]["monotonic_ms"] >= invocation_ms and pair["go"][1]["monotonic_ms"] >= pair_completed_ms and
                    projections["go"][1]["response"] == verify_response_record(main_response["go"], f"main response {case_name}"),
                    "cleanup.link", "server timeout is not linked to the main driver response")
            inclusion_index = 2
        main_inclusion = inclusion_cases[case_name]["go"]
        require(main_inclusion["post_state"] is not None and
                pair["go"][inclusion_index]["status_height"] >= main_inclusion["post_state"]["height"] and
                pair["rs"][inclusion_index]["status_height"] >= main_inclusion["post_state"]["height"],
                "cleanup.link", f"{name} inclusion height is not linked to indexed inclusion")
        if name == "mempool_capacity" and pair["go"][1]["mempool_size"] == 0:
            require(pair["go"][1]["status_height"] == main_inclusion["post_state"]["height"] and
                    pair["rs"][1]["status_height"] == main_inclusion["post_state"]["height"],
                    "cleanup.link", "post-rejection empty mempool is not linked to A inclusion")
        indexed_heights = [row["go"]["post_state"]["height"] for row in pair["indexed_inclusions"]]
        if name == "mempool_capacity":
            require(pair["go"][4]["status_height"] >= indexed_heights[0] and pair["go"][5]["status_height"] >= indexed_heights[1],
                    "cleanup.link", "capacity B/followup heights are not linked to indexed inclusions")
        elif name in {"disconnect", "grpc_cancel"}:
            require(pair["go"][4]["status_height"] >= indexed_heights[0],
                    "cleanup.link", f"{name} followup height is not linked to indexed inclusion")
        else:
            require(pair["go"][5]["status_height"] >= indexed_heights[0],
                    "cleanup.link", "server timeout followup height is not linked to indexed inclusion")
        event_hashes = [event["tx_hash"] for event in pair["go"]]
        require(all(value is None or re.fullmatch(r"[0-9a-f]{64}", value) is not None for value in event_hashes),
                "cleanup.hash", name)
        require(event_hashes == expected_hashes[name], "cleanup.hash", f"{name} hashes are not bound to captured inputs")
        if name == "mempool_capacity":
            require(event_hashes[0] == event_hashes[2] and event_hashes[1] == event_hashes[3] == event_hashes[4] and
                    len({event_hashes[0], event_hashes[1], event_hashes[5]}) == 3 and event_hashes[6] is None,
                    "cleanup.hash", name)
        elif name in {"disconnect", "grpc_cancel"}:
            require(len(set(event_hashes[:4])) == 1 and event_hashes[4] != event_hashes[0] and event_hashes[5] is None,
                    "cleanup.hash", name)
        else:
            require(len(set(event_hashes[:4])) == 1 and event_hashes[4] is None and
                    event_hashes[5] != event_hashes[0] and event_hashes[6] is None,
                    "cleanup.hash", name)
        intervals = pair["block_intervals_ms"]
        blocks = pair["blocks"]
        require(isinstance(intervals, list), "cleanup.timing", name)
        if name == "server_timeout":
            require(isinstance(blocks, list) and len(blocks) == 4, "cleanup.timing", "server timeout needs four common blocks")
            parsed_times: list[dt.datetime] = []
            heights: list[int] = []
            timeout_start, timeout_end = generation_intervals["timeout"]
            for block in blocks:
                row = harness.require_fields(block, {"generation", "go", "rs"}, "server timeout block")
                require(row["generation"] == "timeout", "cleanup.timing", "block observed outside timeout generation")
                block_fields = {"height", "block_hash", "app_hash", "time", "observed_monotonic_ms"}
                go_block = harness.require_fields(row["go"], block_fields, "timeout Go block")
                rs_block = harness.require_fields(row["rs"], block_fields, "timeout Rust block")
                require({key: go_block[key] for key in block_fields - {"observed_monotonic_ms"}} ==
                        {key: rs_block[key] for key in block_fields - {"observed_monotonic_ms"}},
                        "cleanup.timing", "timeout common block differs")
                require(timeout_start <= go_block["observed_monotonic_ms"] <= timeout_end and
                        timeout_start <= rs_block["observed_monotonic_ms"] <= timeout_end and
                        abs(go_block["observed_monotonic_ms"] - rs_block["observed_monotonic_ms"]) <= 250,
                        "cleanup.timing", "timeout block observation outside generation/skew")
                heights.append(go_block["height"])
                parsed_times.append(parse_time(go_block["time"], "timeout block time"))
            require(all(right == left + 1 for left, right in zip(heights, heights[1:])),
                    "cleanup.timing", "timeout blocks are not consecutive")
            derived = [int((right - left).total_seconds() * 1000) for left, right in zip(parsed_times, parsed_times[1:])]
            require(intervals == derived and len(intervals) == 3 and all(value > 1000 for value in intervals),
                    "cleanup.timing", "server timeout intervals do not match raw common blocks")
            observed_start = min(blocks[0][side]["observed_monotonic_ms"] for side in ("go", "rs"))
            observed_end = max(blocks[-1][side]["observed_monotonic_ms"] for side in ("go", "rs"))
            require(observed_start <= pair["go"][0]["monotonic_ms"] <= pair["go"][1]["monotonic_ms"] <= observed_end,
                    "cleanup.timing", "server timeout request/outcome is not bracketed by common blocks")
        else:
            require(intervals == [] and blocks == [], "cleanup.timing", f"unexpected block evidence for {name}")
    final = harness.require_fields(cleanup["final"], {"go_root", "rs_root", "go_balance", "rs_balance", "go_sequence", "rs_sequence", "go_delegation", "rs_delegation", "go_mempool", "rs_mempool", "liveness"}, "cleanup final")
    for left, right in (("go_root", "rs_root"), ("go_balance", "rs_balance"), ("go_sequence", "rs_sequence"), ("go_delegation", "rs_delegation")):
        require(final[left] == final[right], "cleanup.final", f"{left}/{right} differ")
    require(final["go_mempool"] == final["rs_mempool"] == 0, "cleanup.final", "final mempool nonempty")
    require((final["go_root"], final["go_balance"], final["go_sequence"], final["go_delegation"]) ==
            (last_state["app_hash"], last_state["balance"], last_state["sequence"], last_state.get("delegation")),
            "cleanup.final", "final state is not linked to last inclusion")
    live = harness.require_fields(final["liveness"], {"generation", "block", "state", "mempool"}, "final liveness")
    require(live["generation"] == "restored", "cleanup.final", "final liveness generation differs")
    live_blocks = harness.require_fields(live["block"], {"go", "rs"}, "final liveness block")
    block_fields = {"height", "block_hash", "app_hash", "time", "observed_monotonic_ms"}
    go_block = harness.require_fields(live_blocks["go"], block_fields, "final Go block")
    rs_block = harness.require_fields(live_blocks["rs"], block_fields, "final Rust block")
    require({key: go_block[key] for key in block_fields - {"observed_monotonic_ms"}} ==
            {key: rs_block[key] for key in block_fields - {"observed_monotonic_ms"}},
            "cleanup.final", "final common block differs")
    require(isinstance(go_block["height"], int) and go_block["height"] > 0 and
            isinstance(go_block["block_hash"], str) and bool(go_block["block_hash"]) and
            isinstance(go_block["app_hash"], str) and bool(go_block["app_hash"]),
            "cleanup.final", "final common block fields differ")
    parse_time(go_block["time"], "final block time")
    restored_start, restored_end = generation_intervals["restored"]
    require(restored_start <= go_block["observed_monotonic_ms"] <= restored_end and
            restored_start <= rs_block["observed_monotonic_ms"] <= restored_end and
            abs(go_block["observed_monotonic_ms"] - rs_block["observed_monotonic_ms"]) <= 250,
            "cleanup.final", "final liveness observation is outside restored generation")
    raw_case = next(row for row in inputs["cases"] if row["name"] == "raw-malformed-protobuf")
    require(go_block["height"] > raw_case["pre_state"]["go"]["height"], "cleanup.final",
            "final liveness did not advance after malformed protobuf")
    live_states = harness.require_fields(live["state"], {"go", "rs"}, "final liveness state")
    state_fields = {"height", "app_hash", "account_number", "sequence", "balance", "delegation"}
    go_state = harness.require_fields(live_states["go"], state_fields, "final Go state")
    rs_state = harness.require_fields(live_states["rs"], state_fields, "final Rust state")
    require(go_state == rs_state and go_state["height"] == go_block["height"] and
            go_state["app_hash"] == go_block["app_hash"] and
            (go_state["app_hash"], go_state["balance"], go_state["sequence"], go_state["delegation"]) ==
            (final["go_root"], final["go_balance"], final["go_sequence"], final["go_delegation"]),
            "cleanup.final", "fresh final state binding differs")
    require(live["mempool"] == {"go": 0, "rs": 0}, "cleanup.final", "fresh final mempool differs")
    require(len(inclusions["cases"]) == len(harness.CASE_SPECS), "cleanup.final", "inclusion inventory differs")


def verify_blob(blob: dict[str, Any], inputs: dict[str, Any], receipts: dict[str, Any]) -> None:
    fields = {"request_id", "method", "request_count", "forward_count", "source_hash", "raw_request", "destination", "cli_argv", "cli_source_sha256", "envelope", "submitted_go_sha256", "submitted_rs_sha256", "inner_tx_sha256", "unwrapped_inner_sha256", "namespace", "data_sha256", "share_version", "blob_signer_hex", "cli_signer", "absent_before_go", "absent_before_rs", "read_methods"}
    harness.require_fields(blob, fields, "BlobTx proxy")
    require(blob["request_id"] == 0 and blob["method"] == "broadcast_tx_sync", "blob.proxy", "proxy request identity differs")
    require(blob["request_count"] == 1 and blob["forward_count"] == 0, "blob.proxy", "proxy request/forward count differs")
    require(blob["absent_before_go"] is True and blob["absent_before_rs"] is True, "blob.proxy", "pre-submit absence missing")
    require(blob["read_methods"] == [], "blob.proxy", "unexpected CLI read method reached proxy")
    harness.loopback_endpoint(blob["destination"], "BlobTx proxy destination")
    require(isinstance(blob["cli_argv"], list) and all(isinstance(value, str) for value in blob["cli_argv"]),
            "blob.cli", "CLI argv invalid")
    require(re.fullmatch(r"[0-9a-f]{64}", blob["cli_source_sha256"]) is not None,
            "blob.cli", "CLI source hash invalid")
    envelope = harness.decode_blob(blob["envelope"], "BlobTx envelope")
    raw_request = harness.decode_blob(blob["raw_request"], "BlobTx raw proxy request")
    request = harness.strict_json(raw_request)
    harness.require_fields(request, {"jsonrpc", "id", "method", "params"}, "BlobTx JSON-RPC request")
    require(request["jsonrpc"] == "2.0" and request["id"] == 0 and request["method"] == "broadcast_tx_sync",
            "blob.proxy", "raw JSON-RPC identity differs")
    params = harness.require_fields(request["params"], {"tx"}, "BlobTx JSON-RPC params")
    try:
        request_envelope = __import__("base64").b64decode(params["tx"], validate=True)
    except (ValueError, TypeError) as error:
        raise VerificationError("blob.proxy: invalid request transaction base64") from error
    require(request_envelope == envelope, "blob.proxy", "raw request envelope differs")
    digest = hashlib.sha256(envelope).hexdigest()
    require(digest == blob["submitted_go_sha256"] == blob["submitted_rs_sha256"], "blob.envelope", "submitted envelope differs")
    decoded = harness.decode_canonical_blob_tx(envelope)
    require(len(decoded["blobs"]) == 1, "blob.unwrap", "expected one blob")
    decoded_blob = decoded["blobs"][0]
    require(decoded_blob["namespace_version"] == 0 and
            decoded_blob["namespace_id"][:18] == b"\0" * 18 and
            decoded_blob["share_version"] == 0 and decoded_blob["signer"] == b"",
            "blob.unwrap", "CLI provenance requires a v0 namespace, share v0 and empty signer")
    inner_digest = hashlib.sha256(decoded["tx"]).hexdigest()
    require(blob["inner_tx_sha256"] == blob["unwrapped_inner_sha256"] == inner_digest,
            "blob.unwrap", "inner transaction differs")
    require(isinstance(blob["source_hash"], str) and re.fullmatch(r"[0-9a-f]{64}", blob["source_hash"]) is not None,
            "blob.source", "proxy source hash invalid")
    require(blob["source_hash"] == receipts["capture_sources"]["blob_proxy.py"],
            "blob.source", "proxy source is not the reviewed local implementation")
    require(blob["cli_source_sha256"] == receipts["go"]["cli_source_sha256"],
            "blob.cli", "CLI source hash differs from Go receipt")
    require(isinstance(blob["namespace"], str) and re.fullmatch(r"[0-9a-f]{58}", blob["namespace"]) is not None and
            isinstance(blob["data_sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", blob["data_sha256"]) is not None and
            blob["share_version"] == 0 and blob["blob_signer_hex"] == "" and
            isinstance(blob["cli_signer"], str) and blob["cli_signer"].startswith("celestia1"),
            "blob.fields", "blob fields differ")
    full_namespace = bytes([decoded_blob["namespace_version"]]) + decoded_blob["namespace_id"]
    require(full_namespace.hex() == blob["namespace"] and decoded_blob["share_version"] == blob["share_version"] and
            hashlib.sha256(decoded_blob["data"]).hexdigest() == blob["data_sha256"] and
            decoded_blob["signer"] == bytes.fromhex(blob["blob_signer_hex"]),
            "blob.unwrap", "decoded blob fields differ")
    names = {row["name"]: row for row in inputs["cases"]}
    prestate = names["blob-envelope"]["pre_state"]["go"]
    expected_argv = [
        "tx", "blob", "pay-for-blob", "0x" + decoded_blob["namespace_id"][-10:].hex(),
        "0x" + decoded_blob["data"].hex(), "--from", blob["cli_signer"],
        "--keyring-backend", "test", "--account-number", str(prestate["account_number"]),
        "--sequence", str(prestate["sequence"]), "--chain-id", "rs-devnet-187", "--offline",
        "--gas", "200000", "--fees", "100utia", "--node", blob["destination"], "--broadcast-mode", "sync",
        "--yes", "--output", "json",
    ]
    require(blob["cli_argv"] == expected_argv, "blob.cli", "CLI positional command or frozen flag/value differs")
    request = harness.decode_blob(names["blob-envelope"]["requests"]["go"], "paired BlobTx request")
    require(request == envelope, "blob.envelope", "proxy and paired envelope differ")


def verify_resources(resources: dict[str, Any], capture_started: dt.datetime,
                     capture_completed: dt.datetime, topology: dict[str, Any],
                     receipts: dict[str, Any]) -> dict[str, tuple[int, int]]:
    harness.require_fields(resources, {"processes", "samples", "generation_intervals", "disk_samples", "preflight_estimate_bytes", "preflight_free_bytes", "controller_started_monotonic_ms", "controller_completed_monotonic_ms", "process_group_drained", "process_group_term_sent", "process_group_kill_sent"}, "resources")
    require(resources["process_group_drained"] is True, "resource.process_group", "capture process group was not drained")
    require(isinstance(resources["process_group_term_sent"], bool) and isinstance(resources["process_group_kill_sent"], bool),
            "resource.process_group", "process group drain flags invalid")
    processes = resources["processes"]
    expected_identities = {(generation, role) for generation in ("normal", "timeout", "restored") for role in ("go", "rs")}
    require(isinstance(processes, list) and {(row.get("generation"), row.get("role")) for row in processes} == expected_identities,
            "resource.process", "process generation/role inventory differs")
    for row in processes:
        harness.require_fields(row, {"generation", "role", "pid_start_ticks", "executable_sha256"}, "resource process")
        require(row["pid_start_ticks"] > 0 and len(row["executable_sha256"]) == 64, "resource.process", row["role"])
        expected_hash = receipts["go" if row["role"] == "go" else "rust"]["binary_sha256"]
        listener_ticks = {listener["pid_start_ticks"] for listener in topology["generations"][row["generation"]]["listeners"]
                          if listener["name"].startswith(f"{row['role']}_")}
        require(row["executable_sha256"] == expected_hash and listener_ticks == {row["pid_start_ticks"]},
                "resource.identity", f"{row['generation']}/{row['role']} process differs from listener/build receipt")
    samples = resources["samples"]
    require(isinstance(samples, list) and samples, "resource.samples", "resource samples empty")
    previous = -1
    for sample in samples:
        harness.require_fields(sample, {"monotonic_ms", "generation", "role", "pid_start_ticks", "executable_sha256", "rss_bytes", "threads", "fds"}, "resource sample")
        require(sample["monotonic_ms"] >= previous, "resource.order", "resource timestamps regress")
        previous = sample["monotonic_ms"]
        process = next(row for row in processes if row["generation"] == sample["generation"] and row["role"] == sample["role"])
        require((sample["pid_start_ticks"], sample["executable_sha256"]) == (process["pid_start_ticks"], process["executable_sha256"]),
                "resource.identity", sample["role"])
    controller_start = resources["controller_started_monotonic_ms"]
    controller_end = resources["controller_completed_monotonic_ms"]
    interval_rows = harness.require_fields(resources["generation_intervals"], {"normal", "timeout", "restored"}, "resource generation intervals")
    intervals: dict[str, tuple[int, int]] = {}
    previous_end = controller_start
    for generation in ("normal", "timeout", "restored"):
        interval = harness.require_fields(interval_rows[generation], {"started_monotonic_ms", "completed_monotonic_ms"}, f"resource {generation} interval")
        start, end = interval["started_monotonic_ms"], interval["completed_monotonic_ms"]
        require(isinstance(start, int) and isinstance(end, int) and previous_end <= start < end <= controller_end,
                "resource.capture", f"{generation} interval is invalid/overlapping")
        intervals[generation] = (start, end)
        previous_end = end
        for role in ("go", "rs"):
            role_samples = [sample for sample in samples if sample["generation"] == generation and sample["role"] == role]
            require(len(role_samples) >= 2, "resource.cadence", f"too few {generation}/{role} samples")
            require(all(0 < right["monotonic_ms"] - left["monotonic_ms"] <= 2500 for left, right in zip(role_samples, role_samples[1:])),
                    "resource.cadence", f"{generation}/{role} resource cadence exceeds 2.5s")
            require(start <= role_samples[0]["monotonic_ms"] <= start + 2500 and
                    end - 2500 <= role_samples[-1]["monotonic_ms"] <= end,
                    "resource.capture", f"{generation}/{role} samples do not span generation interval")
    disk = resources["disk_samples"]
    require(isinstance(disk, list) and len(disk) >= 2, "resource.disk", "disk samples missing")
    for row in disk:
        harness.require_fields(row, {"monotonic_ms", "free_bytes"}, "disk sample")
    for left, right in zip(disk, disk[1:]):
        require(0 < right["monotonic_ms"] - left["monotonic_ms"] <= 2500, "resource.disk", "disk cadence exceeds 2.5s")
    require(all(row["free_bytes"] >= harness.MIN_FREE_BYTES for row in disk), "resource.disk", "disk floor crossed")
    require(controller_start <= disk[0]["monotonic_ms"] <= controller_start + 2500 and
            controller_end - 2500 <= disk[-1]["monotonic_ms"] <= controller_end,
            "resource.capture", "disk samples do not span capture controller interval")
    require(resources["preflight_free_bytes"] - resources["preflight_estimate_bytes"] >= harness.MIN_FREE_BYTES,
            "resource.preflight", "preflight estimate crosses disk floor")
    require(capture_started <= capture_completed, "capture.order", "capture timestamps regress")
    wall_elapsed_ms = int((capture_completed - capture_started).total_seconds() * 1000)
    require(abs((controller_end - controller_start) - wall_elapsed_ms) <= 5000,
            "resource.capture", "monotonic and wall-clock capture durations differ")
    return intervals


def verify(root: pathlib.Path) -> None:
    harness.reject_symlink_components(root, "capture root")
    root = root.resolve(strict=True)
    manifest = load(root / "manifest.json")
    harness.require_fields(manifest, {"schema", "cases", "files", "support"}, "manifest")
    require(manifest["schema"] == harness.SCHEMA, "manifest.schema", "schema differs")
    try:
        harness.validate_case_inventory(manifest["cases"])
    except harness.HarnessError as error:
        raise VerificationError(f"case.manifest: {error}") from error
    support = harness.require_fields(manifest["support"], {"capture_started", "capture_completed", "candidate_commit", "normalizations", "intentional_differences", "exclusions"}, "manifest support")
    require(support["normalizations"] == ["lowercase-header-names", "trim-optional-whitespace", "sort-declared-multivalue-headers"],
            "manifest.normalization", "normalizations differ")
    require(support["intentional_differences"] == ["raw-malformed-protobuf"], "manifest.difference", "intentional differences differ")
    require(support["exclusions"] == ["http.date", "http.x-server-time"], "manifest.exclusion", "exclusions differ")
    capture_started = parse_time(support["capture_started"], "capture start")
    capture_completed = parse_time(support["capture_completed"], "capture completion")
    require(re.fullmatch(r"[0-9a-f]{40}", support["candidate_commit"]) is not None, "manifest.commit", "candidate commit invalid")
    safe_inventory(root, manifest)
    observation = load(root / "mainnet-observation.json")
    receipts = load(root / "build-receipts.json")
    topology = load(root / "topology.json")
    genesis = load(root / "genesis.json")
    configs = load(root / "configs.json")
    inputs = load(root / "inputs.json")
    responses = load(root / "paired-responses.json")
    inclusions = load(root / "inclusions.json")
    cleanup = load(root / "cleanup.json")
    resources = load(root / "resources.json")
    blob = load(root / "blob-proxy.json")
    verify_observation(observation, capture_started)
    verify_receipts(receipts, support)
    verify_topology(topology, receipts)
    verify_genesis(genesis)
    verify_configs(configs, topology)
    generation_intervals = verify_resources(resources, capture_started, capture_completed, topology, receipts)
    last_state = verify_cases(inputs, responses, inclusions, cleanup, topology["max_txs"],
                              topology["max_txs_bytes"], receipts, generation_intervals)
    verify_blob(blob, inputs, receipts)
    verify_cleanup(cleanup, inputs, responses, inclusions, receipts, last_state, generation_intervals)
    require(re.fullmatch(r"[0-9a-f]{64}", EXPECTED_SEMANTIC_DIGEST) is not None,
            "semantic.digest", "reviewed semantic digest is unset")
    require(semantic_digest(root) == EXPECTED_SEMANTIC_DIGEST,
            "semantic.digest", "semantic evidence differs from reviewed digest")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture_root", type=pathlib.Path)
    args = parser.parse_args()
    try:
        verify(args.capture_root)
    except (VerificationError, harness.HarnessError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print("PASS: paired live-transaction closure evidence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
