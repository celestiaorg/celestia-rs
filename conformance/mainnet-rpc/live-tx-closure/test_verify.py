#!/usr/bin/env python3
"""Semantic mutation tests for the paired live-transaction verifier."""

from __future__ import annotations

import base64
import copy
import datetime as dt
import hashlib
import io
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock
import urllib.error

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import harness
import capture
import campaign
import verify


HERE = pathlib.Path(__file__).parent
CANDIDATE = "1" * 40
NOW = dt.datetime(2026, 10, 31, 4, 0, tzinfo=dt.timezone.utc)


def blob(data: bytes) -> dict[str, object]:
    return {"base64": base64.b64encode(data).decode(), "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def dump(path: pathlib.Path, value: object) -> None:
    path.write_bytes(harness.canonical_json(value))


def observation() -> dict[str, object]:
    status = json.dumps({"result": {"node_info": {"network": "celestia"}, "sync_info": {"latest_block_height": "100"}}}, separators=(",", ":"))
    timeout_info = {name: str(value) for name, value in harness.ACTIVE_TIMEOUT_INFO_NS.items()}
    abci = json.dumps({"result": {"response": {"data": "celestia-app", "version": "10.5.0",
                                                "app_version": "10", "last_block_height": "100",
                                                "last_block_app_hash": base64.b64encode(b"a" * 32).decode(),
                                                "timeout_info": timeout_info}}},
                      separators=(",", ":"))
    return {
        "endpoint": "https://rpc.celestia.example",
        "observed_at": (NOW - dt.timedelta(minutes=1)).isoformat(),
        "chain_id": "celestia",
        "version": "10.5.0",
        "app_protocol": 10,
        "status_height": 100,
        "abci_height": 100,
        "responses": {
            "status": {"body": status, "sha256": hashlib.sha256(status.encode()).hexdigest()},
            "abci_info": {"body": abci, "sha256": hashlib.sha256(abci.encode()).hexdigest()},
        },
    }


def fixture(root: pathlib.Path) -> None:
    source_hashes = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in ("harness.py", "verify.py", "capture.py", "driver.py", "blob_proxy.py", "campaign.py")}
    build_info = "celestia-appd: go1.26.9\npath\tgithub.com/celestiaorg/celestia-app/v10/cmd/celestia-appd\nmod\tgithub.com/celestiaorg/celestia-app/v10\t(devel)\n=>\tgithub.com/celestiaorg/celestia-core\tv0.42.3\n"
    go_source = "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    go_ldflags = (
        "-X github.com/cosmos/cosmos-sdk/version.Name=celestia-app "
        "-X github.com/cosmos/cosmos-sdk/version.AppName=celestia-appd "
        "-X github.com/cosmos/cosmos-sdk/version.Version=10.5.0 "
        f"-X github.com/cosmos/cosmos-sdk/version.Commit={go_source} "
        "-X github.com/cosmos/cosmos-sdk/version.BuildTags=ledger "
        "-X github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd/cmd.v2UpgradeHeight=0"
    )
    cargo_lock_sha = hashlib.sha256((HERE.parents[2] / "Cargo.lock").read_bytes()).hexdigest()
    client_root = HERE.parents[2] / "conformance/provider-client"
    dump(root / "mainnet-observation.json", observation())
    dump(root / "build-receipts.json", {
        "go": {
            "module": "github.com/celestiaorg/celestia-app/v10@v10.5.0",
            "source_commit": go_source,
            "core_commit": "a5537d84a008d566abc503d2ea41e30f2301403d",
            "compiler": "go1.26.9", "tags": ["ledger"], "binary_sha256": "a" * 64,
            "command": ["go", "build", "-buildvcs=false", "-tags=ledger", f"-ldflags={go_ldflags}", "-o", "celestia-appd", "./cmd/celestia-appd"], "build_info": build_info,
            "build_info_sha256": hashlib.sha256(build_info.encode()).hexdigest(), "cli_source_sha256": "7" * 64,
        },
        "rust": {
            "source_commit": CANDIDATE, "compiler": "rustc 1.98.0", "binary_sha256": "b" * 64,
            "command": ["cargo", "build", "--locked", "--bin", "celestia-rs"], "features": ["default"],
            "cargo_lock_sha256": cargo_lock_sha,
            "version_long": f"celestia-rs commit {CANDIDATE} app 10.5.0",
        },
        "transport_client": {
            "binary_sha256": "6" * 64,
            "source_sha256": hashlib.sha256((client_root / "main.go").read_bytes()).hexdigest(),
            "go_mod_sha256": hashlib.sha256((client_root / "go.mod").read_bytes()).hexdigest(),
            "go_sum_sha256": hashlib.sha256((client_root / "go.sum").read_bytes()).hexdigest(),
            "compiler": "go1.26.9", "command": ["go", "build", "-trimpath", "-o", "provider-client", "."],
        },
        "capture_sources": source_hashes,
    })
    endpoint_values = {
        "go_rpc": "http://127.0.0.1:31001", "go_core": "127.0.0.1:31002",
        "go_sdk": "127.0.0.1:31003", "go_rest": "http://127.0.0.1:31004",
        "go_p2p": "tcp://127.0.0.1:31005",
        "rs_rpc": "http://127.0.0.1:32001", "rs_core": "127.0.0.1:32002",
        "rs_sdk": "127.0.0.1:32003", "rs_rest": "http://127.0.0.1:32004",
        "rs_p2p": "tcp://127.0.0.1:32005",
    }
    topology_generations = {}
    for generation_index, generation in enumerate(("normal", "timeout", "restored")):
        listeners = []
        for index, (name, value) in enumerate(endpoint_values.items()):
            host, port = harness.loopback_endpoint(value, "test")
            ticks = 1 + generation_index * 2 if name.startswith("go_") else 2 + generation_index * 2
            listeners.append({"name": name, "address": host, "port": port, "pid_start_ticks": ticks,
                              "executable_sha256": ("a" if name.startswith("go_") else "b") * 64})
        topology_generations[generation] = {"endpoints": copy.deepcopy(endpoint_values), "listeners": listeners}
    dump(root / "topology.json", {
        "generations": topology_generations, "pex": False, "mempool_broadcast": False,
        "validator_power": {"go": "1", "rs": "1"}, "mempool_size": 0, "max_txs": 1,
        "max_txs_bytes": harness.MEMPOOL_MAX_TXS_BYTES,
        "max_subscription_clients": copy.deepcopy(harness.SUBSCRIPTION_CLIENT_LIMITS),
        "max_subscriptions_per_client": 1,
    })
    normal = {}
    for node in ("go", "rs"):
        normal[node] = {
            "config.toml": {
                "priv_validator_grpc_laddr": "",
                "rpc.timeout_broadcast_tx_commit": "10s", "rpc.laddr": endpoint_values[f"{node}_rpc"],
                "rpc.grpc_laddr": endpoint_values[f"{node}_core"],
                "rpc.max_subscription_clients": harness.SUBSCRIPTION_CLIENT_LIMITS[node],
                "rpc.max_subscriptions_per_client": 1, "p2p.pex": False, "mempool.broadcast": False,
                "p2p.laddr": endpoint_values[f"{node}_p2p"],
                "mempool.size": 0, "mempool.max_txs_bytes": harness.MEMPOOL_MAX_TXS_BYTES,
                "tx_index.indexer": "kv", "consensus.timeout_commit": "5s",
            },
            "app.toml": {"grpc.enable": True, "grpc.address": endpoint_values[f"{node}_sdk"],
                         "grpc-web.enable": False, "api.enable": True,
                         "api.address": endpoint_values[f"{node}_rest"]},
        }
    timeout = copy.deepcopy(normal)
    for node in ("go", "rs"):
        timeout[node]["config.toml"]["rpc.timeout_broadcast_tx_commit"] = "250ms"
    def full_config(rows):
        result = {}
        for node, files in rows.items():
            config, app = files["config.toml"], files["app.toml"]
            result[node] = {
                "config.toml": {
                    "priv_validator_grpc_laddr": config["priv_validator_grpc_laddr"],
                    "rpc": {"timeout_broadcast_tx_commit": config["rpc.timeout_broadcast_tx_commit"],
                            "laddr": config["rpc.laddr"], "grpc_laddr": config["rpc.grpc_laddr"],
                            "max_subscription_clients": config["rpc.max_subscription_clients"],
                            "max_subscriptions_per_client": config["rpc.max_subscriptions_per_client"]},
                    "p2p": {"pex": config["p2p.pex"], "laddr": config["p2p.laddr"]},
                    "mempool": {"broadcast": config["mempool.broadcast"],
                                "size": config["mempool.size"],
                                "max_txs_bytes": config["mempool.max_txs_bytes"]},
                    "tx_index": {"indexer": config["tx_index.indexer"]},
                    "consensus": {"timeout_commit": config["consensus.timeout_commit"]},
                },
                "app.toml": {"grpc": {"enable": app["grpc.enable"], "address": app["grpc.address"]},
                             "grpc-web": {"enable": app["grpc-web.enable"]},
                             "api": {"enable": app["api.enable"], "address": app["api.address"]}},
            }
        return result
    dump(root / "configs.json", {"normal": normal, "timeout": timeout, "restored": copy.deepcopy(normal),
                                  "full": {"normal": full_config(normal), "timeout": full_config(timeout),
                                           "restored": full_config(normal)}})
    dump(root / "genesis.json", {
        "app_name": "celestia-appd", "app_version": "10.5.0",
        "genesis_time": NOW.isoformat(), "chain_id": "rs-devnet-187", "initial_height": 1,
        "app_hash": "", "app_state": {"auth": {"accounts": []}},
        "consensus": {"params": {"block": {"max_bytes": "22020096"}}, "validators": [
            {"address": "A", "pub_key": {"type": "tendermint/PubKeyEd25519", "value": "A"}, "power": "1", "name": "go"},
            {"address": "B", "pub_key": {"type": "tendermint/PubKeyEd25519", "value": "B"}, "power": "1", "name": "rs"},
        ]},
    })
    input_cases = []
    response_cases = []
    inclusion_cases = []
    sequence = 0
    height = 10
    balance = "1000000"
    app_hash = "initial-app"
    delegation = None
    inner_blob_tx = b"signed-inner-pfb"
    namespace_id = b"\0" * 27 + b"\1"
    blob_data = b"celestia-rs-live-tx"
    blob_proto = (
        harness.protobuf_bytes_field(1, namespace_id)
        + harness.protobuf_bytes_field(2, blob_data)
    )
    blob_envelope = (
        harness.protobuf_bytes_field(1, inner_blob_tx)
        + harness.protobuf_bytes_field(2, blob_proto)
        + harness.protobuf_bytes_field(3, b"BLOB")
    )
    no_message_txraw = (
        harness.protobuf_bytes_field(1, harness.protobuf_bytes_field(2, b"no messages"))
        + harness.protobuf_bytes_field(2, harness.protobuf_bytes_field(2, b"auth"))
        + harness.protobuf_bytes_field(3, b"signature")
    )
    invalid_rest_request = harness.canonical_json({"tx_bytes": "***", "mode": "BROADCAST_MODE_SYNC"})
    lifecycle_input_bytes = {
        "mempool_followup": b"mempool-followup",
        "disconnect_followup": b"disconnect-followup",
        "grpc_cancel_followup": b"grpc-cancel-followup",
        "server_timeout_probe": b"server-timeout-probe",
        "server_timeout_followup": b"server-timeout-followup",
    }
    lifecycle_indexed: dict[str, list[dict[str, object]]] = {
        "mempool_capacity": [], "disconnect": [], "grpc_cancel": [], "server_timeout": [],
    }
    for index, spec in enumerate(harness.CASE_SPECS):
        name, transport, mode, family, expected = spec
        request = (invalid_rest_request if name == "rest-invalid-base64" else
                   no_message_txraw if name == "cosmos-grpc-no-message" else
                   blob_envelope if name == "blob-envelope" else
                   b"A" * 300 if name == "mempool-capacity-recovery" else
                   f"request-{index}".encode())
        pre = {"height": height, "block_hash": f"block-{index}", "app_hash": app_hash, "account_number": 1, "sequence": sequence, "balance": balance, "delegation": delegation}
        boundary_time = 18000 if name == "server-timeout" else 27000 if name == "raw-malformed-protobuf" else 10000 + index * 100
        boundary_row = {"height": pre["height"], "block_hash": pre["block_hash"], "app_hash": pre["app_hash"], "block_time": f"2026-10-31T04:{index:02d}:00Z", "received_monotonic_ms": boundary_time}
        input_cases.append({
            "name": name, "generation": "timeout" if name == "server-timeout" else "restored" if name == "raw-malformed-protobuf" else "normal",
            "transport": transport, "mode": mode, "family": family, "expected_outcome": expected,
            "pre_state": {"go": pre, "rs": copy.deepcopy(pre)},
            "boundary": {"go": boundary_row, "rs": dict(boundary_row, received_monotonic_ms=boundary_row["received_monotonic_ms"] + 20), "barrier_release_monotonic_ms": boundary_row["received_monotonic_ms"] + 30, "commit_window_remaining_ms": 2590},
            "requests": {"go": blob(request), "rs": blob(request)},
            "request_semantics": {"jsonrpc_id": None if transport == "comet-uri" else 187, "method": {"async": "broadcast_tx_async", "sync": "broadcast_tx_sync", "commit": "broadcast_tx_commit", "commit-disconnect": "broadcast_tx_commit"}.get(mode)} if transport.startswith("comet-") else None,
        })
        admitted = expected not in {"rejected", "policy-difference"}
        tx_hash = hashlib.sha256(request).hexdigest()
        admission = {side: {"outcome": "admitted" if admitted else "rejected", "completed_after_barrier_ms": 100, "status_height": pre["height"], "mempool_present": admitted, "proof": "mempool-present" if admitted else "mempool-absent", "indexed_height": None, "tx_hash": tx_hash} for side in ("go", "rs")}
        is_grpc = "grpc" in transport
        is_websocket = transport == "comet-websocket"
        go_response = {
            "transport": transport, "wire": blob(name.encode()), "http_status": None if is_grpc or is_websocket else 200,
            "http_headers": [] if is_grpc or is_websocket else [["content-type", "application/json"]],
            "json": ({"id": -1 if transport == "comet-uri" else 187, "result": {"code": 0 if admitted else 1, "log": "" if admitted else "rejected"}} if transport.startswith("comet-") else {"code": 0 if admitted else 1}) if "grpc" not in transport else None,
            "grpc_status": 0 if is_grpc else None,
            "grpc_message": "", "initial_metadata": [["content-type", "application/grpc"]] if is_grpc else [],
            "trailing_metadata": [],
            "websocket_frames": [{"opcode": "text", "payload": blob(name.encode())}] if transport == "comet-websocket" else [],
            "websocket_handshake": {"status": 101, "headers": [["connection", "Upgrade"], ["sec-websocket-accept", harness.FIXED_WEBSOCKET_ACCEPT], ["upgrade", "websocket"]], "request_id": 187} if transport == "comet-websocket" else None,
            "client_error": None, "process_effect": None,
        }
        if not is_grpc:
            response_wire = harness.canonical_json(go_response["json"])
            go_response["wire"] = blob(response_wire)
            if is_websocket:
                go_response["websocket_frames"] = [{"opcode": "text", "payload": blob(response_wire)}]
        rs_response = copy.deepcopy(go_response)
        difference = None
        if name == "comet-commit-disconnect":
            for record in (go_response, rs_response):
                record.update({
                    "wire": None, "http_status": None, "http_headers": [], "json": None,
                    "client_error": "client-disconnected-after-complete-write",
                    "process_effect": "client-disconnected-after-complete-write",
                })
        elif name == "core-grpc-cancel":
            for record in (go_response, rs_response):
                record.update({
                    "wire": blob(b""), "grpc_status": None, "grpc_message": None,
                    "initial_metadata": [], "client_error": "context deadline exceeded",
                    "process_effect": "client-canceled",
                })
        elif name == "server-timeout":
            go_response["process_effect"] = "server-timeout"
            rs_response["process_effect"] = "server-timeout"
        if expected == "policy-difference":
            go_response["process_effect"] = "abci-connection-closed"
            go_response["client_error"] = "grpc-transport-closed"
            go_response["grpc_status"] = None
            rs_response["process_effect"] = "continued"
            rs_response["grpc_status"] = 3
            rs_response["grpc_message"] = "invalid-argument-malformed-protobuf"
            difference = {"policy": "rust-bounded-error-go-abci-socket-close", "go": "abci-connection-closed", "rs": "invalid-argument-malformed-protobuf"}
        release_ns = input_cases[-1]["boundary"]["barrier_release_monotonic_ms"] * 1_000_000
        driver_receipt = {
                "barrier_release_monotonic_ns": release_ns,
                "pair_completed_monotonic_ns": release_ns + 2_000_000,
                "go_invoked_monotonic_ns": release_ns + 1_000_000,
                "rs_invoked_monotonic_ns": release_ns + 1_000_000,
                "timeout_ms": 100 if name == "core-grpc-cancel" else 5000 if name == "server-timeout" else 30000,
                "disconnect_after_write": name == "comet-commit-disconnect",
                "request_sha256": hashlib.sha256(request).hexdigest(),
                "driver_sha256": source_hashes["driver.py"],
                "transport_client_sha256": "6" * 64,
                "go_result_sha256": hashlib.sha256(harness.canonical_json(go_response)).hexdigest(),
                "rs_result_sha256": hashlib.sha256(harness.canonical_json(rs_response)).hexdigest(),
        }
        raw_driver = harness.canonical_json({"name": name, **driver_receipt,
                                             "go": go_response, "rs": rs_response})
        response_cases.append({
            "name": name, "admission": admission, "go": go_response, "rs": rs_response,
            "excluded_headers": list(harness.HTTP_EXCLUDED_HEADERS), "intentional_difference": difference,
            "driver": driver_receipt, "raw_driver": blob(raw_driver),
        })
        if admitted:
            result_common = {"gas_wanted": "50000", "gas_used": "40000", "events": [], "codespace": "", "log": "", "data": ""}
            sequence += 1
            balance = str(int(balance) - 1)
            app_hash = f"post-{index}"
            if name == "staking-delegate":
                delegation = "1"
            admission_only = name in {"comet-commit-disconnect", "core-grpc-cancel", "server-timeout"}
            indexed_tx_hash = (hashlib.sha256(inner_blob_tx).hexdigest()
                               if name == "blob-envelope" else tx_hash)
            row = {"included_count": 1, "tx_hash": tx_hash, "indexed_tx_hash": indexed_tx_hash,
                   "check_tx": None if admission_only else {"code": 0, **result_common},
                   "check_tx_source": "mempool-admission" if admission_only else "broadcast-response",
                   "deliver_tx": {"code": 5 if expected == "execution-failed" else 0, **result_common},
                   "post_state": {"height": pre["height"] + 1, "app_hash": app_hash, "balance": balance, "sequence": sequence, "delegation": delegation}}
            inclusion_cases.append({"name": name, "go": row, "rs": copy.deepcopy(row)})
            height = row["post_state"]["height"]
            extra_inputs = {
                "mempool-capacity-recovery": [("mempool_capacity", b"B" * 300), ("mempool_capacity", lifecycle_input_bytes["mempool_followup"])],
                "comet-commit-disconnect": [("disconnect", lifecycle_input_bytes["disconnect_followup"])],
                "core-grpc-cancel": [("grpc_cancel", lifecycle_input_bytes["grpc_cancel_followup"])],
                "server-timeout": [("server_timeout", lifecycle_input_bytes["server_timeout_followup"])],
            }.get(name, [])
            for lifecycle_name, extra_input in extra_inputs:
                sequence += 1
                balance = str(int(balance) - 1)
                height += 1
                app_hash = f"post-{index}-lifecycle-{len(lifecycle_indexed[lifecycle_name])}"
                extra_hash = hashlib.sha256(extra_input).hexdigest()
                extra_row = {
                    "included_count": 1, "tx_hash": extra_hash, "indexed_tx_hash": extra_hash,
                    "check_tx": {"code": 0, **result_common}, "check_tx_source": "broadcast-response",
                    "deliver_tx": {"code": 0, **result_common},
                    "post_state": {"height": height, "app_hash": app_hash, "balance": balance, "sequence": sequence, "delegation": delegation},
                }
                lifecycle_indexed[lifecycle_name].append({"go": extra_row, "rs": copy.deepcopy(extra_row)})
        else:
            row = {"included_count": 0, "tx_hash": tx_hash, "indexed_tx_hash": None,
                   "check_tx": None, "check_tx_source": None,
                   "deliver_tx": None, "post_state": None}
            inclusion_cases.append({"name": name, "go": row, "rs": copy.deepcopy(row)})
            height = pre["height"] + 1
    dump(root / "inputs.json", {
        "cases": input_cases,
        "capacity": {"tx_a": blob(b"A" * 300), "tx_b": blob(b"B" * 300),
                     "mempool_size": 0, "max_txs": 1,
                     "max_txs_bytes": harness.MEMPOOL_MAX_TXS_BYTES},
        "lifecycle_inputs": {name: blob(value) for name, value in lifecycle_input_bytes.items()},
    })
    dump(root / "paired-responses.json", {
        "cases": response_cases, "driver_sha256": source_hashes["driver.py"],
        "transport_client_sha256": "6" * 64,
    })
    dump(root / "inclusions.json", {"cases": inclusion_cases})
    phase_sets = {
        "mempool_capacity": ["a-admitted", "b-rejected", "a-included", "b-retry-admitted", "b-included", "followup-succeeded", "mempool-empty"],
        "disconnect": ["request-written", "client-disconnected", "tx-included", "retry-observed", "followup-succeeded", "mempool-empty"],
        "grpc_cancel": ["request-started", "client-canceled", "tx-included", "retry-observed", "followup-succeeded", "mempool-empty"],
        "server_timeout": ["request-started", "server-timeout", "capacity-probe-rejected", "tx-included", "retry-observed", "normal-config-restored", "followup-succeeded", "mempool-empty"],
    }
    lifecycle_outcomes = {
        "mempool_capacity": ["admitted", "mempool-full", "included", "admitted", "included", "included", "empty"],
        "disconnect": ["written", "disconnected", "included", "duplicate", "included", "empty"],
        "grpc_cancel": ["started", "canceled", "included", "duplicate", "included", "empty"],
        "server_timeout": ["started", "timeout", "capacity-rejected", "included", "duplicate", "restored", "included", "empty"],
    }
    lifecycle_mempools = {
        "mempool_capacity": [1, 1, 0, 1, 0, 0, 0],
        "disconnect": [0, 1, 0, 0, 0, 0],
        "grpc_cancel": [0, 1, 0, 0, 0, 0],
        "server_timeout": [0, 1, 1, 0, 0, 0, 0, 0],
    }
    request_hashes = {row["name"]: row["requests"]["go"]["sha256"] for row in input_cases}
    digest = lambda value: hashlib.sha256(value).hexdigest()
    lifecycle_hashes = {
        "mempool_capacity": [digest(b"A" * 300), digest(b"B" * 300), digest(b"A" * 300), digest(b"B" * 300), digest(b"B" * 300), digest(lifecycle_input_bytes["mempool_followup"]), None],
        "disconnect": [request_hashes["comet-commit-disconnect"]] * 4 + [digest(lifecycle_input_bytes["disconnect_followup"]), None],
        "grpc_cancel": [request_hashes["core-grpc-cancel"]] * 4 + [digest(lifecycle_input_bytes["grpc_cancel_followup"]), None],
        "server_timeout": [request_hashes["server-timeout"], request_hashes["server-timeout"], digest(lifecycle_input_bytes["server_timeout_probe"]), request_hashes["server-timeout"], request_hashes["server-timeout"], None, digest(lifecycle_input_bytes["server_timeout_followup"]), None],
    }
    lifecycle_generations = {
        "mempool_capacity": ["normal"] * 7,
        "disconnect": ["normal"] * 6,
        "grpc_cancel": ["normal"] * 6,
        "server_timeout": ["timeout"] * 5 + ["restored"] * 3,
    }
    lifecycles = {}
    lifecycle_case = {
        "mempool_capacity": "mempool-capacity-recovery",
        "disconnect": "comet-commit-disconnect",
        "grpc_cancel": "core-grpc-cancel",
        "server_timeout": "server-timeout",
    }
    barriers = {row["name"]: row["boundary"]["barrier_release_monotonic_ms"] for row in input_cases}
    input_by_name = {row["name"]: row for row in input_cases}
    response_by_name = {row["name"]: row for row in response_cases}
    inclusion_by_name = {row["name"]: row for row in inclusion_cases}
    response_absent = {
        "mempool_capacity": {"a-included", "b-included", "mempool-empty"},
        "disconnect": {"request-written", "client-disconnected", "tx-included", "mempool-empty"},
        "grpc_cancel": {"request-started", "client-canceled", "tx-included", "mempool-empty"},
        "server_timeout": {"request-started", "tx-included", "normal-config-restored", "mempool-empty"},
    }
    def lifecycle_response(lifecycle_name, phase, tx_hash):
        case_name = lifecycle_case[lifecycle_name]
        if lifecycle_name == "mempool_capacity" and phase == "a-admitted":
            return copy.deepcopy(response_by_name[case_name]["go"])
        if lifecycle_name == "server_timeout" and phase == "server-timeout":
            return copy.deepcopy(response_by_name[case_name]["go"])
        if phase in response_absent[lifecycle_name]:
            return None
        transport = "comet-jsonrpc" if lifecycle_name in {"mempool_capacity", "disconnect"} else "core-grpc"
        outcome = lifecycle_outcomes[lifecycle_name][phase_sets[lifecycle_name].index(phase)]
        if outcome in {"admitted", "included"}:
            code, log = 0, ""
        elif outcome == "mempool-full":
            code, log = 20, "mempool is full"
        elif outcome == "duplicate":
            code, log = 19, "tx already exists in cache"
        else:
            code, log = 32, "account sequence mismatch"
        if transport == "comet-jsonrpc":
            value = {"id": 187, "result": {"code": code, "log": log, "hash": tx_hash}}
            return {
                "transport": transport, "wire": blob(harness.canonical_json(value)), "http_status": 200,
                "http_headers": [["content-type", "application/json"]], "json": value,
                "grpc_status": None, "grpc_message": None, "initial_metadata": [], "trailing_metadata": [],
                "websocket_frames": [], "websocket_handshake": None, "client_error": None, "process_effect": None,
            }
        check_tx = harness.protobuf_uint_field(1, code) + harness.protobuf_bytes_field(3, log.encode())
        return {
            "transport": transport, "wire": blob(harness.protobuf_bytes_field(1, check_tx)), "http_status": None,
            "http_headers": [], "json": None, "grpc_status": 0, "grpc_message": "",
            "initial_metadata": [["content-type", "application/grpc"]], "trailing_metadata": [],
            "websocket_frames": [], "websocket_handshake": None, "client_error": None, "process_effect": None,
        }
    for lifecycle_name, phases in phase_sets.items():
        events = []
        case_name = lifecycle_case[lifecycle_name]
        pre_height = input_by_name[case_name]["pre_state"]["go"]["height"]
        post_height = inclusion_by_name[case_name]["go"]["post_state"]["height"]
        indexed_heights = [row["go"]["post_state"]["height"] for row in lifecycle_indexed[lifecycle_name]]
        if lifecycle_name == "mempool_capacity":
            status_heights = [pre_height, pre_height, post_height, post_height, indexed_heights[0], indexed_heights[1], indexed_heights[1]]
        elif lifecycle_name in {"disconnect", "grpc_cancel"}:
            status_heights = [pre_height, pre_height, post_height, post_height, indexed_heights[0], indexed_heights[0]]
        else:
            status_heights = [pre_height, pre_height, pre_height, post_height, post_height, post_height, indexed_heights[0], indexed_heights[0]]
        initial_delay = 120 if lifecycle_name == "mempool_capacity" else 10
        for offset, phase in enumerate(phases):
            tx_hash = lifecycle_hashes[lifecycle_name][offset]
            event_response = lifecycle_response(lifecycle_name, phase, tx_hash)
            event_time = (25010 + (offset - 5) * 80 if lifecycle_name == "server_timeout" and offset >= 5 else
                          barriers[case_name] + initial_delay + offset * 80)
            operation = None
            if event_response is not None and (lifecycle_name, phase) not in {
                ("mempool_capacity", "a-admitted"), ("server_timeout", "server-timeout"),
            }:
                operation = {
                    "request_sha256": tx_hash, "campaign_sha256": source_hashes["campaign.py"],
                    "transport_client_sha256": "6" * 64,
                    "barrier_release_monotonic_ns": (event_time - 3) * 1_000_000,
                    "invoked_monotonic_ns": (event_time - 2) * 1_000_000,
                    "completed_monotonic_ns": (event_time - 1) * 1_000_000,
                    "result_sha256": hashlib.sha256(harness.canonical_json(event_response)).hexdigest(),
                }
            events.append({
                "phase": phase, "generation": lifecycle_generations[lifecycle_name][offset], "monotonic_ms": event_time,
                "outcome": lifecycle_outcomes[lifecycle_name][offset],
                "tx_hash": tx_hash,
                "mempool_size": lifecycle_mempools[lifecycle_name][offset],
                "status_height": status_heights[offset],
                "response": event_response, "operation": operation,
            })
        lifecycles[lifecycle_name] = {
            "go": events, "rs": copy.deepcopy(events),
            "indexed_inclusions": lifecycle_indexed[lifecycle_name],
            "blocks": ([
                {
                    "generation": "timeout",
                    "go": {"height": 80 + block_offset, "block_hash": f"timeout-block-{block_offset}", "app_hash": f"timeout-app-{block_offset}", "time": (NOW + dt.timedelta(seconds=2 * block_offset)).isoformat(), "observed_monotonic_ms": 17100 + block_offset * 2000},
                    "rs": {"height": 80 + block_offset, "block_hash": f"timeout-block-{block_offset}", "app_hash": f"timeout-app-{block_offset}", "time": (NOW + dt.timedelta(seconds=2 * block_offset)).isoformat(), "observed_monotonic_ms": 17120 + block_offset * 2000},
                }
                for block_offset in range(4)
            ] if lifecycle_name == "server_timeout" else []),
            "block_intervals_ms": [2000, 2000, 2000] if lifecycle_name == "server_timeout" else [],
        }
    dump(root / "cleanup.json", {
        **lifecycles,
        "final": {"go_root": app_hash, "rs_root": app_hash, "go_balance": balance, "rs_balance": balance,
                  "go_sequence": sequence, "rs_sequence": sequence, "go_delegation": delegation,
                  "rs_delegation": delegation, "go_mempool": 0, "rs_mempool": 0,
                  "liveness": {"generation": "restored", "block": {
                      "go": {"height": height + 1, "block_hash": "final-block", "app_hash": app_hash,
                             "time": (NOW + dt.timedelta(seconds=60)).isoformat(), "observed_monotonic_ms": 30000},
                      "rs": {"height": height + 1, "block_hash": "final-block", "app_hash": app_hash,
                             "time": (NOW + dt.timedelta(seconds=60)).isoformat(), "observed_monotonic_ms": 30020}},
                      "state": {side: {"height": height + 1, "app_hash": app_hash, "account_number": 1,
                                       "sequence": sequence, "balance": balance, "delegation": delegation}
                                for side in ("go", "rs")},
                      "mempool": {"go": 0, "rs": 0}}},
    })
    generation_intervals = {
        "normal": {"started_monotonic_ms": 1000, "completed_monotonic_ms": 15000},
        "timeout": {"started_monotonic_ms": 16000, "completed_monotonic_ms": 24000},
        "restored": {"started_monotonic_ms": 25000, "completed_monotonic_ms": 33000},
    }
    samples = []
    processes = []
    for generation_index, generation in enumerate(("normal", "timeout", "restored")):
        interval = generation_intervals[generation]
        for role, ticks, executable_digest in (("go", 1 + generation_index * 2, "a" * 64), ("rs", 2 + generation_index * 2, "b" * 64)):
            processes.append({"generation": generation, "role": role, "pid_start_ticks": ticks, "executable_sha256": executable_digest})
            start = interval["started_monotonic_ms"] if role == "go" else interval["started_monotonic_ms"] + 100
            for sample_time in range(start, interval["completed_monotonic_ms"] + 1, 2000):
                samples.append({"monotonic_ms": sample_time, "generation": generation, "role": role, "pid_start_ticks": ticks, "executable_sha256": executable_digest, "rss_bytes": 1, "threads": 1, "fds": 1})
    samples.sort(key=lambda row: row["monotonic_ms"])
    dump(root / "resources.json", {
        "processes": processes,
        "samples": samples,
        "generation_intervals": generation_intervals,
        "disk_samples": [{"monotonic_ms": value, "free_bytes": harness.MIN_FREE_BYTES} for value in range(1000, 34001, 2000)],
        "preflight_estimate_bytes": 100, "preflight_free_bytes": harness.MIN_FREE_BYTES + 100,
        "controller_started_monotonic_ms": 0, "controller_completed_monotonic_ms": 35000,
        "process_group_drained": True, "process_group_term_sent": False, "process_group_kill_sent": False,
    })
    envelope = blob_envelope
    destination = "http://127.0.0.1:39999"
    raw_request = harness.canonical_json({
        "jsonrpc": "2.0", "id": 0, "method": "broadcast_tx_sync",
        "params": {"tx": base64.b64encode(envelope).decode()},
    })
    dump(root / "blob-proxy.json", {
        "request_id": 0, "method": "broadcast_tx_sync", "request_count": 1, "forward_count": 0,
        "source_hash": source_hashes["blob_proxy.py"], "raw_request": blob(raw_request), "destination": destination,
        "cli_argv": [
            "tx", "blob", "pay-for-blob", "0x" + namespace_id[-10:].hex(), "0x" + blob_data.hex(),
            "--from", "celestia1test", "--keyring-backend", "test", "--account-number", "1",
            "--sequence", "22", "--chain-id", "rs-devnet-187", "--offline", "--gas", "200000",
            "--fees", "100utia", "--node", destination, "--broadcast-mode", "sync", "--yes", "--output", "json",
        ],
        "cli_source_sha256": "7" * 64, "envelope": blob(envelope),
        "submitted_go_sha256": hashlib.sha256(envelope).hexdigest(), "submitted_rs_sha256": hashlib.sha256(envelope).hexdigest(),
        "inner_tx_sha256": hashlib.sha256(inner_blob_tx).hexdigest(), "unwrapped_inner_sha256": hashlib.sha256(inner_blob_tx).hexdigest(),
        "namespace": (b"\0" + namespace_id).hex(), "data_sha256": hashlib.sha256(blob_data).hexdigest(),
        "share_version": 0, "blob_signer_hex": "", "cli_signer": "celestia1test", "absent_before_go": True, "absent_before_rs": True,
        "read_methods": [],
    })
    support = {
        "capture_started": NOW.isoformat(), "capture_completed": (NOW + dt.timedelta(seconds=35)).isoformat(),
        "candidate_commit": CANDIDATE,
        "normalizations": ["lowercase-header-names", "trim-optional-whitespace", "sort-declared-multivalue-headers"],
        "intentional_differences": ["raw-malformed-protobuf"],
        "exclusions": ["http.date", "http.x-server-time"],
    }
    harness.finalize_manifest(root, support)


class VerifyTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        fixture(self.root)
        verify.EXPECTED_SEMANTIC_DIGEST = verify.semantic_digest(self.root)

    def tearDown(self):
        self.temp.cleanup()

    def rewrite(self, name: str, mutate) -> None:
        path = self.root / name
        value = harness.strict_json(path.read_bytes())
        mutate(value)
        if name == "paired-responses.json":
            for row in value["cases"]:
                row["driver"]["go_result_sha256"] = hashlib.sha256(harness.canonical_json(row["go"])).hexdigest()
                row["driver"]["rs_result_sha256"] = hashlib.sha256(harness.canonical_json(row["rs"])).hexdigest()
        dump(path, value)
        manifest = harness.strict_json((self.root / "manifest.json").read_bytes())
        support = manifest["support"]
        (self.root / "manifest.json").unlink()
        harness.finalize_manifest(self.root, support)

    def reject(self, code: str, function) -> None:
        with self.assertRaisesRegex((verify.VerificationError, harness.HarnessError), code):
            function()

    def test_baseline(self):
        verify.verify(self.root)

    def test_case_deletion(self):
        self.rewrite("inputs.json", lambda value: value["cases"].pop())
        self.reject("case.inventory", lambda: verify.verify(self.root))

    def test_request_rebinding(self):
        self.rewrite("inputs.json", lambda value: value["cases"][0]["requests"].__setitem__("rs", blob(b"different")))
        self.reject("case.request", lambda: verify.verify(self.root))

    def test_rest_invalid_base64_form(self):
        valid = harness.canonical_json({"tx_bytes": base64.b64encode(b"valid").decode(), "mode": "BROADCAST_MODE_SYNC"})
        self.rewrite("inputs.json", lambda value: value["cases"][0]["requests"].update({"go": blob(valid), "rs": blob(valid)}))
        self.reject("case.request", lambda: verify.verify(self.root))

    def test_cosmos_no_message_form(self):
        message_body = harness.protobuf_bytes_field(1, harness.protobuf_bytes_field(1, b"message"))
        txraw = harness.protobuf_bytes_field(1, message_body) + harness.protobuf_bytes_field(2, b"auth") + harness.protobuf_bytes_field(3, b"sig")
        self.rewrite("inputs.json", lambda value: value["cases"][1]["requests"].update({"go": blob(txraw), "rs": blob(txraw)}))
        self.reject("case.request", lambda: verify.verify(self.root))

    def test_raw_malformed_form(self):
        valid = harness.protobuf_bytes_field(1, b"valid")
        self.rewrite("inputs.json", lambda value: value["cases"][27]["requests"].update({"go": blob(valid), "rs": blob(valid)}))
        self.reject("case.request", lambda: verify.verify(self.root))

    def test_response_difference(self):
        self.rewrite("paired-responses.json", lambda value: value["cases"][0]["rs"]["json"].__setitem__("code", 9))
        self.reject("response.driver|response.wire|response.exact", lambda: verify.verify(self.root))

    def test_paired_fabricated_wire_body(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["cases"][0][side]["wire"] = blob(b'{"different":true}')
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.wire", lambda: verify.verify(self.root))

    def test_unknown_normalization(self):
        manifest = harness.strict_json((self.root / "manifest.json").read_bytes())
        manifest["support"]["normalizations"].append("drop-errors")
        dump(self.root / "manifest.json", manifest)
        self.reject("manifest.normalization", lambda: verify.verify(self.root))

    def test_nonloopback_endpoint(self):
        self.rewrite("topology.json", lambda value: value["generations"]["normal"]["endpoints"].__setitem__("go_rpc", "http://8.8.8.8:31001"))
        self.reject("topology.listeners", lambda: verify.verify(self.root))

    def test_config_extra_delta(self):
        self.rewrite("configs.json", lambda value: value["timeout"]["go"]["app.toml"].__setitem__("grpc.enable", False))
        self.reject("config.delta", lambda: verify.verify(self.root))

    def test_restored_config(self):
        self.rewrite("configs.json", lambda value: value["restored"]["go"]["config.toml"].__setitem__("rpc.timeout_broadcast_tx_commit", "250ms"))
        self.reject("config.restore", lambda: verify.verify(self.root))

    def test_p2p_listener_config_binding(self):
        self.rewrite("configs.json", lambda value: value["normal"]["go"]["config.toml"].__setitem__(
            "p2p.laddr", "tcp://127.0.0.1:39999"))
        self.reject("config.delta|config.listeners", lambda: verify.verify(self.root))

    def test_priv_validator_grpc_listener_must_be_disabled(self):
        def mutate(value):
            for phase in ("normal", "timeout", "restored"):
                for node in ("go", "rs"):
                    value[phase][node]["config.toml"]["priv_validator_grpc_laddr"] = "127.0.0.1:26669"
                    value["full"][phase][node]["config.toml"]["priv_validator_grpc_laddr"] = "127.0.0.1:26669"
        self.rewrite("configs.json", mutate)
        self.reject("config.isolation", lambda: verify.verify(self.root))

    def test_subscription_capacity_preserves_one_external_slot_per_role(self):
        self.rewrite("topology.json", lambda value: value.__setitem__(
            "max_subscription_clients", {"go": 1, "rs": 1}))
        self.reject("topology.capacity", lambda: verify.verify(self.root))

    def test_mempool_count_capacity_is_one(self):
        self.rewrite("topology.json", lambda value: value.__setitem__("max_txs", 2))
        self.reject("topology.capacity", lambda: verify.verify(self.root))

    def test_configured_mempool_size_is_zero(self):
        self.rewrite("topology.json", lambda value: value.__setitem__("mempool_size", 1))
        self.reject("topology.capacity", lambda: verify.verify(self.root))

    def test_mempool_byte_floor_cannot_be_recorded_below_effective_value(self):
        self.rewrite("topology.json", lambda value: value.__setitem__("max_txs_bytes", 686))
        self.reject("topology.capacity", lambda: verify.verify(self.root))

    def test_mempool_count_config_is_bound(self):
        def mutate(value):
            for phase in ("normal", "timeout", "restored"):
                for node in ("go", "rs"):
                    value[phase][node]["config.toml"]["mempool.size"] = 2
                    value["full"][phase][node]["config.toml"]["mempool"]["size"] = 2
        self.rewrite("configs.json", mutate)
        self.reject("config.capacity", lambda: verify.verify(self.root))

    def test_go_subscription_limit_reserves_three_internal_clients(self):
        def mutate(value):
            for phase in ("normal", "timeout", "restored"):
                value[phase]["go"]["config.toml"]["rpc.max_subscription_clients"] = 3
                value["full"][phase]["go"]["config.toml"]["rpc"]["max_subscription_clients"] = 3
        self.rewrite("configs.json", mutate)
        self.reject("config.capacity", lambda: verify.verify(self.root))

    def test_rust_subscription_limit_cannot_be_flattened_to_go_limit(self):
        def mutate(value):
            for phase in ("normal", "timeout", "restored"):
                value[phase]["rs"]["config.toml"]["rpc.max_subscription_clients"] = 4
                value["full"][phase]["rs"]["config.toml"]["rpc"]["max_subscription_clients"] = 4
        self.rewrite("configs.json", mutate)
        self.reject("config.capacity", lambda: verify.verify(self.root))

    def test_full_config_hidden_delta(self):
        self.rewrite("configs.json", lambda value: value["full"]["timeout"]["go"]["config.toml"].__setitem__(
            "instrumentation", {"prometheus": True}))
        self.reject("config.full", lambda: verify.verify(self.root))

    def test_blob_envelope_substitution(self):
        def mutate(value):
            replacement = b"inner-tx"
            value["envelope"] = blob(replacement)
            value["submitted_go_sha256"] = hashlib.sha256(replacement).hexdigest()
            value["submitted_rs_sha256"] = hashlib.sha256(replacement).hexdigest()
        self.rewrite("blob-proxy.json", mutate)
        self.reject("blob.proxy|blob.envelope", lambda: verify.verify(self.root))

    def test_disk_floor(self):
        self.rewrite("resources.json", lambda value: value["disk_samples"][1].__setitem__("free_bytes", harness.MIN_FREE_BYTES - 1))
        self.reject("resource.disk", lambda: verify.verify(self.root))

    def test_unlisted_file(self):
        (self.root / "extra.txt").write_text("extra")
        self.reject("inventory.exact", lambda: verify.verify(self.root))

    def test_symlink(self):
        (self.root / "link").symlink_to("inputs.json")
        self.reject("inventory.symlink", lambda: verify.verify(self.root))

    def test_empty_genesis(self):
        self.rewrite("genesis.json", lambda value: value.clear())
        self.reject("genesis", lambda: verify.verify(self.root))

    def test_genesis_app_version(self):
        self.rewrite("genesis.json", lambda value: value.__setitem__("app_version", "10.4.0"))
        self.reject("genesis.identity", lambda: verify.verify(self.root))

    def test_boundary_prestate_disconnect(self):
        self.rewrite("inputs.json", lambda value: value["cases"][0]["boundary"]["go"].__setitem__("app_hash", "other"))
        self.reject("boundary", lambda: verify.verify(self.root))

    def test_deliver_code_change(self):
        def mutate(value):
            value["cases"][5]["go"]["deliver_tx"]["code"] = 7
            value["cases"][5]["rs"]["deliver_tx"]["code"] = 7
        self.rewrite("inclusions.json", mutate)
        self.reject("inclusion.code", lambda: verify.verify(self.root))

    def test_inclusion_event_schema(self):
        def mutate(value):
            value["cases"][5]["go"]["deliver_tx"]["events"] = ["not-an-event"]
            value["cases"][5]["rs"]["deliver_tx"]["events"] = ["not-an-event"]
        self.rewrite("inclusions.json", mutate)
        self.reject("inclusion.events", lambda: verify.verify(self.root))

    def test_blob_index_hash_must_be_unwrapped_inner_transaction(self):
        def mutate(value):
            row = next(item for item in value["cases"] if item["name"] == "blob-envelope")
            for side in ("go", "rs"):
                row[side]["indexed_tx_hash"] = row[side]["tx_hash"]
        self.rewrite("inclusions.json", mutate)
        self.reject("inclusion.indexed_hash", lambda: verify.verify(self.root))

    def test_paired_gas_change_hits_reviewed_digest(self):
        def mutate(value):
            value["cases"][5]["go"]["deliver_tx"]["gas_used"] = "39999"
            value["cases"][5]["rs"]["deliver_tx"]["gas_used"] = "39999"
        self.rewrite("inclusions.json", mutate)
        self.reject("semantic.digest", lambda: verify.verify(self.root))

    def test_blob_fields_are_structured(self):
        def mutate(value):
            value["namespace"] = "garbage"
            value["data_sha256"] = "garbage"
            value["source_hash"] = "garbage"
        self.rewrite("blob-proxy.json", mutate)
        self.reject("blob.source|blob.fields", lambda: verify.verify(self.root))

    def test_protocol_version(self):
        self.rewrite("mainnet-observation.json", lambda value: value.__setitem__("app_protocol", 9))
        self.reject("observation.release", lambda: verify.verify(self.root))

    def test_consensus_freshness_bound_uses_active_round_zero_timeouts(self):
        timeout_info = {name: str(value) for name, value in harness.ACTIVE_TIMEOUT_INFO_NS.items()}
        self.assertEqual(harness.consensus_freshness_bound_ms(timeout_info), 5600)
        self.assertEqual(harness.consensus_commit_window_ms(timeout_info), 2600)

    def test_common_block_requires_a_post_call_height(self):
        self.assertEqual(campaign.next_common_height(10, [12, 11]), 13)
        self.assertEqual(campaign.next_common_height(20, [12, 11]), 20)
        self.reject("initial heights are invalid", lambda: campaign.next_common_height(1, [12]))

    def test_boundary_commit_window_is_bound_to_chronology(self):
        inputs = harness.strict_json((self.root / "inputs.json").read_bytes())
        boundary = inputs["cases"][0]["boundary"]
        harness.assert_boundary(boundary, "boundary")
        boundary["commit_window_remaining_ms"] += 1
        self.reject("window chronology differs", lambda: harness.assert_boundary(boundary, "boundary"))

    def test_boundary_preserves_one_second_for_admission(self):
        inputs = harness.strict_json((self.root / "inputs.json").read_bytes())
        boundary = inputs["cases"][0]["boundary"]
        observed = max(boundary[side]["received_monotonic_ms"] for side in ("go", "rs"))
        boundary["barrier_release_monotonic_ms"] = observed + 1601
        boundary["commit_window_remaining_ms"] = 999
        self.reject("window below 1 second", lambda: harness.assert_boundary(boundary, "boundary"))

    def test_consensus_timeout_rebinding_is_rejected(self):
        def mutate(value):
            record = value["responses"]["abci_info"]
            body = harness.strict_json(record["body"])
            body["result"]["response"]["timeout_info"]["timeout_propose"] = "30000000000"
            record["body"] = harness.canonical_json(body).decode()
            record["sha256"] = hashlib.sha256(record["body"].encode()).hexdigest()
        self.rewrite("mainnet-observation.json", mutate)
        self.reject("observation.timeouts", lambda: verify.verify(self.root))

    def test_real_abci_response_inventory_is_accepted(self):
        value = observation()
        response = harness.strict_json(value["responses"]["abci_info"]["body"])["result"]["response"]
        self.assertEqual(set(response), harness.ABCI_INFO_RESPONSE_FIELDS)
        self.assertEqual(campaign.consensus_bounds_from_observation(value), (5600, 2600))
        verify.verify_observation(value, NOW)

    def test_missing_real_abci_response_field_is_rejected(self):
        def mutate(value):
            record = value["responses"]["abci_info"]
            body = harness.strict_json(record["body"])
            del body["result"]["response"]["last_block_app_hash"]
            record["body"] = harness.canonical_json(body).decode()
            record["sha256"] = hashlib.sha256(record["body"].encode()).hexdigest()
        self.rewrite("mainnet-observation.json", mutate)
        mutated = harness.strict_json((self.root / "mainnet-observation.json").read_bytes())
        self.reject("mainnet observation ABCI payload.*fields differ",
                    lambda: campaign.consensus_bounds_from_observation(mutated))
        self.reject("observation ABCI response.*fields differ", lambda: verify.verify(self.root))

    def test_extra_abci_response_field_is_rejected(self):
        def mutate(value):
            record = value["responses"]["abci_info"]
            body = harness.strict_json(record["body"])
            body["result"]["response"]["unreviewed"] = "value"
            record["body"] = harness.canonical_json(body).decode()
            record["sha256"] = hashlib.sha256(record["body"].encode()).hexdigest()
        self.rewrite("mainnet-observation.json", mutate)
        mutated = harness.strict_json((self.root / "mainnet-observation.json").read_bytes())
        self.reject("mainnet observation ABCI payload.*fields differ",
                    lambda: campaign.consensus_bounds_from_observation(mutated))
        self.reject("observation ABCI response.*fields differ", lambda: verify.verify(self.root))

    def test_consensus_timeout_inventory_is_exact(self):
        timeout_info = {name: str(value) for name, value in harness.ACTIVE_TIMEOUT_INFO_NS.items()}
        timeout_info["unreviewed_timeout"] = "1"
        self.reject("timeout inventory differs", lambda: harness.consensus_freshness_bound_ms(timeout_info))

    def test_listener_binary_receipt_binding(self):
        self.rewrite("topology.json", lambda value: value["generations"]["normal"]["listeners"][0].__setitem__("executable_sha256", "f" * 64))
        self.reject("topology.binary", lambda: verify.verify(self.root))

    def test_restart_process_identity(self):
        normal_tick = lambda value: value["generations"]["normal"]["listeners"][0]["pid_start_ticks"]
        def mutate(value):
            tick = normal_tick(value)
            for listener in value["generations"]["timeout"]["listeners"]:
                if listener["name"].startswith("go_"):
                    listener["pid_start_ticks"] = tick
        self.rewrite("topology.json", mutate)
        self.reject("topology.process", lambda: verify.verify(self.root))

    def test_resource_generation_identity(self):
        def mutate(value):
            sample = next(row for row in value["samples"] if row["generation"] == "timeout" and row["role"] == "go")
            sample["pid_start_ticks"] = 1
        self.rewrite("resources.json", mutate)
        self.reject("resource.identity", lambda: verify.verify(self.root))

    def test_resource_cadence(self):
        self.rewrite("resources.json", lambda value: value["samples"].__setitem__(1, dict(value["samples"][1], monotonic_ms=10000)))
        self.reject("resource.order|resource.cadence", lambda: verify.verify(self.root))

    def test_resource_capture_span(self):
        self.rewrite("resources.json", lambda value: value["generation_intervals"]["normal"].__setitem__("completed_monotonic_ms", 9000))
        self.reject("resource.capture", lambda: verify.verify(self.root))

    def test_case_outside_sampling_interval(self):
        def mutate(value):
            value["cases"][0]["boundary"]["go"]["received_monotonic_ms"] = 16000
            value["cases"][0]["boundary"]["rs"]["received_monotonic_ms"] = 16000
            value["cases"][0]["boundary"]["barrier_release_monotonic_ms"] = 16000
            value["cases"][0]["boundary"]["commit_window_remaining_ms"] = 2600
        self.rewrite("inputs.json", mutate)
        self.reject("case.capture", lambda: verify.verify(self.root))

    def test_case_generation(self):
        self.rewrite("inputs.json", lambda value: next(row for row in value["cases"] if row["name"] == "server-timeout").__setitem__("generation", "normal"))
        self.reject("case.generation", lambda: verify.verify(self.root))

    def test_process_group_drain(self):
        self.rewrite("resources.json", lambda value: value.__setitem__("process_group_drained", False))
        self.reject("resource.process_group", lambda: verify.verify(self.root))

    def test_state_continuity(self):
        def mutate(value):
            value["cases"][6]["pre_state"]["go"]["sequence"] += 1
            value["cases"][6]["pre_state"]["rs"]["sequence"] += 1
        self.rewrite("inputs.json", mutate)
        self.reject("case.continuity", lambda: verify.verify(self.root))

    def test_jsonrpc_id(self):
        def mutate(value):
            value["cases"][3]["go"]["json"]["id"] = 9
            value["cases"][3]["rs"]["json"]["id"] = 9
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.wire|response.jsonrpc", lambda: verify.verify(self.root))

    def test_websocket_opcode(self):
        def mutate(value):
            value["cases"][18]["go"]["websocket_frames"][0]["opcode"] = "garbage"
            value["cases"][18]["rs"]["websocket_frames"][0]["opcode"] = "garbage"
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.websocket", lambda: verify.verify(self.root))

    def test_websocket_fixed_key_accept(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["cases"][18][side]["websocket_handshake"]["headers"][1][1] = "arbitrary-equal-value"
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.websocket", lambda: verify.verify(self.root))

    def test_grpc_metadata_required(self):
        def mutate(value):
            value["cases"][1]["go"]["initial_metadata"] = []
            value["cases"][1]["rs"]["initial_metadata"] = []
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.grpc", lambda: verify.verify(self.root))

    def test_capacity_is_tied_to_input_sizes(self):
        self.rewrite("inputs.json", lambda value: value["capacity"].__setitem__("tx_b", blob(b"")))
        self.reject("capacity.boundary", lambda: verify.verify(self.root))

    def test_capacity_input_count_is_bound(self):
        self.rewrite("inputs.json", lambda value: value["capacity"].__setitem__("max_txs", 2))
        self.reject("capacity.config", lambda: verify.verify(self.root))

    def test_capacity_input_mempool_size_is_bound(self):
        self.rewrite("inputs.json", lambda value: value["capacity"].__setitem__("mempool_size", 1))
        self.reject("capacity.config", lambda: verify.verify(self.root))

    def test_capacity_tx_a_is_submitted(self):
        self.rewrite("inputs.json", lambda value: value["capacity"].__setitem__("tx_a", blob(b"C" * 300)))
        self.reject("capacity.request", lambda: verify.verify(self.root))

    def test_response_driver_receipt(self):
        self.rewrite("paired-responses.json", lambda value: value.__setitem__("driver_sha256", "0" * 64))
        self.reject("response.driver", lambda: verify.verify(self.root))

    def test_case_driver_request_binding(self):
        self.rewrite("paired-responses.json", lambda value: value["cases"][0]["driver"].__setitem__("request_sha256", "0" * 64))
        self.reject("response.driver", lambda: verify.verify(self.root))

    def test_complete_raw_driver_substitution(self):
        self.rewrite("paired-responses.json", lambda value: value["cases"][0].__setitem__("raw_driver", blob(b"{}")))
        self.reject("raw driver|response.driver", lambda: verify.verify(self.root))

    def test_case_driver_barrier_binding(self):
        self.rewrite("paired-responses.json", lambda value: value["cases"][0]["driver"].__setitem__("barrier_release_monotonic_ns", 1))
        self.reject("response.driver", lambda: verify.verify(self.root))

    def test_lifecycle_phase_deletion(self):
        self.rewrite("cleanup.json", lambda value: value["grpc_cancel"]["go"].pop(1))
        self.reject("cleanup.phases", lambda: verify.verify(self.root))

    def test_lifecycle_monotonic_order(self):
        self.rewrite("cleanup.json", lambda value: value["disconnect"]["go"][2].__setitem__("monotonic_ms", 1))
        self.reject("cleanup.order", lambda: verify.verify(self.root))

    def test_lifecycle_raw_response_pair(self):
        def mutate(value):
            value["mempool_capacity"]["rs"][1]["response"]["json"]["ok"] = False
        self.rewrite("cleanup.json", mutate)
        self.reject("response.wire|cleanup.exact", lambda: verify.verify(self.root))

    def test_lifecycle_target_hash_binding(self):
        def mutate(value):
            for side in ("go", "rs"):
                for event in value["disconnect"][side][:4]:
                    event["tx_hash"] = "0" * 64
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.operation|cleanup.hash", lambda: verify.verify(self.root))

    def test_lifecycle_wrong_transport(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["disconnect"][side][3]["response"]["transport"] = "rest"
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.response", lambda: verify.verify(self.root))

    def test_lifecycle_generation(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["server_timeout"][side][5]["generation"] = "timeout"
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.generation", lambda: verify.verify(self.root))

    def test_lifecycle_campaign_receipt(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["disconnect"][side][3]["operation"]["campaign_sha256"] = "0" * 64
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.operation", lambda: verify.verify(self.root))

    def test_lifecycle_indexed_inclusion(self):
        self.rewrite("cleanup.json", lambda value: value["mempool_capacity"]["indexed_inclusions"].pop())
        self.reject("cleanup.inclusion", lambda: verify.verify(self.root))

    def test_lifecycle_decoded_outcome(self):
        def mutate(value):
            for side in ("go", "rs"):
                response = value["mempool_capacity"][side][1]["response"]
                response["json"]["result"]["log"] = "unrelated rejection"
                response["wire"] = blob(harness.canonical_json(response["json"]))
                operation = value["mempool_capacity"][side][1]["operation"]
                operation["result_sha256"] = hashlib.sha256(harness.canonical_json(response)).hexdigest()
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.outcome", lambda: verify.verify(self.root))

    def test_capacity_rejection_sample_may_follow_a_inclusion(self):
        def mutate(value):
            for side in ("go", "rs"):
                rejected = value["mempool_capacity"][side][1]
                included = value["mempool_capacity"][side][2]
                rejected["mempool_size"] = 0
                rejected["status_height"] = included["status_height"]
        self.rewrite("cleanup.json", mutate)
        verify.EXPECTED_SEMANTIC_DIGEST = verify.semantic_digest(self.root)
        verify.verify(self.root)

    def test_capacity_empty_rejection_sample_requires_a_inclusion_height(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["mempool_capacity"][side][1]["mempool_size"] = 0
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.link", lambda: verify.verify(self.root))

    def test_capacity_rejection_sample_rejects_unrelated_pool_size(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["mempool_capacity"][side][1]["mempool_size"] = 2
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.transitions", lambda: verify.verify(self.root))

    def test_lifecycle_followup_inclusion_state(self):
        def mutate(value):
            pair = value["disconnect"]["indexed_inclusions"][0]
            for side in ("go", "rs"):
                pair[side]["post_state"]["sequence"] += 1
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.inclusion", lambda: verify.verify(self.root))

    def test_admission_only_checktx_cannot_be_synthetic(self):
        def mutate(value):
            row = next(item for item in value["cases"] if item["name"] == "comet-commit-disconnect")
            for side in ("go", "rs"):
                row[side]["check_tx_source"] = "broadcast-response"
                row[side]["check_tx"] = {"code": 0, "gas_wanted": "0", "gas_used": "0", "events": [],
                                          "codespace": "", "log": "", "data": ""}
        self.rewrite("inclusions.json", mutate)
        self.reject("inclusion.result", lambda: verify.verify(self.root))

    def test_indexed_inclusion_closes_admission_poll_race(self):
        inclusions = json.loads((self.root / "inclusions.json").read_text())
        row = next(item for item in inclusions["cases"] if item["name"] == "comet-commit-disconnect")
        indexed_height = row["go"]["post_state"]["height"]

        def mutate_response(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            proof = item["admission"]["go"]
            proof.update(proof="indexed-inclusion", indexed_height=indexed_height,
                         status_height=indexed_height, mempool_present=False,
                         completed_after_barrier_ms=1200)

        def mutate_inclusion(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            for side in ("go", "rs"):
                item[side]["check_tx_source"] = "indexed-inclusion"

        def mutate_cleanup(value):
            inputs = json.loads((self.root / "inputs.json").read_text())
            barrier = next(
                row for row in inputs["cases"] if row["name"] == "comet-commit-disconnect"
            )["boundary"]["barrier_release_monotonic_ms"]
            for side in ("go", "rs"):
                side_events = value["disconnect"][side]
                side_events[1]["mempool_size"] = 0
                side_events[1]["status_height"] = indexed_height
                for offset, event in enumerate(side_events[1:], 1):
                    event["monotonic_ms"] = barrier + 1200 + (offset - 1) * 80

        self.rewrite("paired-responses.json", mutate_response)
        self.rewrite("inclusions.json", mutate_inclusion)
        self.rewrite("cleanup.json", mutate_cleanup)
        verify.EXPECTED_SEMANTIC_DIGEST = verify.semantic_digest(self.root)
        verify.verify(self.root)

    def test_indexed_admission_rejects_pending_cleanup_claim(self):
        inclusions = json.loads((self.root / "inclusions.json").read_text())
        indexed_height = next(
            item for item in inclusions["cases"] if item["name"] == "comet-commit-disconnect"
        )["go"]["post_state"]["height"]

        def mutate_response(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            proof = item["admission"]["go"]
            proof.update(proof="indexed-inclusion", indexed_height=indexed_height,
                         status_height=indexed_height, mempool_present=False,
                         completed_after_barrier_ms=1200)

        def mutate_inclusion(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            for side in ("go", "rs"):
                item[side]["check_tx_source"] = "indexed-inclusion"

        self.rewrite("paired-responses.json", mutate_response)
        self.rewrite("inclusions.json", mutate_inclusion)
        self.reject("cleanup.transitions|cleanup.link", lambda: verify.verify(self.root))

    def test_indexed_admission_respects_consensus_freshness_bound(self):
        inclusions = json.loads((self.root / "inclusions.json").read_text())
        indexed_height = next(
            item for item in inclusions["cases"] if item["name"] == "comet-commit-disconnect"
        )["go"]["post_state"]["height"]

        def mutate(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            proof = item["admission"]["go"]
            proof.update(proof="indexed-inclusion", indexed_height=indexed_height,
                         status_height=indexed_height, mempool_present=False,
                         completed_after_barrier_ms=5601)
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.admission", lambda: verify.verify(self.root))

    def test_indexed_admission_height_is_cross_bound(self):
        def mutate(value):
            item = next(row for row in value["cases"] if row["name"] == "comet-commit-disconnect")
            proof = item["admission"]["go"]
            proof.update(proof="indexed-inclusion", indexed_height=proof["status_height"] + 2,
                         status_height=proof["status_height"] + 2, mempool_present=False)
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.admission|inclusion.admission", lambda: verify.verify(self.root))

    def test_rejected_case_cannot_claim_indexed_admission(self):
        def mutate(value):
            item = next(row for row in value["cases"] if row["name"] == "rest-invalid-base64")
            proof = item["admission"]["go"]
            proof.update(proof="indexed-inclusion", indexed_height=proof["status_height"] + 1,
                         status_height=proof["status_height"] + 1)
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.admission", lambda: verify.verify(self.root))

    def test_lifecycle_inclusion_height_link(self):
        def mutate(value):
            for side in ("go", "rs"):
                for event in value["grpc_cancel"][side][2:]:
                    event["status_height"] += 10
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.link", lambda: verify.verify(self.root))

    def test_lifecycle_driver_chronology(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["disconnect"][side][0]["monotonic_ms"] -= 10
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.link", lambda: verify.verify(self.root))

    def test_server_timeout_block_intervals(self):
        self.rewrite("cleanup.json", lambda value: value["server_timeout"].__setitem__("block_intervals_ms", [900, 1300, 1400]))
        self.reject("cleanup.timing", lambda: verify.verify(self.root))

    def test_server_timeout_raw_block_time(self):
        def mutate(value):
            for side in ("go", "rs"):
                value["server_timeout"]["blocks"][2][side]["time"] = value["server_timeout"]["blocks"][1][side]["time"]
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.timing", lambda: verify.verify(self.root))

    def test_controller_rejects_arbitrary_driver(self):
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "capture"
            self.reject("receipt-bound outer campaign", lambda: capture.run_driver(output, 0, ["/bin/true"]))

    def test_process_group_live_members_ignore_only_zombies(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = pathlib.Path(directory)
            for pid, state, group, name in (
                (101, "S", 77, "live worker"),
                (102, "Z", 77, "dead ) worker"),
                (103, "R", 88, "other"),
            ):
                path = proc / str(pid)
                path.mkdir()
                (path / "stat").write_text(f"{pid} ({name}) {state} 1 {group} 0 0 0\n")
            self.assertEqual(capture.process_group_live_members(77, proc), {101})
            self.assertEqual(capture.process_group_live_members(88, proc), {103})
            self.assertEqual(capture.process_group_live_members(99, proc), set())

    def test_final_state_link(self):
        def mutate(value):
            value["final"]["go_root"] = "other"
            value["final"]["rs_root"] = "other"
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.final", lambda: verify.verify(self.root))

    def test_final_liveness_binding(self):
        def mutate(value):
            value["final"]["liveness"]["state"]["go"]["sequence"] += 1
            value["final"]["liveness"]["state"]["rs"]["sequence"] += 1
        self.rewrite("cleanup.json", mutate)
        self.reject("cleanup.final", lambda: verify.verify(self.root))

    def test_mainnet_raw_hash(self):
        self.rewrite("mainnet-observation.json", lambda value: value["responses"]["status"].__setitem__("body", "{}"))
        self.reject("observation.hash", lambda: verify.verify(self.root))

    def test_transport_client_source_receipt(self):
        self.rewrite("build-receipts.json", lambda value: value["transport_client"].__setitem__("source_sha256", "0" * 64))
        self.reject("receipt.client", lambda: verify.verify(self.root))

    def test_go_build_command(self):
        self.rewrite("build-receipts.json", lambda value: value["go"].__setitem__("command", ["false"]))
        self.reject("receipt.go", lambda: verify.verify(self.root))

    def test_request_jsonrpc_semantics(self):
        self.rewrite("inputs.json", lambda value: value["cases"][3]["request_semantics"].__setitem__("jsonrpc_id", 9))
        self.reject("case.jsonrpc", lambda: verify.verify(self.root))

    def test_malformed_process_outcome(self):
        def mutate(value):
            value["cases"][27]["go"]["process_effect"] = "continued"
        self.rewrite("paired-responses.json", mutate)
        self.reject("response.driver|response.grpc|response.policy", lambda: verify.verify(self.root))

    def test_blob_raw_request(self):
        self.rewrite("blob-proxy.json", lambda value: value.__setitem__("raw_request", blob(b'{}')))
        self.reject("BlobTx JSON-RPC request|blob.proxy", lambda: verify.verify(self.root))

    def test_blob_destination(self):
        self.rewrite("blob-proxy.json", lambda value: value.__setitem__("destination", "http://8.8.8.8:80"))
        self.reject("loopback", lambda: verify.verify(self.root))

    def test_blob_cli_frozen_flags(self):
        self.rewrite("blob-proxy.json", lambda value: value["cli_argv"].remove("--offline"))
        self.reject("blob.cli", lambda: verify.verify(self.root))

    def test_blob_cli_account_value(self):
        def mutate(value):
            position = value["cli_argv"].index("--account-number") + 1
            value["cli_argv"][position] = "999999"
        self.rewrite("blob-proxy.json", mutate)
        self.reject("blob.cli", lambda: verify.verify(self.root))

    def test_blob_cli_full_canonical_namespace(self):
        def mutate(value):
            decoded = harness.decode_canonical_blob_tx(
                harness.decode_blob(value["envelope"], "blob envelope")
            )
            value["cli_argv"][3] = "0x" + decoded["blobs"][0]["namespace_id"].hex()
        self.rewrite("blob-proxy.json", mutate)
        self.reject("blob.cli", lambda: verify.verify(self.root))

    def test_blob_nonzero_v0_namespace_prefix_after_internal_rebind(self):
        blob_artifact = harness.strict_json((self.root / "blob-proxy.json").read_bytes())
        inputs = harness.strict_json((self.root / "inputs.json").read_bytes())
        receipts = harness.strict_json((self.root / "build-receipts.json").read_bytes())
        original = harness.decode_blob(blob_artifact["envelope"], "blob envelope")
        decoded = harness.decode_canonical_blob_tx(original)["blobs"][0]
        replacement_id = b"\1" + decoded["namespace_id"][1:]
        replacement = original.replace(decoded["namespace_id"], replacement_id, 1)
        self.assertNotEqual(replacement, original)

        blob_artifact["envelope"] = blob(replacement)
        digest = hashlib.sha256(replacement).hexdigest()
        blob_artifact["submitted_go_sha256"] = digest
        blob_artifact["submitted_rs_sha256"] = digest
        blob_artifact["namespace"] = (b"\0" + replacement_id).hex()
        raw_request = json.loads(
            harness.decode_blob(blob_artifact["raw_request"], "raw blob request")
        )
        raw_request["params"]["tx"] = base64.b64encode(replacement).decode()
        blob_artifact["raw_request"] = blob(harness.canonical_json(raw_request))
        case = next(row for row in inputs["cases"] if row["name"] == "blob-envelope")
        case["requests"] = {"go": blob(replacement), "rs": blob(replacement)}

        self.reject(
            "blob.unwrap",
            lambda: verify.verify_blob(blob_artifact, inputs, receipts),
        )

    def test_blob_unwrap_data(self):
        self.rewrite("blob-proxy.json", lambda value: value.__setitem__("data_sha256", "0" * 64))
        self.reject("blob.unwrap", lambda: verify.verify(self.root))

    def test_campaign_decodes_core_checktx_events(self):
        attribute = (harness.protobuf_bytes_field(1, b"sender") +
                     harness.protobuf_bytes_field(2, b"celestia1test") +
                     harness.protobuf_uint_field(3, 1))
        event = harness.protobuf_bytes_field(1, b"message") + harness.protobuf_bytes_field(2, attribute)
        check_tx = (harness.protobuf_uint_field(1, 0) + harness.protobuf_bytes_field(7, event))
        response = {"transport": "core-grpc", "wire": blob(harness.protobuf_bytes_field(1, check_tx))}
        self.assertEqual(campaign.response_result(response)["events"], [{
            "type": "message",
            "attributes": [{"key": "sender", "value": "celestia1test", "index": True}],
        }])

    def test_state_query_retries_only_exact_future_height_error(self):
        error_body = harness.canonical_json({
            "code": 2, "message": campaign.FUTURE_HEIGHT_MESSAGE, "details": [],
        })
        error = urllib.error.HTTPError(
            "http://127.0.0.1:19317/account", 500, "Internal Server Error", {}, io.BytesIO(error_body)
        )

        class Response(io.BytesIO):
            status = 200

        response = Response(b'{"account":{"sequence":"4"}}')
        with mock.patch.object(campaign.urllib.request, "urlopen", side_effect=[error, response]) as opened:
            self.assertEqual(
                campaign.get_json("http://127.0.0.1:19317/account", "rs account", height=16),
                {"account": {"sequence": "4"}},
            )
        self.assertEqual(opened.call_count, 2)
        request = opened.call_args_list[0].args[0]
        self.assertEqual(request.get_header("X-cosmos-block-height"), "16")
        self.assertTrue(campaign.future_height_error(error_body))
        self.assertFalse(campaign.future_height_error(harness.canonical_json({
            "code": 2, "message": "unrelated internal error", "details": [],
        })))

    def test_state_query_future_height_timeout_is_not_missing(self):
        error_body = harness.canonical_json({
            "code": 2, "message": campaign.FUTURE_HEIGHT_MESSAGE, "details": [],
        })
        error = urllib.error.HTTPError(
            "http://127.0.0.1:19317/account", 500, "Internal Server Error", {}, io.BytesIO(error_body)
        )
        with mock.patch.object(campaign.urllib.request, "urlopen", side_effect=error):
            self.reject("future height remained unavailable", lambda: campaign.get_json(
                "http://127.0.0.1:19317/account", "rs account", height=16, missing=True,
                _future_height_timeout=0,
            ))

    def test_state_query_unrelated_http_500_fails_immediately(self):
        error = urllib.error.HTTPError(
            "http://127.0.0.1:19317/account", 500, "Internal Server Error", {},
            io.BytesIO(harness.canonical_json({"code": 2, "message": "unrelated", "details": []})),
        )
        with mock.patch.object(campaign.urllib.request, "urlopen", side_effect=error) as opened:
            self.reject("rs account HTTP 500", lambda: campaign.get_json(
                "http://127.0.0.1:19317/account", "rs account", height=16,
            ))
        self.assertEqual(opened.call_count, 1)

    def test_campaign_rejects_home_genesis_substitution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            genesis = {"consensus": {"params": {}, "validators": [
                {"name": "go", "address": "A", "pub_key": {"type": "ed25519", "value": "go"}},
                {"name": "rs", "address": "B", "pub_key": {"type": "ed25519", "value": "rs"}},
            ]}}
            homes = {}
            for role, address in (("go", "A"), ("rs", "B")):
                config = root / role / "config"
                config.mkdir(parents=True)
                homes[role] = root / role
                dump(config / "genesis.json", genesis)
                dump(config / "priv_validator_key.json", {
                    "address": address,
                    "pub_key": {"type": "ed25519", "value": role},
                })
            campaign.validate_home_provenance(homes, genesis)
            dump(homes["rs"] / "config" / "genesis.json", {"consensus": {"params": {}, "validators": []}})
            self.reject("home genesis differs", lambda: campaign.validate_home_provenance(homes, genesis))

    def test_substituted_campaign_module_cannot_execute_before_hash_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            marker = root / "executed"
            source = root / "driver.py"
            source.write_text(f"from pathlib import Path\nPath({str(marker)!r}).write_text('bad')\n")
            self.reject("source hash differs", lambda: campaign.load_reviewed_module(source, "0" * 64, "driver"))
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
