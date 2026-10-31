#!/usr/bin/env python3
"""Receipt-bound outer runtime campaign for issue #187.

This controller has no child-command input.  A strict plan names reviewed
executables, immutable request bytes, complete node configurations, and seed
receipts.  The controller itself owns all starts, restarts, broadcasts, read
queries, resource sampling, and artifact writes.
"""

from __future__ import annotations

import argparse
import base64
import copy
import concurrent.futures
import datetime as dt
import hashlib
import importlib.util
import ipaddress
import os
import pathlib
import signal
import socket
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from types import ModuleType
from typing import Any

import harness


GENERATIONS = ("normal", "timeout", "restored")
ROLES = ("go", "rs")
SEED_FILES = ("mainnet-observation.json", "build-receipts.json", "genesis.json", "configs.json", "blob-proxy.json")
ENDPOINT_NAMES = {f"{role}_{kind}" for role in ROLES for kind in ("rpc", "core", "sdk", "rest", "p2p")}
LIFECYCLE_INPUTS = {"mempool_followup", "disconnect_followup", "grpc_cancel_followup",
                    "server_timeout_probe", "server_timeout_followup"}
CLIENT_FIELDS = {"transport", "wire", "http_status", "http_headers", "json", "grpc_status", "grpc_message",
                 "initial_metadata", "trailing_metadata", "websocket_frames", "websocket_handshake",
                 "client_error", "process_effect"}
CONFIG_FIELDS = {
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
        "tx_index.indexer": ("tx_index", "indexer"), "consensus.timeout_commit": ("consensus", "timeout_commit"),
    },
    "app.toml": {"grpc.enable": ("grpc", "enable"), "grpc.address": ("grpc", "address"),
                 "grpc-web.enable": ("grpc-web", "enable"),
                 "api.enable": ("api", "enable"), "api.address": ("api", "address")},
}


def now_ms() -> int:
    return time.monotonic_ns() // 1_000_000


def checked_executable(path: pathlib.Path, digest: str, label: str) -> pathlib.Path:
    harness.reject_symlink_components(path, label)
    resolved = path.resolve(strict=True)
    harness.require(resolved.is_file() and not resolved.is_symlink(), f"{label} is not a regular file")
    harness.require(hashlib.sha256(resolved.read_bytes()).hexdigest() == digest, f"{label} hash differs")
    return resolved


def checked_seed(record: Any, label: str) -> pathlib.Path:
    row = harness.require_fields(record, {"path", "sha256"}, label)
    path = pathlib.Path(row["path"])
    harness.reject_symlink_components(path, label)
    path = path.resolve(strict=True)
    harness.require(path.is_file() and not path.is_symlink(), f"{label} is not a regular file")
    harness.require(hashlib.sha256(path.read_bytes()).hexdigest() == row["sha256"], f"{label} hash differs")
    return path


def load_reviewed_module(path: pathlib.Path, expected_sha256: str, name: str) -> ModuleType:
    """Load only bytes that have already matched the reviewed receipt."""
    harness.reject_symlink_components(path, f"{name} source")
    path = path.resolve(strict=True)
    harness.require(path.is_file() and not path.is_symlink(), f"{name} source is not a regular file")
    source = path.read_bytes()
    harness.require(hashlib.sha256(source).hexdigest() == expected_sha256, f"{name} source hash differs")
    spec = importlib.util.spec_from_file_location(f"live_tx_closure_{name}", path)
    harness.require(spec is not None, f"{name} module specification failed")
    module = importlib.util.module_from_spec(spec)
    exec(compile(source, str(path), "exec"), module.__dict__)
    return module


def write_new(path: pathlib.Path, value: Any) -> None:
    harness.require(path.parent.is_dir() and not path.exists(), f"campaign refuses to replace {path.name}")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(harness.canonical_json(value))


def replace_config(path: pathlib.Path, data: bytes) -> None:
    temp = path.with_name(f".{path.name}.campaign-{os.getpid()}")
    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def load_plan(path: pathlib.Path) -> dict[str, Any]:
    harness.reject_symlink_components(path, "campaign plan")
    plan = harness.strict_json(path.read_bytes())
    harness.require_fields(plan, {
        "schema", "output", "go_binary", "go_sha256", "rust_binary", "rust_sha256",
        "transport_client", "transport_client_sha256", "driver_sha256", "homes", "inputs",
        "lifecycle_inputs", "capacity", "endpoints", "config_files", "seed_artifacts", "state",
    }, "campaign plan")
    harness.require(plan["schema"] == harness.SCHEMA, "campaign plan schema differs")
    harness.require_fields(plan["homes"], GENERATIONS, "campaign homes")
    harness.require(set(plan["inputs"]) == set(harness.CASE_NAMES), "campaign input inventory differs")
    harness.require_fields(plan["lifecycle_inputs"], LIFECYCLE_INPUTS, "campaign lifecycle inputs")
    harness.require_fields(plan["capacity"], {"tx_a", "tx_b", "mempool_size", "max_txs", "max_txs_bytes"}, "campaign capacity")
    harness.require_fields(plan["endpoints"], ENDPOINT_NAMES, "campaign endpoints")
    harness.validate_endpoints(plan["endpoints"])
    harness.require_fields(plan["config_files"], GENERATIONS, "campaign config files")
    harness.require_fields(plan["seed_artifacts"], SEED_FILES, "campaign seed artifacts")
    harness.require_fields(plan["state"], {"address", "denom", "validator"}, "campaign state")
    return plan


def fixed_node_command(binary: pathlib.Path, role: str, home: pathlib.Path) -> list[str]:
    return ([str(binary), "start", "--force-no-bbr", "--home", str(home)] if role == "go"
            else [str(binary), "start", "--home", str(home)])


def toml_projection(raw: bytes, fields: dict[str, tuple[str, ...]], label: str) -> dict[str, Any]:
    parsed = tomllib.loads(raw.decode("utf-8"))
    result: dict[str, Any] = {}
    for name, keys in fields.items():
        value: Any = parsed
        for key in keys:
            harness.require(isinstance(value, dict) and key in value, f"{label} lacks {name}")
            value = value[key]
        result[name] = value
    return result


def normalize_timeout_config(value: dict[str, Any], normal: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(value)
    result["rpc"]["timeout_broadcast_tx_commit"] = normal["rpc"]["timeout_broadcast_tx_commit"]
    return result


def validate_home_provenance(homes: dict[str, pathlib.Path], genesis: dict[str, Any]) -> None:
    consensus = harness.require_fields(genesis.get("consensus"), {"params", "validators"}, "genesis consensus")
    validators = {row["name"]: row for row in consensus["validators"]}
    harness.require(set(validators) == set(ROLES), "genesis validator roles differ")
    for role in ROLES:
        genesis_path = homes[role] / "config" / "genesis.json"
        key_path = homes[role] / "config" / "priv_validator_key.json"
        harness.reject_symlink_components(genesis_path, f"{role} home genesis")
        harness.reject_symlink_components(key_path, f"{role} validator key")
        harness.require(genesis_path.is_file() and key_path.is_file(), f"{role} home provenance files are missing")
        home_genesis = harness.strict_json(genesis_path.read_bytes())
        harness.require(home_genesis == genesis, f"{role} home genesis differs from public receipt")
        validator_key = harness.strict_json(key_path.read_bytes())
        harness.require(validator_key.get("address") == validators[role]["address"] and
                        validator_key.get("pub_key") == validators[role]["pub_key"],
                        f"{role} validator key does not match public genesis")


def consensus_bounds_from_observation(observation: dict[str, Any]) -> tuple[int, int]:
    observation_responses = harness.require_fields(observation.get("responses"), {"status", "abci_info"},
                                                   "mainnet observation responses")
    abci_record = harness.require_fields(observation_responses["abci_info"], {"body", "sha256"},
                                         "mainnet observation ABCI response")
    abci_body = harness.strict_json(abci_record["body"])
    abci_result = harness.require_fields(abci_body.get("result"), {"response"},
                                         "mainnet observation ABCI result")
    abci_response = harness.require_fields(abci_result["response"], harness.ABCI_INFO_RESPONSE_FIELDS,
                                           "mainnet observation ABCI payload")
    return (harness.consensus_freshness_bound_ms(abci_response["timeout_info"]),
            harness.consensus_commit_window_ms(abci_response["timeout_info"]))


def next_common_height(minimum: int, initial_heights: list[int]) -> int:
    harness.require(isinstance(minimum, int) and minimum > 0 and
                    len(initial_heights) == len(ROLES) and
                    all(isinstance(height, int) and height >= 0 for height in initial_heights),
                    "common block initial heights are invalid")
    return max(minimum, max(initial_heights) + 1)


def validate_plan(plan: dict[str, Any]) -> dict[str, Any]:
    binaries = {
        "go": checked_executable(pathlib.Path(plan["go_binary"]), plan["go_sha256"], "Go node"),
        "rs": checked_executable(pathlib.Path(plan["rust_binary"]), plan["rust_sha256"], "Rust node"),
    }
    client = checked_executable(pathlib.Path(plan["transport_client"]), plan["transport_client_sha256"], "transport client")
    driver = pathlib.Path(__file__).with_name("driver.py").resolve(strict=True)
    harness.require(hashlib.sha256(driver.read_bytes()).hexdigest() == plan["driver_sha256"], "driver source hash differs")
    output = pathlib.Path(plan["output"])
    harness.reject_symlink_components(output, "campaign output")
    output = output.resolve(strict=True)
    harness.require(output.is_dir() and not any(output.iterdir()), "campaign output is not the empty controller-owned directory")
    root = os.environ.get("LIVE_TX_CAPTURE_ROOT")
    harness.require(root is not None and pathlib.Path(root).resolve(strict=True) == output,
                    "campaign output is not bound to LIVE_TX_CAPTURE_ROOT")

    homes: dict[str, pathlib.Path] = {}
    for generation in GENERATIONS:
        row = harness.require_fields(plan["homes"][generation], ROLES, f"{generation} homes")
        for role in ROLES:
            path = pathlib.Path(row[role])
            harness.reject_symlink_components(path, f"{generation}/{role} home")
            path = path.resolve(strict=True)
            harness.require(path.is_dir(), f"{generation}/{role} home is missing")
            if role in homes:
                harness.require(homes[role] == path, f"{role} restart changed persistent data home")
            homes[role] = path
    for name, value in plan["inputs"].items():
        harness.decode_blob(value, f"case input {name}")
    for name, value in plan["lifecycle_inputs"].items():
        harness.decode_blob(value, f"lifecycle input {name}")
    tx_a = harness.decode_blob(plan["capacity"]["tx_a"], "capacity A")
    tx_b = harness.decode_blob(plan["capacity"]["tx_b"], "capacity B")
    max_txs = plan["capacity"]["max_txs"]
    mempool_size = plan["capacity"]["mempool_size"]
    maximum = plan["capacity"]["max_txs_bytes"]
    harness.require(mempool_size == 0 and max_txs == 1 and maximum == harness.MEMPOOL_MAX_TXS_BYTES and
                    tx_a != tx_b and 0 < len(tx_a) <= maximum and 0 < len(tx_b) <= maximum,
                    "capacity inputs do not exercise the configured boundary")
    harness.require(tx_a == harness.decode_blob(plan["inputs"]["mempool-capacity-recovery"], "capacity case"),
                    "capacity A differs from main case request")

    seeds = {name: checked_seed(plan["seed_artifacts"][name], f"seed {name}") for name in SEED_FILES}
    observation = harness.strict_json(seeds["mainnet-observation.json"].read_bytes())
    freshness_bound_ms, commit_window_ms = consensus_bounds_from_observation(observation)
    configs = harness.strict_json(seeds["configs.json"].read_bytes())
    harness.require_fields(configs, {*GENERATIONS, "full"}, "configs receipt")
    full_configs = harness.require_fields(configs["full"], GENERATIONS, "full configs receipt")
    for generation in GENERATIONS:
        sources = harness.require_fields(plan["config_files"][generation], ROLES, f"{generation} config sources")
        full_generation = harness.require_fields(full_configs[generation], ROLES, f"{generation} full configs")
        for role in ROLES:
            files = harness.require_fields(sources[role], CONFIG_FIELDS, f"{generation}/{role} config sources")
            full_files = harness.require_fields(full_generation[role], CONFIG_FIELDS, f"{generation}/{role} full configs")
            for filename, fields in CONFIG_FIELDS.items():
                raw = harness.decode_blob(files[filename], f"{generation}/{role}/{filename}")
                parsed = tomllib.loads(raw.decode("utf-8"))
                harness.require(parsed == full_files[filename], f"{generation}/{role}/{filename} full config differs")
                harness.require(toml_projection(raw, fields, f"{generation}/{role}/{filename}") == configs[generation][role][filename],
                                f"{generation}/{role}/{filename} projection differs from receipt")
            harness.require(
                configs[generation][role]["config.toml"]["priv_validator_grpc_laddr"] == "",
                f"{generation}/{role} priv_validator_grpc_laddr must be disabled",
            )
            harness.require(
                configs[generation][role]["config.toml"]["rpc.max_subscription_clients"] ==
                harness.SUBSCRIPTION_CLIENT_LIMITS[role]
                and configs[generation][role]["config.toml"]["rpc.max_subscriptions_per_client"] == 1,
                f"{generation}/{role} effective external subscription capacity differs",
            )
    harness.validate_config_delta(configs["normal"], configs["timeout"])
    harness.require(configs["restored"] == configs["normal"], "restored config differs from normal")
    for role in ROLES:
        harness.require(full_configs["restored"][role] == full_configs["normal"][role],
                        f"{role} restored full config differs from normal")
        harness.require(full_configs["timeout"][role]["app.toml"] == full_configs["normal"][role]["app.toml"] and
                        normalize_timeout_config(full_configs["timeout"][role]["config.toml"],
                                                 full_configs["normal"][role]["config.toml"]) ==
                        full_configs["normal"][role]["config.toml"],
                        f"{role} timeout full config has an undeclared delta")
    harness.require(maximum == configs["normal"]["go"]["config.toml"]["mempool.max_txs_bytes"] ==
                    configs["normal"]["rs"]["config.toml"]["mempool.max_txs_bytes"],
                    "capacity plan/config maximum differs")
    harness.require(mempool_size == configs["normal"]["go"]["config.toml"]["mempool.size"] ==
                    configs["normal"]["rs"]["config.toml"]["mempool.size"],
                    "capacity plan/config transaction count differs")
    for generation in GENERATIONS:
        for role in ROLES:
            config = configs[generation][role]["config.toml"]
            app = configs[generation][role]["app.toml"]
            harness.require(config["rpc.laddr"] == plan["endpoints"][f"{role}_rpc"] and
                            config["rpc.grpc_laddr"] == plan["endpoints"][f"{role}_core"] and
                            config["p2p.laddr"] == plan["endpoints"][f"{role}_p2p"] and
                            app["grpc.address"] == plan["endpoints"][f"{role}_sdk"] and
                            app["api.address"] == plan["endpoints"][f"{role}_rest"] and
                            app["grpc-web.enable"] is False,
                            f"{generation}/{role} configured endpoints differ from plan")

    receipts = harness.strict_json(seeds["build-receipts.json"].read_bytes())
    harness.require(receipts["go"]["binary_sha256"] == plan["go_sha256"] and
                    receipts["rust"]["binary_sha256"] == plan["rust_sha256"] and
                    receipts["transport_client"]["binary_sha256"] == plan["transport_client_sha256"],
                    "binary hashes differ from build receipts")
    here = pathlib.Path(__file__).parent
    for name in ("harness.py", "verify.py", "capture.py", "driver.py", "blob_proxy.py", "campaign.py"):
        harness.require(receipts["capture_sources"][name] == hashlib.sha256((here / name).read_bytes()).hexdigest(),
                        f"capture source receipt differs: {name}")
    harness.require(plan["driver_sha256"] == receipts["capture_sources"]["driver.py"],
                    "driver plan/receipt source hash differs")
    driver_module = load_reviewed_module(driver, receipts["capture_sources"]["driver.py"], "driver")
    blob_proxy_module = load_reviewed_module(here / "blob_proxy.py", receipts["capture_sources"]["blob_proxy.py"],
                                             "blob_proxy")
    genesis = harness.strict_json(seeds["genesis.json"].read_bytes())
    validate_home_provenance(homes, genesis)
    harness.require(all(isinstance(value, str) and value for value in plan["state"].values()), "state query identity is empty")
    return {"binaries": binaries, "client": client, "driver": driver, "output": output,
            "driver_module": driver_module, "blob_proxy_module": blob_proxy_module,
            "homes": homes, "seeds": seeds, "configs": configs, "receipts": receipts,
            "freshness_bound_ms": freshness_bound_ms, "commit_window_ms": commit_window_ms}


def rpc_url(endpoint: str, path: str, query: dict[str, str] | None = None) -> str:
    parsed = urllib.parse.urlsplit(endpoint if "://" in endpoint else f"tcp://{endpoint}")
    return urllib.parse.urlunsplit(("http", parsed.netloc, path, urllib.parse.urlencode(query or {}), ""))


FUTURE_HEIGHT_MESSAGE = ("codespace sdk code 26: invalid height: cannot query with height in the future; "
                         "please provide a valid height")


def future_height_error(raw: bytes) -> bool:
    try:
        value = harness.strict_json(raw)
    except (UnicodeError, harness.HarnessError, ValueError):
        return False
    return value == {"code": 2, "message": FUTURE_HEIGHT_MESSAGE, "details": []}


def get_json(url: str, label: str, *, height: int | None = None, missing: bool = False,
             _future_height_timeout: float = 5) -> Any:
    harness.loopback_endpoint(url, label)
    request = urllib.request.Request(url, method="GET")
    if height is not None:
        request.add_header("x-cosmos-block-height", str(height))
    deadline = time.monotonic() + _future_height_timeout
    while True:
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                raw = response.read(16 * 1024 * 1024 + 1)
                harness.require(response.status == 200 and len(raw) <= 16 * 1024 * 1024,
                                f"{label} response differs")
            return harness.strict_json(raw)
        except urllib.error.HTTPError as error:
            error_raw = error.read(16 * 1024 * 1024 + 1)
            future = (height is not None and error.code == 500 and
                      len(error_raw) <= 16 * 1024 * 1024 and future_height_error(error_raw))
            if future and time.monotonic() < deadline:
                time.sleep(0.05)
                continue
            if future:
                raise harness.HarnessError(f"{label} future height remained unavailable") from error
            if missing and error.code in {400, 404, 500}:
                return None
            raise harness.HarnessError(f"{label} HTTP {error.code}") from error


def status(endpoint: str) -> tuple[int, Any]:
    raw = get_json(rpc_url(endpoint, "/status"), "status")
    return int(raw["result"]["sync_info"]["latest_block_height"]), raw


def block(endpoint: str, height: int) -> tuple[dict[str, Any], Any]:
    raw = get_json(rpc_url(endpoint, "/block", {"height": str(height)}), "block")
    result, observed = raw["result"], now_ms()
    header = result["block"]["header"]
    return {"height": int(header["height"]), "block_hash": result["block_id"]["hash"],
            "app_hash": header["app_hash"], "time": header["time"], "observed_monotonic_ms": observed,
            "_observed_wall": dt.datetime.now(dt.timezone.utc)}, raw


def common_block(endpoints: dict[str, str], minimum: int, freshness_bound_ms: int,
                 timeout: float = 30) -> dict[str, Any]:
    harness.require(isinstance(freshness_bound_ms, int) and freshness_bound_ms > 0,
                    "common block freshness bound is invalid")
    initial_heights = [status(endpoints[f"{side}_rpc"])[0] for side in ROLES]
    target = next_common_height(minimum, initial_heights)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        height = min(status(endpoints[f"{side}_rpc"])[0] for side in ROLES)
        if height >= target:
            rows = {side: block(endpoints[f"{side}_rpc"], height)[0] for side in ROLES}
            stable = lambda row: {key: row[key] for key in ("height", "block_hash", "app_hash", "time")}
            block_time = dt.datetime.fromisoformat(rows["go"]["time"].replace("Z", "+00:00"))
            age_ms = int((rows["go"]["_observed_wall"] - block_time).total_seconds() * 1000)
            if (stable(rows["go"]) == stable(rows["rs"]) and
                    abs(rows["go"]["observed_monotonic_ms"] - rows["rs"]["observed_monotonic_ms"]) <= 250 and
                    -250 <= age_ms <= freshness_bound_ms):
                return rows
        time.sleep(0.05)
    raise harness.HarnessError("common block query timed out")


def common_block_at(endpoints: dict[str, str], height: int, timeout: float = 15) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if all(status(endpoints[f"{side}_rpc"])[0] >= height for side in ROLES):
            rows = {side: block(endpoints[f"{side}_rpc"], height)[0] for side in ROLES}
            stable = lambda row: {key: row[key] for key in ("height", "block_hash", "app_hash", "time")}
            if stable(rows["go"]) == stable(rows["rs"]) and abs(
                    rows["go"]["observed_monotonic_ms"] - rows["rs"]["observed_monotonic_ms"]) <= 250:
                return rows
        time.sleep(0.05)
    raise harness.HarnessError(f"common block {height} query timed out")


def mempool(endpoint: str) -> tuple[list[str], Any]:
    raw = get_json(rpc_url(endpoint, "/unconfirmed_txs", {"limit": "100"}), "mempool")
    result = raw["result"]
    txs = result.get("txs") or []
    harness.require(int(result.get("n_txs", len(txs))) == len(txs), "raw mempool count differs")
    return [hashlib.sha256(base64.b64decode(value, validate=True)).hexdigest() for value in txs], raw


def indexed_tx(endpoint: str, digest: str) -> tuple[dict[str, Any], Any] | None:
    raw = get_json(rpc_url(endpoint, "/tx", {"hash": "0x" + digest, "prove": "false"}), "indexed tx", missing=True)
    return None if raw is None or raw.get("error") is not None else (raw["result"], raw)


def base_account(value: Any) -> dict[str, Any]:
    if isinstance(value, dict):
        if "account_number" in value and "sequence" in value:
            return value
        for child in value.values():
            try:
                return base_account(child)
            except harness.HarnessError:
                pass
    raise harness.HarnessError("account response lacks a base account")


def query_state(plan: dict[str, Any], side: str, height: int, app_hash: str) -> dict[str, Any]:
    configured = plan["endpoints"][f"{side}_rest"]
    parsed = urllib.parse.urlsplit(configured if "://" in configured else f"tcp://{configured}")
    root = urllib.parse.urlunsplit(("http", parsed.netloc, "", "", "")).rstrip("/")
    address, denom, validator = (plan["state"][key] for key in ("address", "denom", "validator"))
    account = base_account(get_json(f"{root}/cosmos/auth/v1beta1/accounts/{address}", f"{side} account", height=height))
    balance_raw = get_json(f"{root}/cosmos/bank/v1beta1/balances/{address}/by_denom?denom={urllib.parse.quote(denom)}",
                           f"{side} balance", height=height)
    delegation_raw = get_json(f"{root}/cosmos/staking/v1beta1/validators/{validator}/delegations/{address}",
                              f"{side} delegation", height=height, missing=True)
    balance = str((balance_raw.get("balance") or {}).get("amount", "0"))
    delegation = "0" if delegation_raw is None else str(
        ((delegation_raw.get("delegation_response") or {}).get("balance") or {}).get("amount", "0"))
    harness.require(balance.isdigit() and delegation.isdigit(), f"{side} state amounts differ")
    return {"height": height, "app_hash": app_hash, "account_number": int(account["account_number"]),
            "sequence": int(account["sequence"]), "balance": balance, "delegation": delegation}


def endpoint_for(endpoints: dict[str, str], side: str, transport: str) -> str:
    kind = {"rest": "rest", "cosmos-grpc": "sdk", "core-grpc": "core", "comet-jsonrpc": "rpc",
            "comet-uri": "rpc", "comet-websocket": "rpc"}[transport]
    value = endpoints[f"{side}_{kind}"]
    parsed = urllib.parse.urlsplit(value if "://" in value else f"tcp://{value}")
    if transport == "comet-websocket":
        return urllib.parse.urlunsplit(("ws", parsed.netloc, "/websocket", "", ""))
    if transport in {"comet-jsonrpc", "comet-uri", "rest"}:
        return urllib.parse.urlunsplit(("http", parsed.netloc, "", "", ""))
    return parsed.netloc


def pid_ticks(pid: int) -> int:
    return int(pathlib.Path(f"/proc/{pid}/stat").read_text().split()[21])


def resource_sample(generation: str, role: str, process: subprocess.Popen[bytes], digest: str) -> dict[str, Any]:
    statm = pathlib.Path(f"/proc/{process.pid}/statm").read_text().split()
    status_lines = pathlib.Path(f"/proc/{process.pid}/status").read_text().splitlines()
    threads = int(next(line.split()[1] for line in status_lines if line.startswith("Threads:")))
    return {"monotonic_ms": now_ms(), "generation": generation, "role": role,
            "pid_start_ticks": pid_ticks(process.pid), "executable_sha256": digest,
            "rss_bytes": int(statm[1]) * os.sysconf("SC_PAGE_SIZE"), "threads": threads,
            "fds": len(list(pathlib.Path(f"/proc/{process.pid}/fd").iterdir()))}


class Sampler:
    def __init__(self, generation: str, processes: dict[str, subprocess.Popen[bytes]], digests: dict[str, str]) -> None:
        self.generation, self.processes, self.digests = generation, processes, digests
        self.rows: list[dict[str, Any]] = []
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self._run, name=f"sample-{generation}", daemon=True)

    def take(self) -> None:
        self.rows.extend(resource_sample(self.generation, role, self.processes[role], self.digests[role]) for role in ROLES)

    def _run(self) -> None:
        while not self.stopping.is_set():
            self.take()
            self.stopping.wait(2)

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> list[dict[str, Any]]:
        self.stopping.set()
        self.thread.join(timeout=3)
        self.take()
        return self.rows


@dataclass
class Generation:
    name: str
    processes: dict[str, subprocess.Popen[bytes]]
    sampler: Sampler
    started_ms: int


@dataclass
class State:
    plan: dict[str, Any]
    checked: dict[str, Any]
    campaign_sha: str
    cases: list[dict[str, Any]] = field(default_factory=list)
    responses: list[dict[str, Any]] = field(default_factory=list)
    inclusions: list[dict[str, Any]] = field(default_factory=list)
    cleanup: dict[str, Any] = field(default_factory=dict)
    topology_generations: dict[str, Any] = field(default_factory=dict)
    process_rows: list[dict[str, Any]] = field(default_factory=list)
    sample_rows: list[dict[str, Any]] = field(default_factory=list)
    intervals: dict[str, Any] = field(default_factory=dict)
    previous_pre_height: int = 0
    last_state: dict[str, Any] | None = None
    blob_artifact: dict[str, Any] | None = None


def install_configs(state: State, generation: str) -> None:
    for role in ROLES:
        directory = state.checked["homes"][role] / "config"
        harness.require(directory.is_dir() and not directory.is_symlink(), f"{role} config directory differs")
        for filename in CONFIG_FIELDS:
            raw = harness.decode_blob(state.plan["config_files"][generation][role][filename],
                                      f"{generation}/{role}/{filename}")
            replace_config(directory / filename, raw)


def stop_processes(processes: dict[str, subprocess.Popen[bytes]]) -> None:
    for process in processes.values():
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
    deadline = time.monotonic() + 10
    for process in processes.values():
        try:
            process.wait(timeout=max(0.0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def wait_ready(endpoints: dict[str, str], processes: dict[str, subprocess.Popen[bytes]]) -> None:
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        harness.require(all(process.poll() is None for process in processes.values()), "node exited during startup")
        try:
            for role in ROLES:
                status(endpoints[f"{role}_rpc"])
            for name, endpoint in endpoints.items():
                host, port = harness.loopback_endpoint(endpoint, name)
                with socket.create_connection((host, port), timeout=0.25):
                    pass
            return
        except (OSError, KeyError, ValueError, harness.HarnessError):
            time.sleep(0.1)
    raise harness.HarnessError("node listeners did not become ready")


def owned_listeners(pid: int) -> set[tuple[str, int]]:
    inodes: set[str] = set()
    for descriptor in pathlib.Path(f"/proc/{pid}/fd").iterdir():
        try:
            target = os.readlink(descriptor)
        except FileNotFoundError:
            continue
        if target.startswith("socket:[") and target.endswith("]"):
            inodes.add(target[8:-1])
    listeners: set[tuple[str, int]] = set()
    tables = ((pathlib.Path(f"/proc/{pid}/net/tcp"), 4),
              (pathlib.Path(f"/proc/{pid}/net/tcp6"), 16))
    for table, size in tables:
        for line in table.read_text().splitlines()[1:]:
            columns = line.split()
            if len(columns) >= 10 and columns[3] == "0A" and columns[9] in inodes:
                encoded_host, encoded_port = columns[1].rsplit(":", 1)
                packed = bytes.fromhex(encoded_host)
                if size == 4:
                    packed = packed[::-1]
                else:
                    packed = b"".join(packed[offset:offset + 4][::-1] for offset in range(0, 16, 4))
                listeners.add((str(ipaddress.ip_address(packed)), int(encoded_port, 16)))
    return listeners


def start_generation(state: State, generation: str) -> Generation:
    install_configs(state, generation)
    for role in ROLES:
        for filename in CONFIG_FIELDS:
            expected = harness.decode_blob(state.plan["config_files"][generation][role][filename],
                                           f"{generation}/{role}/{filename}")
            harness.require((state.checked["homes"][role] / "config" / filename).read_bytes() == expected,
                            f"{generation}/{role}/{filename} installed bytes differ")
    processes: dict[str, subprocess.Popen[bytes]] = {}
    try:
        for role in ROLES:
            command = fixed_node_command(state.checked["binaries"][role], role, state.checked["homes"][role])
            processes[role] = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                                stderr=subprocess.DEVNULL)
        wait_ready(state.plan["endpoints"], processes)
        started = now_ms()
        digests = {"go": state.plan["go_sha256"], "rs": state.plan["rust_sha256"]}
        for role in ROLES:
            proc_executable = pathlib.Path(f"/proc/{processes[role].pid}/exe").resolve(strict=True)
            harness.require(hashlib.sha256(proc_executable.read_bytes()).hexdigest() == digests[role],
                            f"{generation}/{role} running executable hash differs")
        sampler = Sampler(generation, processes, digests)
        sampler.start()
        role_listeners = {role: owned_listeners(processes[role].pid) for role in ROLES}
        expected_by_role = {
            role: {harness.loopback_endpoint(value, name) for name, value in state.plan["endpoints"].items()
                   if name.startswith(f"{role}_")}
            for role in ROLES
        }
        harness.require(role_listeners == expected_by_role,
                        f"{generation} listener inventory includes an extra, missing, or non-loopback socket")
        listeners = []
        for name in sorted(state.plan["endpoints"]):
            role = name.split("_", 1)[0]
            host, port = harness.loopback_endpoint(state.plan["endpoints"][name], name)
            with socket.create_connection((host, port), timeout=2):
                pass
            harness.require((host, port) in role_listeners[role],
                            f"{generation}/{name} is not owned by the recorded process")
            listeners.append({"name": name, "address": host, "port": port,
                              "pid_start_ticks": pid_ticks(processes[role].pid),
                              "executable_sha256": digests[role]})
        harness.assert_expected_listeners(state.plan["endpoints"], listeners)
        state.topology_generations[generation] = {"endpoints": dict(state.plan["endpoints"]), "listeners": listeners}
        state.process_rows.extend({"generation": generation, "role": role,
                                   "pid_start_ticks": pid_ticks(processes[role].pid),
                                   "executable_sha256": digests[role]} for role in ROLES)
        return Generation(generation, processes, sampler, started)
    except Exception:
        stop_processes(processes)
        raise


def stop_generation(state: State, runtime: Generation) -> None:
    time.sleep(0.01)
    state.sample_rows.extend(runtime.sampler.stop())
    completed = now_ms()
    stop_processes(runtime.processes)
    state.intervals[runtime.name] = {"started_monotonic_ms": runtime.started_ms, "completed_monotonic_ms": completed}


def invoke_driver(state: State, name: str, transport: str, mode: str, request: dict[str, Any]) -> dict[str, Any]:
    timeout_ms = 100 if name == "core-grpc-cancel" else 5000 if name == "server-timeout" else 30000
    spec = {"name": name, "transport": transport, "mode": mode, "request": request,
            "go_endpoint": endpoint_for(state.plan["endpoints"], "go", transport),
            "rs_endpoint": endpoint_for(state.plan["endpoints"], "rs", transport),
            "timeout_ms": timeout_ms, "disconnect_after_write": mode == "commit-disconnect"}
    result = state.checked["driver_module"].paired(spec, state.checked["client"])
    harness.require_fields(result, {"name", "barrier_release_monotonic_ns", "pair_completed_monotonic_ns",
                                    "timeout_ms", "disconnect_after_write", "go_invoked_monotonic_ns",
                                    "rs_invoked_monotonic_ns", "request_sha256", "transport_client_sha256",
                                    "driver_sha256", "go_result_sha256", "rs_result_sha256", "go", "rs"},
                           f"driver result {name}")
    return result


def paired_operation(state: State, request: bytes, transport: str, mode: str, name: str,
                     timeout_ms: int = 30000) -> dict[str, Any]:
    ready, release = threading.Barrier(3), threading.Event()
    common = [str(state.checked["client"]), "--mode", "exact-broadcast", "--case-name", name,
              "--transport", transport, "--broadcast-mode", mode, "--request-base64",
              base64.b64encode(request).decode(), "--jsonrpc-id", "187", "--timeout-ms", str(timeout_ms)]

    def invoke(side: str) -> tuple[int, int, dict[str, Any]]:
        command = common + ["--endpoint", endpoint_for(state.plan["endpoints"], side, transport)]
        ready.wait(timeout=10)
        harness.require(release.wait(timeout=10), "lifecycle release timed out")
        invoked = time.monotonic_ns()
        completed = subprocess.run(command, capture_output=True, timeout=max(10, timeout_ms / 1000 + 10), check=False)
        finished = time.monotonic_ns()
        harness.require(completed.returncode in {0, 3}, f"lifecycle client exited {completed.returncode}")
        result = harness.strict_json(completed.stdout)
        harness.require_fields(result, CLIENT_FIELDS, f"lifecycle {name}/{side}")
        return invoked, finished, result

    with concurrent.futures.ThreadPoolExecutor(max_workers=2, thread_name_prefix="lifecycle") as pool:
        futures = {side: pool.submit(invoke, side) for side in ROLES}
        ready.wait(timeout=10)
        barrier = time.monotonic_ns()
        release.set()
        rows = {side: futures[side].result() for side in ROLES}
    digest = hashlib.sha256(request).hexdigest()
    receipts = {side: {"request_sha256": digest, "campaign_sha256": state.campaign_sha,
                       "transport_client_sha256": state.plan["transport_client_sha256"],
                       "barrier_release_monotonic_ns": barrier, "invoked_monotonic_ns": rows[side][0],
                       "completed_monotonic_ns": rows[side][1],
                       "result_sha256": hashlib.sha256(harness.canonical_json(rows[side][2])).hexdigest()}
                for side in ROLES}
    return {"digest": digest, "barrier": barrier, "receipts": receipts,
            "go": rows["go"][2], "rs": rows["rs"][2]}


def response_result(response: dict[str, Any]) -> dict[str, Any] | None:
    def events(raw_events: list[bytes]) -> list[dict[str, Any]]:
        decoded: list[dict[str, Any]] = []
        for raw_event in raw_events:
            event_fields = harness.protobuf_fields(raw_event)
            types = [value for number, wire, value in event_fields if number == 1 and wire == 2]
            attributes = [value for number, wire, value in event_fields if number == 2 and wire == 2]
            harness.require(len(types) == 1 and isinstance(types[0], bytes), "ABCI event type differs")
            decoded_attributes = []
            for raw_attribute in attributes:
                harness.require(isinstance(raw_attribute, bytes), "ABCI event attribute differs")
                fields = harness.protobuf_fields(raw_attribute)
                keys = [value for number, wire, value in fields if number == 1 and wire == 2]
                values = [value for number, wire, value in fields if number == 2 and wire == 2]
                indices = [value for number, wire, value in fields if number == 3 and wire == 0]
                harness.require(len(keys) == len(values) == 1 and len(indices) <= 1,
                                "ABCI event attribute schema differs")
                decoded_attributes.append({"key": bytes(keys[0]).decode("utf-8", "strict"),
                                           "value": bytes(values[0]).decode("utf-8", "strict"),
                                           "index": bool(indices[0]) if indices else False})
            decoded.append({"type": bytes(types[0]).decode("utf-8", "strict"),
                            "attributes": decoded_attributes})
        return decoded

    value: dict[str, Any] | None = None
    if response["transport"] in {"comet-jsonrpc", "comet-uri", "comet-websocket"}:
        candidate = (response.get("json") or {}).get("result")
        value = candidate if isinstance(candidate, dict) else None
    elif response["transport"] == "rest":
        candidate = (response.get("json") or {}).get("tx_response")
        value = candidate if isinstance(candidate, dict) else None
    elif response["transport"] == "core-grpc" and response.get("wire") is not None:
        raw = harness.decode_blob(response["wire"], "core response")
        outer = [item for number, wire, item in harness.protobuf_fields(raw) if number == 1 and wire == 2]
        if outer:
            decoded = harness.protobuf_fields(outer[0])
            fields = {number: item for number, _, item in decoded if number != 7}
            value = {"code": fields.get(1, 0), "data": base64.b64encode(fields.get(2, b"")).decode(),
                     "log": bytes(fields.get(3, b"")).decode(), "gas_wanted": fields.get(5, 0),
                     "gas_used": fields.get(6, 0),
                     "events": events([bytes(item) for number, wire, item in decoded if number == 7 and wire == 2]),
                     "codespace": bytes(fields.get(8, b"")).decode()}
    elif response["transport"] == "cosmos-grpc" and response.get("wire") is not None:
        raw = harness.decode_blob(response["wire"], "Cosmos broadcast response")
        outer = [item for number, wire, item in harness.protobuf_fields(raw) if number == 1 and wire == 2]
        if outer:
            decoded = harness.protobuf_fields(outer[0])
            fields = {number: item for number, _, item in decoded if number != 13}
            value = {"code": fields.get(4, 0), "data": bytes(fields.get(5, b"")).decode("utf-8"),
                     "log": bytes(fields.get(6, b"")).decode("utf-8"), "gas_wanted": fields.get(9, 0),
                     "gas_used": fields.get(10, 0),
                     "events": events([bytes(item) for number, wire, item in decoded if number == 13 and wire == 2]),
                     "codespace": bytes(fields.get(3, b"")).decode("utf-8")}
    if value is None:
        return None
    return {"code": int(value.get("code", 0)), "gas_wanted": str(value.get("gas_wanted", "0")),
            "gas_used": str(value.get("gas_used", "0")), "events": value.get("events") or [],
            "codespace": str(value.get("codespace", "")), "log": str(value.get("log", "")),
            "data": str(value.get("data", ""))}


def wait_inclusion(state: State, digest: str, responses: dict[str, dict[str, Any]], timeout: float = 45,
                   indexed_digest: str | None = None) -> dict[str, Any]:
    index_key = indexed_digest or digest
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        found = {side: indexed_tx(state.plan["endpoints"][f"{side}_rpc"], index_key) for side in ROLES}
        if all(found.values()):
            records = {}
            for side in ROLES:
                result = found[side][0]  # type: ignore[index]
                height = int(result["height"])
                raw = block(state.plan["endpoints"][f"{side}_rpc"], height)[0]
                post = query_state(state.plan, side, height, raw["app_hash"])
                check_tx = response_result(responses[side])
                records[side] = {"included_count": 1, "tx_hash": digest, "indexed_tx_hash": index_key,
                                 "check_tx": check_tx,
                                 "check_tx_source": "broadcast-response" if check_tx is not None else "mempool-admission",
                                 "deliver_tx": {"code": int(result["tx_result"].get("code", 0)),
                                                "gas_wanted": str(result["tx_result"].get("gas_wanted", "0")),
                                                "gas_used": str(result["tx_result"].get("gas_used", "0")),
                                                "events": result["tx_result"].get("events") or [],
                                                "codespace": str(result["tx_result"].get("codespace", "")),
                                                "log": str(result["tx_result"].get("log", "")),
                                                "data": str(result["tx_result"].get("data", ""))},
                                 "post_state": {key: post[key] for key in ("height", "app_hash", "balance", "sequence", "delegation")}}
            harness.require(records["go"] == records["rs"], f"paired indexed result differs: {digest}")
            state.last_state = records["go"]["post_state"]
            return records
        time.sleep(0.1)
    raise harness.HarnessError(f"indexed transaction query timed out: {index_key} (submitted {digest})")


def admission_watch(state: State, digest: str, indexed_digest: str, stopping: threading.Event,
                    observed: dict[str, dict[str, Any] | None]) -> None:
    while not stopping.is_set():
        for side in ROLES:
            if observed[side] is None:
                try:
                    hashes, _ = mempool(state.plan["endpoints"][f"{side}_rpc"])
                    if digest in hashes:
                        height = status(state.plan["endpoints"][f"{side}_rpc"])[0]
                        observed[side] = {"proof": "mempool-present", "completed_ms": now_ms(),
                                          "status_height": height, "indexed_height": None}
                    else:
                        indexed = indexed_tx(state.plan["endpoints"][f"{side}_rpc"], indexed_digest)
                        if indexed is not None:
                            result, _ = indexed
                            height = status(state.plan["endpoints"][f"{side}_rpc"])[0]
                            observed[side] = {"proof": "indexed-inclusion", "completed_ms": now_ms(),
                                              "status_height": height,
                                              "indexed_height": int(result["height"])}
                except (OSError, KeyError, ValueError, harness.HarnessError):
                    pass
        stopping.wait(0.005)


def case_semantics(transport: str, mode: str) -> Any:
    if not transport.startswith("comet-"):
        return None
    method = {"async": "broadcast_tx_async", "sync": "broadcast_tx_sync", "commit": "broadcast_tx_commit",
              "commit-disconnect": "broadcast_tx_commit"}.get(mode)
    return {"jsonrpc_id": None if transport == "comet-uri" else 187, "method": method}


def capture_case(state: State, generation: str, spec: tuple[str, str, str, str, str]) -> tuple[dict[str, Any], dict[str, Any]]:
    name, transport, mode, family, expected = spec
    common = common_block(state.plan["endpoints"], state.previous_pre_height + 1,
                          state.checked["freshness_bound_ms"])
    state.previous_pre_height = common["go"]["height"]
    prestates = {}
    for side in ROLES:
        prestates[side] = query_state(state.plan, side, common[side]["height"], common[side]["app_hash"])
        prestates[side]["block_hash"] = common[side]["block_hash"]
    harness.require(prestates["go"] == prestates["rs"], f"{name} paired pre-state differs")
    request_record = state.plan["inputs"][name]
    digest = request_record["sha256"]
    indexed_digest = digest
    if name == "blob-envelope":
        capture_blob_artifact(state, prestates["go"])
        harness.require(state.blob_artifact is not None, "BlobTx capture did not produce an artifact")
        indexed_digest = state.blob_artifact["inner_tx_sha256"]
        harness.require(all(indexed_tx(state.plan["endpoints"][f"{side}_rpc"], indexed_digest) is None for side in ROLES),
                        "BlobTx inner transaction was already indexed before paired submission")
    observed: dict[str, dict[str, Any] | None] = {side: None for side in ROLES}
    stopping = threading.Event()
    watcher = threading.Thread(target=admission_watch, args=(state, digest, indexed_digest, stopping, observed), daemon=True)
    watcher.start()
    try:
        driver = invoke_driver(state, name, transport, mode, request_record)
        if expected not in {"rejected", "policy-difference"}:
            # A transaction can leave the mempool for the next proposal before
            # the 5 ms poll observes it. Its index entry is then available only
            # after that consensus round commits, so retain the watcher for the
            # exact timeout-derived healthy round-zero freshness bound.
            deadline = time.monotonic() + state.checked["freshness_bound_ms"] / 1000
            while any(observed[side] is None for side in ROLES) and time.monotonic() < deadline:
                time.sleep(0.005)
    finally:
        stopping.set()
        watcher.join(timeout=2)
    release_ms = driver["barrier_release_monotonic_ns"] // 1_000_000
    observed_ms = max(common[side]["observed_monotonic_ms"] for side in ROLES)
    remaining = state.checked["commit_window_ms"] - max(0, release_ms - observed_ms)
    harness.require(remaining >= 1000, f"{name} release missed the one-second admission window")
    admitted = expected not in {"rejected", "policy-difference"}
    admission = {}
    for side in ROLES:
        if admitted:
            harness.require(observed[side] is not None, f"{name}/{side} admission was not observed")
            proof_row = observed[side]  # type: ignore[assignment]
            completed_ms = proof_row["completed_ms"]
            observed_height = proof_row["status_height"]
            proof = proof_row["proof"]
            indexed_height = proof_row["indexed_height"]
            present = proof == "mempool-present"
            if proof == "indexed-inclusion":
                harness.require(indexed_height > common[side]["height"] and
                                observed_height >= indexed_height,
                                f"{name}/{side} indexed admission height differs")
            if name in {"mempool-capacity-recovery", "server-timeout"}:
                harness.require(proof == "mempool-present",
                                f"{name}/{side} requires a pending transaction for its capacity lifecycle")
        else:
            hashes, _ = mempool(state.plan["endpoints"][f"{side}_rpc"])
            harness.require(digest not in hashes, f"{name}/{side} rejection entered mempool")
            harness.require(indexed_tx(state.plan["endpoints"][f"{side}_rpc"], digest) is None,
                            f"{name}/{side} rejection was indexed")
            observed_height = status(state.plan["endpoints"][f"{side}_rpc"])[0]
            completed_ms = now_ms()
            proof, present, indexed_height = "mempool-absent", False, None
        if proof != "indexed-inclusion":
            harness.require(observed_height == common[side]["height"],
                            f"{name}/{side} height advanced during admission")
        admission[side] = {"outcome": "admitted" if admitted else "rejected",
                           "completed_after_barrier_ms": completed_ms - release_ms,
                           "status_height": observed_height, "mempool_present": present,
                           "proof": proof, "indexed_height": indexed_height, "tx_hash": digest}
    input_row = {
        "name": name, "generation": generation, "transport": transport, "mode": mode, "family": family,
        "expected_outcome": expected, "pre_state": prestates,
        "boundary": {
            "go": {"height": common["go"]["height"], "block_hash": common["go"]["block_hash"],
                   "app_hash": common["go"]["app_hash"], "block_time": common["go"]["time"],
                   "received_monotonic_ms": common["go"]["observed_monotonic_ms"]},
            "rs": {"height": common["rs"]["height"], "block_hash": common["rs"]["block_hash"],
                   "app_hash": common["rs"]["app_hash"], "block_time": common["rs"]["time"],
                   "received_monotonic_ms": common["rs"]["observed_monotonic_ms"]},
            "barrier_release_monotonic_ms": release_ms, "commit_window_remaining_ms": remaining,
        },
        "requests": {"go": request_record, "rs": request_record},
        "request_semantics": case_semantics(transport, mode),
    }
    difference = ({"policy": "rust-bounded-error-go-abci-socket-close", "go": "abci-connection-closed",
                   "rs": "invalid-argument-malformed-protobuf"} if name == "raw-malformed-protobuf" else None)
    raw_driver = harness.canonical_json(driver)
    response_row = {"name": name, "admission": admission, "go": driver["go"], "rs": driver["rs"],
                    "excluded_headers": list(harness.HTTP_EXCLUDED_HEADERS), "intentional_difference": difference,
                    "raw_driver": {"base64": base64.b64encode(raw_driver).decode(),
                                   "sha256": hashlib.sha256(raw_driver).hexdigest(), "size": len(raw_driver)},
                    "driver": {key: driver[key] for key in ("barrier_release_monotonic_ns", "pair_completed_monotonic_ns",
                                                            "go_invoked_monotonic_ns", "rs_invoked_monotonic_ns", "timeout_ms",
                                                            "disconnect_after_write", "request_sha256", "driver_sha256",
                                                            "transport_client_sha256", "go_result_sha256", "rs_result_sha256")}}
    state.cases.append(input_row)
    state.responses.append(response_row)
    return input_row, response_row


def event(state: State, generation: str, phase: str, outcome: str, digest: str | None,
          response: dict[str, Any] | None, receipts: dict[str, Any] | None,
          previous_ms: int, expected_pool: int | None = None, minimum_height: int | None = None) -> dict[str, Any]:
    stamp = now_ms()
    while stamp <= previous_ms:
        time.sleep(0.001)
        stamp = now_ms()
    heights = {side: status(state.plan["endpoints"][f"{side}_rpc"])[0] for side in ROLES}
    pools = {side: len(mempool(state.plan["endpoints"][f"{side}_rpc"])[0]) for side in ROLES}
    harness.require(heights["go"] == heights["rs"], f"{phase} status heights differ")
    harness.require(pools["go"] == pools["rs"], f"{phase} mempool counts differ")
    if expected_pool is not None:
        harness.require(pools["go"] == expected_pool, f"{phase} mempool transition differs")
    if minimum_height is not None:
        harness.require(heights["go"] >= minimum_height, f"{phase} precedes its indexed inclusion")
    return {side: {"phase": phase, "generation": generation, "monotonic_ms": stamp, "outcome": outcome,
                   "tx_hash": digest, "mempool_size": pools[side], "status_height": heights[side],
                   "response": None if response is None else response[side],
                   "operation": None if receipts is None else receipts[side]} for side in ROLES}


def witnessed_event(generation: str, phase: str, outcome: str, digest: str | None, stamp: int,
                    height: int, pool: int, response: dict[str, Any] | None = None) -> dict[str, Any]:
    """Record chronology already witnessed by the paired driver/admission poller."""
    return {side: {"phase": phase, "generation": generation, "monotonic_ms": stamp, "outcome": outcome,
                   "tx_hash": digest, "mempool_size": pool, "status_height": height,
                   "response": None if response is None else response[side], "operation": None}
            for side in ROLES}


def cleanup_document(events: list[dict[str, Any]], inclusions: list[dict[str, Any]],
                     blocks: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    block_rows = blocks or []
    intervals = []
    for left, right in zip(block_rows, block_rows[1:]):
        ltime = dt.datetime.fromisoformat(left["go"]["time"].replace("Z", "+00:00"))
        rtime = dt.datetime.fromisoformat(right["go"]["time"].replace("Z", "+00:00"))
        intervals.append(int((rtime - ltime).total_seconds() * 1000))
    return {"go": [row["go"] for row in events], "rs": [row["rs"] for row in events],
            "indexed_inclusions": inclusions, "blocks": block_rows, "block_intervals_ms": intervals}


def record_main_inclusion(state: State, source: dict[str, Any], response: dict[str, Any]) -> dict[str, Any]:
    name, expected = source["name"], source["expected_outcome"]
    digest = source["requests"]["go"]["sha256"]
    if expected in {"included", "execution-failed"}:
        indexed_digest = (state.blob_artifact["inner_tx_sha256"]
                          if name == "blob-envelope" and state.blob_artifact is not None else digest)
        pair = wait_inclusion(state, digest, {side: response[side] for side in ROLES},
                              indexed_digest=indexed_digest)
        admission_only = {"comet-commit-disconnect", "core-grpc-cancel", "server-timeout"}
        if name in admission_only and any(
            response["admission"][side]["proof"] == "indexed-inclusion" for side in ROLES
        ):
            for side in ROLES:
                pair[side]["check_tx_source"] = "indexed-inclusion"
    else:
        rejected = {"included_count": 0, "tx_hash": digest, "indexed_tx_hash": None,
                    "check_tx": None, "check_tx_source": None,
                    "deliver_tx": None, "post_state": None}
        pair = {"go": rejected, "rs": dict(rejected)}
        if state.last_state is None:
            pre = source["pre_state"]["go"]
            state.last_state = {key: pre[key] for key in ("height", "app_hash", "balance", "sequence", "delegation")}
    row = {"name": name, **pair}
    state.inclusions.append(row)
    return row


def capacity_lifecycle(state: State, source: dict[str, Any], response: dict[str, Any]) -> None:
    a = harness.decode_blob(state.plan["capacity"]["tx_a"], "capacity A")
    b = harness.decode_blob(state.plan["capacity"]["tx_b"], "capacity B")
    events, indexed, previous = [], [], -1
    events.append(event(state, "normal", "a-admitted", "admitted", hashlib.sha256(a).hexdigest(),
                        {side: response[side] for side in ROLES}, None, previous, expected_pool=1))
    previous = events[-1]["go"]["monotonic_ms"]
    rejected = paired_operation(state, b, "comet-jsonrpc", "sync", "lifecycle-capacity-b")
    rejected_event = event(state, "normal", "b-rejected", "mempool-full", rejected["digest"], rejected,
                           rejected["receipts"], previous)
    # The concrete CheckTx response proves the capacity rejection. The next
    # block may commit A between that response and this follow-up mempool
    # sample, so both the pre-commit and post-commit observations are valid.
    harness.require(rejected_event["go"]["mempool_size"] in {0, 1},
                    "b-rejected mempool transition differs")
    events.append(rejected_event)
    previous = events[-1]["go"]["monotonic_ms"]
    main = record_main_inclusion(state, source, response)
    events.append(event(state, "normal", "a-included", "included", hashlib.sha256(a).hexdigest(), None, None,
                        previous, expected_pool=0, minimum_height=main["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    retry = paired_operation(state, b, "comet-jsonrpc", "sync", "lifecycle-capacity-b-retry")
    events.append(event(state, "normal", "b-retry-admitted", "admitted", retry["digest"], retry,
                        retry["receipts"], previous, expected_pool=1))
    previous = events[-1]["go"]["monotonic_ms"]
    included_b = wait_inclusion(state, retry["digest"], {side: retry[side] for side in ROLES})
    indexed.append({"go": included_b["go"], "rs": included_b["rs"]})
    events.append(event(state, "normal", "b-included", "included", retry["digest"], None, None, previous,
                        expected_pool=0, minimum_height=included_b["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    follow_bytes = harness.decode_blob(state.plan["lifecycle_inputs"]["mempool_followup"], "capacity followup")
    follow = paired_operation(state, follow_bytes, "comet-jsonrpc", "sync", "lifecycle-capacity-followup")
    included_follow = wait_inclusion(state, follow["digest"], {side: follow[side] for side in ROLES})
    indexed.append({"go": included_follow["go"], "rs": included_follow["rs"]})
    events.append(event(state, "normal", "followup-succeeded", "included", follow["digest"], follow,
                        follow["receipts"], previous, expected_pool=0,
                        minimum_height=included_follow["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    events.append(event(state, "normal", "mempool-empty", "empty", None, None, None, previous, expected_pool=0))
    state.cleanup["mempool_capacity"] = cleanup_document(events, indexed)


def followup_lifecycle(state: State, kind: str, source: dict[str, Any], response: dict[str, Any]) -> None:
    definitions = {
        "disconnect": ("disconnect_followup", "comet-jsonrpc", "request-written", "written",
                       "client-disconnected", "disconnected"),
        "grpc_cancel": ("grpc_cancel_followup", "core-grpc", "request-started", "started",
                        "client-canceled", "canceled"),
    }
    input_name, transport, phase_one, outcome_one, phase_two, outcome_two = definitions[kind]
    digest = source["requests"]["go"]["sha256"]
    driver = response["driver"]
    pre_height = source["pre_state"]["go"]["height"]
    first_ms = max(driver["go_invoked_monotonic_ns"], driver["rs_invoked_monotonic_ns"]) // 1_000_000
    completed_ms = (driver["pair_completed_monotonic_ns"] + 999_999) // 1_000_000
    main = record_main_inclusion(state, source, response)
    second = {}
    for side in ROLES:
        admission = response["admission"][side]
        side_ms = max(first_ms + 1, completed_ms,
                      source["boundary"]["barrier_release_monotonic_ms"] + admission["completed_after_barrier_ms"])
        second[side] = {"phase": phase_two, "generation": "normal", "monotonic_ms": side_ms,
                        "outcome": outcome_two, "tx_hash": digest,
                        "mempool_size": 0 if admission["proof"] == "indexed-inclusion" else 1,
                        "status_height": admission["status_height"], "response": None, "operation": None}
    events = [witnessed_event("normal", phase_one, outcome_one, digest, first_ms, pre_height, 0), second]
    previous = max(second[side]["monotonic_ms"] for side in ROLES)
    events.append(event(state, "normal", "tx-included", "included", digest, None, None, previous,
                        expected_pool=0, minimum_height=main["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    retry_bytes = harness.decode_blob(source["requests"]["go"], f"{kind} retry")
    retry = paired_operation(state, retry_bytes, transport, "commit", f"lifecycle-{kind}-retry")
    events.append(event(state, "normal", "retry-observed", "duplicate", digest, retry, retry["receipts"],
                        previous, expected_pool=0))
    previous = events[-1]["go"]["monotonic_ms"]
    follow_bytes = harness.decode_blob(state.plan["lifecycle_inputs"][input_name], f"{kind} followup")
    follow = paired_operation(state, follow_bytes, transport, "commit", f"lifecycle-{kind}-followup")
    inclusion = wait_inclusion(state, follow["digest"], {side: follow[side] for side in ROLES})
    events.append(event(state, "normal", "followup-succeeded", "included", follow["digest"], follow,
                        follow["receipts"], previous, expected_pool=0,
                        minimum_height=inclusion["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    events.append(event(state, "normal", "mempool-empty", "empty", None, None, None, previous, expected_pool=0))
    state.cleanup[kind] = cleanup_document(events, [{"go": inclusion["go"], "rs": inclusion["rs"]}])


def timeout_lifecycle_prefix(state: State, source: dict[str, Any], response: dict[str, Any],
                             timeout_blocks: list[dict[str, Any]]) -> dict[str, Any]:
    digest = source["requests"]["go"]["sha256"]
    driver = response["driver"]
    pre_height = source["pre_state"]["go"]["height"]
    first_ms = max(driver["go_invoked_monotonic_ns"], driver["rs_invoked_monotonic_ns"]) // 1_000_000
    completed_ms = (driver["pair_completed_monotonic_ns"] + 999_999) // 1_000_000
    admission_ms = source["boundary"]["barrier_release_monotonic_ms"] + max(
        response["admission"][side]["completed_after_barrier_ms"] for side in ROLES)
    second_ms = max(first_ms + 1, completed_ms, admission_ms)
    events = [witnessed_event("timeout", "request-started", "started", digest, first_ms, pre_height, 0),
              witnessed_event("timeout", "server-timeout", "timeout", digest, second_ms, pre_height, 1,
                              {side: response[side] for side in ROLES})]
    previous = second_ms
    probe_bytes = harness.decode_blob(state.plan["lifecycle_inputs"]["server_timeout_probe"], "timeout capacity probe")
    probe = paired_operation(state, probe_bytes, "core-grpc", "commit", "lifecycle-timeout-probe", 5000)
    events.append(event(state, "timeout", "capacity-probe-rejected", "capacity-rejected", probe["digest"], probe,
                        probe["receipts"], previous, expected_pool=1))
    previous = events[-1]["go"]["monotonic_ms"]
    main = record_main_inclusion(state, source, response)
    events.append(event(state, "timeout", "tx-included", "included", digest, None, None, previous,
                        expected_pool=0, minimum_height=main["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    retry_bytes = harness.decode_blob(source["requests"]["go"], "timeout retry")
    retry = paired_operation(state, retry_bytes, "core-grpc", "commit", "lifecycle-timeout-retry")
    events.append(event(state, "timeout", "retry-observed", "duplicate", digest, retry, retry["receipts"],
                        previous, expected_pool=0))
    while len(timeout_blocks) < 4:
        next_height = timeout_blocks[-1]["go"]["height"] + 1
        row = common_block_at(state.plan["endpoints"], next_height, 15)
        timeout_blocks.append({"generation": "timeout", **{
            side: {key: row[side][key] for key in ("height", "block_hash", "app_hash", "time", "observed_monotonic_ms")}
            for side in ROLES
        }})
    return {"events": events, "previous": events[-1]["go"]["monotonic_ms"], "blocks": timeout_blocks}


def timeout_lifecycle_suffix(state: State, pending: dict[str, Any]) -> None:
    events, previous = pending["events"], pending["previous"]
    events.append(event(state, "restored", "normal-config-restored", "restored", None, None, None,
                        previous, expected_pool=0))
    previous = events[-1]["go"]["monotonic_ms"]
    follow_bytes = harness.decode_blob(state.plan["lifecycle_inputs"]["server_timeout_followup"], "timeout followup")
    follow = paired_operation(state, follow_bytes, "core-grpc", "commit", "lifecycle-timeout-followup")
    inclusion = wait_inclusion(state, follow["digest"], {side: follow[side] for side in ROLES})
    events.append(event(state, "restored", "followup-succeeded", "included", follow["digest"], follow,
                        follow["receipts"], previous, expected_pool=0,
                        minimum_height=inclusion["go"]["post_state"]["height"]))
    previous = events[-1]["go"]["monotonic_ms"]
    events.append(event(state, "restored", "mempool-empty", "empty", None, None, None, previous, expected_pool=0))
    state.cleanup["server_timeout"] = cleanup_document(events, [{"go": inclusion["go"], "rs": inclusion["rs"]}],
                                                        pending["blocks"])


def capture_blob_artifact(state: State, prestate: dict[str, Any]) -> None:
    """Run the reviewed one-shot proxy and exact Go CLI before paired submission."""
    frozen = harness.strict_json(state.checked["seeds"]["blob-proxy.json"].read_bytes())
    fields = {"request_id", "method", "request_count", "forward_count", "source_hash", "raw_request", "destination",
              "cli_argv", "cli_source_sha256", "envelope", "submitted_go_sha256", "submitted_rs_sha256",
              "inner_tx_sha256", "unwrapped_inner_sha256", "namespace", "data_sha256", "share_version",
              "blob_signer_hex", "cli_signer", "absent_before_go", "absent_before_rs", "read_methods"}
    harness.require_fields(frozen, fields, "frozen BlobTx CLI specification")
    envelope = harness.decode_blob(state.plan["inputs"]["blob-envelope"], "BlobTx case request")
    decoded = harness.decode_canonical_blob_tx(envelope)
    harness.require(len(decoded["blobs"]) == 1, "BlobTx must contain exactly one blob")
    item = decoded["blobs"][0]
    harness.require(
        item["namespace_version"] == 0
        and item["namespace_id"][:18] == b"\0" * 18
        and item["share_version"] == 0
        and item["signer"] == b"",
        "BlobTx CLI provenance requires a v0 namespace, share v0 and empty blob signer",
    )
    cli_argv = [
        # The BlobTx carries the canonical 28-byte namespace ID. The v10.5
        # CLI accepts only the 10-byte user-controlled suffix for a v0
        # namespace and reconstructs the canonical prefix itself.
        "tx", "blob", "pay-for-blob", "0x" + item["namespace_id"][-10:].hex(), "0x" + item["data"].hex(),
        "--from", frozen["cli_signer"], "--keyring-backend", "test",
        "--account-number", str(prestate["account_number"]), "--sequence", str(prestate["sequence"]),
        "--chain-id", "rs-devnet-187", "--offline", "--gas", "200000", "--fees", "100utia",
        "--node", frozen["destination"], "--broadcast-mode", "sync", "--yes", "--output", "json",
    ]
    harness.require(frozen["cli_argv"] == cli_argv, "frozen BlobTx CLI argv differs from live pre-state")
    parsed = urllib.parse.urlsplit(frozen["destination"])
    harness.require(parsed.scheme == "http" and parsed.path in {"", "/"} and not parsed.query and not parsed.fragment,
                    "BlobTx proxy destination differs")
    host, port = harness.loopback_endpoint(frozen["destination"], "BlobTx proxy destination")
    inner_digest = hashlib.sha256(decoded["tx"]).hexdigest()
    absent = {side: indexed_tx(state.plan["endpoints"][f"{side}_rpc"], inner_digest) is None
              for side in ROLES}
    harness.require(all(absent.values()), "BlobTx inner transaction exists before CLI capture")

    raw_path = state.checked["output"] / ".blob-proxy.raw.json"
    failures: list[BaseException] = []

    def serve() -> None:
        try:
            state.checked["blob_proxy_module"].serve(f"{host}:{port}", raw_path, 30)
        except BaseException as error:  # retained and re-raised on the controller thread
            failures.append(error)

    proxy = threading.Thread(target=serve, name="blob-proxy")
    proxy.start()
    deadline = time.monotonic() + 5
    while (host, port) not in owned_listeners(os.getpid()) and time.monotonic() < deadline and not failures:
        time.sleep(0.01)
    harness.require(not failures and (host, port) in owned_listeners(os.getpid()),
                    "BlobTx proxy did not bind its listener")
    cli_env = dict(os.environ)
    cli_env["HOME"] = str(state.checked["homes"]["go"].parent)
    completed = subprocess.run([str(state.checked["binaries"]["go"]), *cli_argv], capture_output=True,
                               timeout=40, check=False, env=cli_env)
    proxy.join(timeout=35)
    harness.require(not proxy.is_alive() and not failures, f"BlobTx proxy failed: {failures[:1]}")
    harness.require(completed.returncode == 0, f"BlobTx CLI failed: {completed.stderr[:4096]!r}")
    captured = harness.strict_json(raw_path.read_bytes())
    raw_path.unlink()
    harness.require_fields(captured, {"raw_request", "envelope", "request_id", "method", "request_count", "forward_count"},
                           "live BlobTx proxy capture")
    harness.require(harness.decode_blob(captured["envelope"], "captured BlobTx envelope") == envelope,
                    "live CLI BlobTx envelope differs from frozen paired input")
    inner = decoded["tx"]
    namespace = bytes([item["namespace_version"]]) + item["namespace_id"]
    digest = hashlib.sha256(envelope).hexdigest()
    state.blob_artifact = {
        **captured, "source_hash": state.checked["receipts"]["capture_sources"]["blob_proxy.py"],
        "destination": frozen["destination"], "cli_argv": cli_argv,
        "cli_source_sha256": state.checked["receipts"]["go"]["cli_source_sha256"],
        "submitted_go_sha256": digest, "submitted_rs_sha256": digest,
        "inner_tx_sha256": hashlib.sha256(inner).hexdigest(),
        "unwrapped_inner_sha256": hashlib.sha256(inner).hexdigest(), "namespace": namespace.hex(),
        "data_sha256": hashlib.sha256(item["data"]).hexdigest(), "share_version": item["share_version"],
        "blob_signer_hex": item["signer"].hex(), "cli_signer": frozen["cli_signer"],
        "absent_before_go": absent["go"], "absent_before_rs": absent["rs"], "read_methods": [],
    }


def validated_blob_artifact(state: State) -> dict[str, Any]:
    harness.require(state.blob_artifact is not None, "live BlobTx proxy capture is missing")
    value = state.blob_artifact
    fields = {"request_id", "method", "request_count", "forward_count", "source_hash", "raw_request", "destination",
              "cli_argv", "cli_source_sha256", "envelope", "submitted_go_sha256", "submitted_rs_sha256",
              "inner_tx_sha256", "unwrapped_inner_sha256", "namespace", "data_sha256", "share_version",
              "blob_signer_hex", "cli_signer", "absent_before_go", "absent_before_rs", "read_methods"}
    harness.require_fields(value, fields, "BlobTx proxy artifact")
    harness.require(value["request_id"] == 0 and value["method"] == "broadcast_tx_sync" and
                    value["request_count"] == 1 and value["forward_count"] == 0 and
                    value["absent_before_go"] is True and value["absent_before_rs"] is True and
                    value["read_methods"] == [], "BlobTx proxy provenance differs")
    raw = harness.decode_blob(value["raw_request"], "BlobTx raw proxy request")
    request = harness.strict_json(raw)
    harness.require_fields(request, {"jsonrpc", "id", "method", "params"}, "BlobTx raw JSON-RPC request")
    harness.require(request["jsonrpc"] == "2.0" and request["id"] == 0 and
                    request["method"] == "broadcast_tx_sync", "BlobTx raw request identity differs")
    params = harness.require_fields(request["params"], {"tx"}, "BlobTx raw params")
    envelope = base64.b64decode(params["tx"], validate=True)
    harness.require(envelope == harness.decode_blob(value["envelope"], "BlobTx envelope"), "proxy request envelope differs")
    decoded = harness.decode_canonical_blob_tx(envelope)
    harness.require(len(decoded["blobs"]) == 1, "BlobTx must contain exactly one blob")
    item, inner = decoded["blobs"][0], decoded["tx"]
    namespace = bytes([item["namespace_version"]]) + item["namespace_id"]
    digest = hashlib.sha256(envelope).hexdigest()
    harness.require(digest == value["submitted_go_sha256"] == value["submitted_rs_sha256"] and
                    hashlib.sha256(inner).hexdigest() == value["inner_tx_sha256"] == value["unwrapped_inner_sha256"] and
                    namespace.hex() == value["namespace"] and hashlib.sha256(item["data"]).hexdigest() == value["data_sha256"] and
                    item["share_version"] == value["share_version"] and item["signer"].hex() == value["blob_signer_hex"],
                    "BlobTx unwrap binding differs")
    harness.require(envelope == harness.decode_blob(state.plan["inputs"]["blob-envelope"], "BlobTx case request"),
                    "BlobTx proxy envelope differs from paired request")
    harness.require(value["source_hash"] == hashlib.sha256(pathlib.Path(__file__).with_name("blob_proxy.py").read_bytes()).hexdigest() and
                    value["cli_source_sha256"] == state.checked["receipts"]["go"]["cli_source_sha256"],
                    "BlobTx proxy source hash differs")
    return value


def copy_seed(state: State, name: str) -> None:
    write_new(state.checked["output"] / name, harness.strict_json(state.checked["seeds"][name].read_bytes()))


def write_payloads(state: State, resources_only: bool = False) -> None:
    output = state.checked["output"]
    if resources_only:
        write_new(output / "resources.json", {"processes": state.process_rows,
                                               "samples": sorted(state.sample_rows, key=lambda row: row["monotonic_ms"]),
                                               "generation_intervals": state.intervals})
        return
    for name in ("mainnet-observation.json", "build-receipts.json", "genesis.json", "configs.json"):
        copy_seed(state, name)
    write_new(output / "blob-proxy.json", validated_blob_artifact(state))
    maximum = state.checked["configs"]["normal"]["go"]["config.toml"]["mempool.max_txs_bytes"]
    max_txs = state.checked["configs"]["normal"]["go"]["config.toml"]["mempool.size"]
    write_new(output / "topology.json", {"generations": state.topology_generations, "pex": False,
                                          "mempool_broadcast": False, "validator_power": {"go": "1", "rs": "1"},
                                          "mempool_size": max_txs, "max_txs": 1,
                                          "max_txs_bytes": maximum,
                                          "max_subscription_clients": harness.SUBSCRIPTION_CLIENT_LIMITS,
                                          "max_subscriptions_per_client": 1})
    write_new(output / "inputs.json", {"cases": state.cases, "capacity": state.plan["capacity"],
                                        "lifecycle_inputs": state.plan["lifecycle_inputs"]})
    write_new(output / "paired-responses.json", {"cases": state.responses,
                                                  "driver_sha256": state.plan["driver_sha256"],
                                                  "transport_client_sha256": state.plan["transport_client_sha256"]})
    write_new(output / "inclusions.json", {"cases": state.inclusions})
    harness.require(state.last_state is not None, "campaign has no final included state")
    live_block = common_block(state.plan["endpoints"], state.previous_pre_height + 1,
                              state.checked["freshness_bound_ms"])
    live_states = {side: query_state(state.plan, side, live_block[side]["height"], live_block[side]["app_hash"])
                   for side in ROLES}
    harness.require(live_states["go"] == live_states["rs"], "final paired state differs")
    pools = {side: len(mempool(state.plan["endpoints"][f"{side}_rpc"])[0]) for side in ROLES}
    final = state.last_state
    fresh = live_states["go"]
    harness.require((fresh["app_hash"], fresh["balance"], fresh["sequence"], fresh["delegation"]) ==
                    (final["app_hash"], final["balance"], final["sequence"], final["delegation"]),
                    "final fresh state differs from last indexed state")
    liveness = {"generation": "restored", "block": {
        side: {key: live_block[side][key] for key in ("height", "block_hash", "app_hash", "time", "observed_monotonic_ms")}
        for side in ROLES}, "state": live_states, "mempool": pools}
    state.cleanup["final"] = {"go_root": fresh["app_hash"], "rs_root": fresh["app_hash"],
                              "go_balance": fresh["balance"], "rs_balance": fresh["balance"],
                              "go_sequence": fresh["sequence"], "rs_sequence": fresh["sequence"],
                              "go_delegation": fresh["delegation"], "rs_delegation": fresh["delegation"],
                              "go_mempool": pools["go"], "rs_mempool": pools["rs"], "liveness": liveness}
    write_new(output / "cleanup.json", state.cleanup)


def run(plan: dict[str, Any]) -> None:
    checked = validate_plan(plan)
    state = State(plan, checked, hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest())
    runtime: Generation | None = None
    try:
        runtime = start_generation(state, "normal")
        for spec in harness.CASE_SPECS:
            if spec[0] in {"server-timeout", "raw-malformed-protobuf"}:
                continue
            source, response = capture_case(state, "normal", spec)
            if spec[0] == "mempool-capacity-recovery":
                capacity_lifecycle(state, source, response)
            elif spec[0] == "comet-commit-disconnect":
                followup_lifecycle(state, "disconnect", source, response)
            elif spec[0] == "core-grpc-cancel":
                followup_lifecycle(state, "grpc_cancel", source, response)
            else:
                record_main_inclusion(state, source, response)
        stop_generation(state, runtime)
        runtime = None

        runtime = start_generation(state, "timeout")
        first = common_block(plan["endpoints"], state.previous_pre_height + 1,
                             state.checked["freshness_bound_ms"])
        timeout_blocks = [{"generation": "timeout", **{
            side: {key: first[side][key] for key in ("height", "block_hash", "app_hash", "time", "observed_monotonic_ms")}
            for side in ROLES
        }}]
        timeout_spec = next(spec for spec in harness.CASE_SPECS if spec[0] == "server-timeout")
        source, response = capture_case(state, "timeout", timeout_spec)
        pending = timeout_lifecycle_prefix(state, source, response, timeout_blocks)
        stop_generation(state, runtime)
        runtime = None

        runtime = start_generation(state, "restored")
        timeout_lifecycle_suffix(state, pending)
        malformed_spec = next(spec for spec in harness.CASE_SPECS if spec[0] == "raw-malformed-protobuf")
        source, response = capture_case(state, "restored", malformed_spec)
        record_main_inclusion(state, source, response)
        write_payloads(state)
        stop_generation(state, runtime)
        runtime = None
        write_payloads(state, resources_only=True)
    finally:
        if runtime is not None:
            try:
                stop_generation(state, runtime)
            except Exception:
                pass


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", type=pathlib.Path, required=True)
    parser.add_argument("--source-sha256", required=True)
    args = parser.parse_args()
    try:
        source = pathlib.Path(__file__).read_bytes()
        harness.require(hashlib.sha256(source).hexdigest() == args.source_sha256, "campaign source hash differs")
        run(load_plan(args.plan))
    except (OSError, subprocess.SubprocessError, threading.BrokenBarrierError, ValueError, harness.HarnessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
