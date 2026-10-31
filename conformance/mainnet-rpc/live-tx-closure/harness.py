#!/usr/bin/env python3
"""Fail-closed capture primitives for the paired live-transaction campaign."""

from __future__ import annotations

import base64
import hashlib
import ipaddress
import json
import math
import os
import pathlib
import signal
import socket
import threading
import time
import urllib.parse
from dataclasses import dataclass
from typing import Any, Iterable


SCHEMA = "celestia-rpc-live-tx-closure-v1"
MIN_FREE_BYTES = 3 * 1024**3
ABORT_MARGIN_BYTES = 128 * 1024**2
HTTP_EXCLUDED_HEADERS = ("date", "x-server-time")
ORDER_INSENSITIVE_HEADERS = ("access-control-allow-headers", "access-control-allow-methods")
FIXED_WEBSOCKET_KEY = "MDEyMzQ1Njc4OWFiY2RlZg=="
FIXED_WEBSOCKET_ACCEPT = "BACScCJPNqyz+UBoqMH89VmURoA="
ACTIVE_TIMEOUT_INFO_NS = {
    "timeout_propose": 3_000_000_000,
    "timeout_propose_delta": 500_000_000,
    "timeout_prevote": 2_000_000_000,
    "timeout_prevote_delta": 500_000_000,
    "timeout_precommit": 3_000_000_000,
    "timeout_precommit_delta": 500_000_000,
    "timeout_commit": 500_000_000,
    "delayed_precommit_timeout": 2_100_000_000,
}
ABCI_INFO_RESPONSE_FIELDS = {
    "data", "version", "app_version", "last_block_height", "last_block_app_hash", "timeout_info",
}
SUBSCRIPTION_CLIENT_LIMITS = {"go": 2, "rs": 1}

CASE_SPECS = (
    ("rest-invalid-base64", "rest", "sync", "transport", "rejected"),
    ("cosmos-grpc-no-message", "cosmos-grpc", "sync", "bank", "rejected"),
    ("rest-bad-signature", "rest", "sync", "bank", "rejected"),
    ("comet-sync-low-gas", "comet-jsonrpc", "sync", "bank", "rejected"),
    ("core-grpc-future-sequence", "core-grpc", "commit", "bank", "rejected"),
    ("bank-duplicate", "comet-jsonrpc", "sync", "bank", "included"),
    ("rest-async-bank", "rest", "async", "bank", "included"),
    ("cosmos-grpc-sync-bank", "cosmos-grpc", "sync", "bank", "included"),
    ("cosmos-grpc-async-bank", "cosmos-grpc", "async", "bank", "included"),
    ("cosmos-grpc-block-unsupported", "cosmos-grpc", "block", "bank", "rejected"),
    ("core-grpc-commit-bank", "core-grpc", "commit", "bank", "included"),
    ("comet-jsonrpc-async-bank", "comet-jsonrpc", "async", "bank", "included"),
    ("comet-jsonrpc-sync-bank", "comet-jsonrpc", "sync", "bank", "included"),
    ("comet-jsonrpc-commit-bank", "comet-jsonrpc", "commit", "bank", "included"),
    ("execution-failure", "cosmos-grpc", "sync", "bank", "execution-failed"),
    ("comet-uri-async-bank", "comet-uri", "async", "bank", "included"),
    ("comet-uri-sync-bank", "comet-uri", "sync", "bank", "included"),
    ("comet-uri-commit-bank", "comet-uri", "commit", "bank", "included"),
    ("comet-websocket-async-bank", "comet-websocket", "async", "bank", "included"),
    ("comet-websocket-sync-bank", "comet-websocket", "sync", "bank", "included"),
    ("comet-websocket-commit-bank", "comet-websocket", "commit", "bank", "included"),
    ("mempool-capacity-recovery", "comet-jsonrpc", "sync", "bank", "included"),
    ("comet-commit-disconnect", "comet-jsonrpc", "commit-disconnect", "bank", "included"),
    ("core-grpc-cancel", "core-grpc", "commit-cancel-100ms", "bank", "included"),
    ("blob-envelope", "comet-jsonrpc", "sync", "blob", "included"),
    ("staking-delegate", "cosmos-grpc", "sync", "staking", "included"),
    ("server-timeout", "core-grpc", "commit-server-timeout-250ms", "bank", "included"),
    ("raw-malformed-protobuf", "core-grpc", "terminal-malformed", "transport", "policy-difference"),
)

CASE_NAMES = tuple(spec[0] for spec in CASE_SPECS)


class HarnessError(ValueError):
    """A capture condition failed and the run cannot become evidence."""


def consensus_freshness_bound_ms(timeout_info: Any) -> int:
    """Return the round-zero header-to-observation bound for the active release."""
    require(isinstance(timeout_info, dict) and set(timeout_info) == set(ACTIVE_TIMEOUT_INFO_NS),
            "active consensus timeout inventory differs")
    parsed: dict[str, int] = {}
    for name, expected in ACTIVE_TIMEOUT_INFO_NS.items():
        value = timeout_info[name]
        require(isinstance(value, str) and value.isdecimal(), f"active consensus {name} is invalid")
        parsed[name] = int(value)
        require(parsed[name] == expected, f"active consensus {name} differs")
    # Header time is the previous commit's weighted-median precommit time. A
    # healthy round-zero successor is therefore observable within its delayed
    # precommit and commit waits, plus one complete proposal window.
    nanoseconds = (parsed["delayed_precommit_timeout"] + parsed["timeout_commit"] +
                   parsed["timeout_propose"])
    require(nanoseconds > 0 and nanoseconds % 1_000_000 == 0,
            "active consensus freshness bound is invalid")
    return nanoseconds // 1_000_000


def consensus_commit_window_ms(timeout_info: Any) -> int:
    """Return the minimum current-commit to next round-zero precommit window."""
    consensus_freshness_bound_ms(timeout_info)
    nanoseconds = (ACTIVE_TIMEOUT_INFO_NS["timeout_commit"] +
                   ACTIVE_TIMEOUT_INFO_NS["delayed_precommit_timeout"])
    require(nanoseconds > 0 and nanoseconds % 1_000_000 == 0,
            "active consensus commit window is invalid")
    return nanoseconds // 1_000_000


def require(condition: bool, message: str) -> None:
    if not condition:
        raise HarnessError(message)


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def strict_json(data: str | bytes) -> Any:
    if isinstance(data, bytes):
        data = data.decode("utf-8")
    value = json.loads(
        data,
        object_pairs_hook=_pairs,
        parse_constant=lambda token: (_ for _ in ()).throw(HarnessError(f"non-finite JSON number: {token}")),
    )
    return value


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n").encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def reject_symlink_components(path: pathlib.Path, label: str) -> None:
    """Reject a symlink in any existing lexical component before resolution."""
    candidate = path.absolute()
    current = pathlib.Path(candidate.anchor)
    for component in candidate.parts[1:]:
        current /= component
        if current.exists() or current.is_symlink():
            require(not current.is_symlink(), f"{label}: symlinked path component: {current}")


def require_fields(value: Any, expected: Iterable[str], label: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{label}: expected object")
    expected_set = set(expected)
    require(set(value) == expected_set, f"{label}: fields differ: {sorted(set(value) ^ expected_set)}")
    return value


def loopback_endpoint(value: str, label: str) -> tuple[str, int]:
    parsed = urllib.parse.urlsplit(value if "://" in value else f"tcp://{value}")
    require(parsed.scheme in {"http", "https", "ws", "wss", "tcp"}, f"{label}: unsupported endpoint scheme")
    require(parsed.hostname is not None and parsed.port is not None, f"{label}: endpoint needs host and port")
    try:
        address = ipaddress.ip_address(parsed.hostname)
    except ValueError as error:
        raise HarnessError(f"{label}: endpoint must use a numeric loopback address") from error
    require(address.is_loopback, f"{label}: endpoint is not loopback")
    return str(address), parsed.port


def validate_endpoints(endpoints: dict[str, str]) -> None:
    expected = {"go_rpc", "go_core", "go_sdk", "go_rest", "go_p2p",
                "rs_rpc", "rs_core", "rs_sdk", "rs_rest", "rs_p2p"}
    require(set(endpoints) == expected, "endpoint inventory differs")
    listeners = [loopback_endpoint(value, name) for name, value in endpoints.items()]
    require(len(set(listeners)) == len(listeners), "endpoint listeners are not unique")


def validate_case_inventory(cases: list[dict[str, Any]]) -> None:
    require([case.get("name") for case in cases] == list(CASE_NAMES), "case inventory/order differs")
    for case, spec in zip(cases, CASE_SPECS, strict=True):
        require_fields(case, {"name", "transport", "mode", "family", "expected_outcome"}, f"case spec {spec[0]}")
        require(tuple(case[field] for field in ("name", "transport", "mode", "family", "expected_outcome")) == spec,
                f"case mapping differs: {spec[0]}")


def decode_blob(record: dict[str, Any], label: str) -> bytes:
    require_fields(record, {"base64", "sha256", "size"}, label)
    try:
        data = base64.b64decode(record["base64"], validate=True)
    except (ValueError, TypeError) as error:
        raise HarnessError(f"{label}: invalid base64") from error
    require(record["size"] == len(data), f"{label}: byte size differs")
    require(record["sha256"] == sha256(data), f"{label}: byte hash differs")
    return data


def protobuf_varint(value: int) -> bytes:
    require(value >= 0, "negative protobuf varint")
    encoded = bytearray()
    while value >= 0x80:
        encoded.append((value & 0x7F) | 0x80)
        value >>= 7
    encoded.append(value)
    return bytes(encoded)


def protobuf_bytes_field(number: int, value: bytes) -> bytes:
    return protobuf_varint(number << 3 | 2) + protobuf_varint(len(value)) + value


def protobuf_uint_field(number: int, value: int) -> bytes:
    return b"" if value == 0 else protobuf_varint(number << 3) + protobuf_varint(value)


def _consume_varint(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    for shift in range(0, 70, 7):
        require(offset < len(data), "truncated protobuf varint")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            require(shift < 63 or byte <= 1, "overflowing protobuf varint")
            return value, offset
    raise HarnessError("overflowing protobuf varint")


def protobuf_fields(data: bytes) -> list[tuple[int, int, bytes | int]]:
    fields: list[tuple[int, int, bytes | int]] = []
    offset = 0
    while offset < len(data):
        tag, offset = _consume_varint(data, offset)
        number, wire = tag >> 3, tag & 7
        require(0 < number < 1 << 29, "invalid protobuf field number")
        if wire == 0:
            value, offset = _consume_varint(data, offset)
        elif wire == 2:
            size, offset = _consume_varint(data, offset)
            require(offset + size <= len(data), "truncated protobuf bytes field")
            value = data[offset : offset + size]
            offset += size
        else:
            raise HarnessError(f"unsupported protobuf wire type {wire}")
        fields.append((number, wire, value))
    return fields


def decode_canonical_blob_tx(envelope: bytes) -> dict[str, Any]:
    tx: bytes | None = None
    blobs: list[dict[str, Any]] = []
    type_id: bytes | None = None
    encoded = bytearray()
    outer_numbers: list[int] = []
    for number, wire, value in protobuf_fields(envelope):
        outer_numbers.append(number)
        require(wire == 2 and isinstance(value, bytes) and number in {1, 2, 3}, "unknown BlobTx field")
        if number == 1:
            require(tx is None, "duplicate BlobTx tx field")
            tx = value
            encoded += protobuf_bytes_field(1, value)
        elif number == 2:
            blob_fields: dict[int, bytes | int] = {}
            blob_encoded = bytearray()
            last_blob_number = 0
            for blob_number, blob_wire, blob_value in protobuf_fields(value):
                require(blob_number > last_blob_number, "Blob fields are not in canonical order")
                last_blob_number = blob_number
                require(blob_number in {1, 2, 3, 4, 5} and blob_number not in blob_fields,
                        "unknown or duplicate Blob field")
                expected_wire = 2 if blob_number in {1, 2, 5} else 0
                require(blob_wire == expected_wire, "Blob field wire type differs")
                blob_fields[blob_number] = blob_value
                if blob_wire == 2:
                    assert isinstance(blob_value, bytes)
                    blob_encoded += protobuf_bytes_field(blob_number, blob_value)
                else:
                    assert isinstance(blob_value, int)
                    blob_encoded += protobuf_uint_field(blob_number, blob_value)
            require(1 in blob_fields and 2 in blob_fields, "Blob lacks namespace or data")
            namespace_id = blob_fields[1]
            data = blob_fields[2]
            require(isinstance(namespace_id, bytes) and len(namespace_id) == 28, "Blob namespace id length differs")
            require(isinstance(data, bytes), "Blob data is not bytes")
            blobs.append({
                "namespace_id": namespace_id,
                "data": data,
                "share_version": int(blob_fields.get(3, 0)),
                "namespace_version": int(blob_fields.get(4, 0)),
                "signer": blob_fields.get(5, b""),
            })
            encoded += protobuf_bytes_field(2, bytes(blob_encoded))
        else:
            require(type_id is None, "duplicate BlobTx type id")
            type_id = value
            encoded += protobuf_bytes_field(3, value)
    require(tx is not None and tx, "BlobTx inner transaction missing")
    require(blobs, "BlobTx blobs missing")
    require(type_id == b"BLOB", "BlobTx type id differs")
    require(outer_numbers == [1] + [2] * len(blobs) + [3], "BlobTx fields are not in canonical order")
    require(bytes(encoded) == envelope, "BlobTx is not canonical protobuf encoding")
    return {"tx": tx, "blobs": blobs, "type_id": "BLOB"}


def assert_txraw_no_messages(txraw: bytes) -> None:
    """Require a canonical TxRaw whose decoded TxBody has no messages."""
    fields = protobuf_fields(txraw)
    require([number for number, _, _ in fields] == [1, 2, 3], "no-message TxRaw field inventory differs")
    require(all(wire == 2 and isinstance(value, bytes) for _, wire, value in fields),
            "no-message TxRaw contains a non-bytes field")
    body = fields[0][2]
    auth = fields[1][2]
    signature = fields[2][2]
    assert isinstance(body, bytes) and isinstance(auth, bytes) and isinstance(signature, bytes)
    require(auth and signature, "no-message TxRaw lacks auth info or signature")
    body_fields = protobuf_fields(body)
    require(all(number != 1 for number, _, _ in body_fields), "no-message TxBody contains a message")
    require(protobuf_bytes_field(1, body) + protobuf_bytes_field(2, auth) + protobuf_bytes_field(3, signature) == txraw,
            "no-message TxRaw is not canonical protobuf")


def assert_invalid_rest_base64_request(raw: bytes) -> None:
    request = strict_json(raw)
    require_fields(request, {"tx_bytes", "mode"}, "REST invalid-base64 request")
    require(request["mode"] == "BROADCAST_MODE_SYNC" and isinstance(request["tx_bytes"], str),
            "REST invalid-base64 request mode/value differs")
    try:
        base64.b64decode(request["tx_bytes"], validate=True)
    except (ValueError, TypeError):
        return
    raise HarnessError("REST invalid-base64 request contains valid base64")


def assert_malformed_protobuf(raw: bytes) -> None:
    require(raw, "raw malformed protobuf is empty")
    try:
        protobuf_fields(raw)
    except HarnessError:
        return
    raise HarnessError("raw malformed protobuf request is structurally valid")


def stable_headers(headers: list[list[str]], *, exclusions: tuple[str, ...] = HTTP_EXCLUDED_HEADERS) -> list[list[str]]:
    projected: dict[str, list[str]] = {}
    for pair in headers:
        require(isinstance(pair, list) and len(pair) == 2 and all(isinstance(item, str) for item in pair),
                "header must be a string pair")
        name, value = pair
        lowered = name.lower()
        require(name == lowered, f"header name is not lowercased: {name}")
        require(lowered not in projected or name == lowered, f"header case alias: {name}")
        if lowered not in exclusions:
            projected.setdefault(lowered, []).append(value.strip(" \t"))
    stable: list[list[str]] = []
    for name in sorted(projected):
        values = sorted(projected[name]) if name in ORDER_INSENSITIVE_HEADERS else projected[name]
        stable.extend([name, value] for value in values)
    return stable


def assert_request_identity(requests: dict[str, Any], label: str) -> bytes:
    require_fields(requests, {"go", "rs"}, f"{label}: requests")
    go = decode_blob(requests["go"], f"{label}: Go request")
    rs = decode_blob(requests["rs"], f"{label}: Rust request")
    require(go == rs, f"{label}: paired request bytes differ")
    return go


def assert_boundary(boundary: dict[str, Any], label: str) -> None:
    require_fields(boundary, {"go", "rs", "barrier_release_monotonic_ms", "commit_window_remaining_ms"},
                   f"{label}: boundary")
    fields = {"height", "block_hash", "app_hash", "block_time", "received_monotonic_ms"}
    go = require_fields(boundary["go"], fields, f"{label}: Go boundary")
    rs = require_fields(boundary["rs"], fields, f"{label}: Rust boundary")
    for field in ("height", "block_hash", "app_hash", "block_time"):
        require(go[field] == rs[field], f"{label}: boundary {field} differs")
    require(abs(go["received_monotonic_ms"] - rs["received_monotonic_ms"]) <= 250,
            f"{label}: boundary receipt skew exceeds 250 ms")
    release = boundary["barrier_release_monotonic_ms"]
    observed = max(go["received_monotonic_ms"], rs["received_monotonic_ms"])
    require(release >= observed,
            f"{label}: barrier predates boundary")
    timeout_info = {name: str(value) for name, value in ACTIVE_TIMEOUT_INFO_NS.items()}
    expected_remaining = consensus_commit_window_ms(timeout_info) - (release - observed)
    require(boundary["commit_window_remaining_ms"] == expected_remaining,
            f"{label}: admission window chronology differs")
    require(expected_remaining >= 1000, f"{label}: admission window below 1 second")


def assert_admission(admission: dict[str, Any], boundary_height: int, expected: str, label: str) -> None:
    require_fields(admission, {"go", "rs"}, f"{label}: admission")
    fields = {"outcome", "completed_after_barrier_ms", "status_height", "mempool_present", "tx_hash"}
    rows = [require_fields(admission[side], fields, f"{label}: {side} admission") for side in ("go", "rs")]
    expected_admission = "rejected" if expected in {"rejected", "policy-difference"} else "admitted"
    for row in rows:
        require(row["outcome"] == expected_admission, f"{label}: unexpected admission outcome")
        require(0 <= row["completed_after_barrier_ms"] <= 1000, f"{label}: admission outside 1 second")
        require(row["status_height"] == boundary_height, f"{label}: height advanced during admission")
        require(row["mempool_present"] is (expected_admission == "admitted"), f"{label}: mempool proof differs")
    require(rows[0]["tx_hash"] == rows[1]["tx_hash"], f"{label}: transaction hash differs")


def validate_config_delta(normal: dict[str, Any], timeout: dict[str, Any]) -> None:
    require(set(normal) == set(timeout), "normal/timeout config file inventory differs")
    changed: list[tuple[str, str, str]] = []
    for node in sorted(normal):
        require(set(normal[node]) == set(timeout[node]), f"config file inventory differs: {node}")
        for filename in sorted(normal[node]):
            left = normal[node][filename]
            right = timeout[node][filename]
            require(isinstance(left, dict) and isinstance(right, dict), "config projections must be objects")
            require(set(left) == set(right), f"config field inventory differs: {node}/{filename}")
            for field in sorted(left):
                if left[field] != right[field]:
                    changed.append((node, filename, field))
    require(changed == [
        ("go", "config.toml", "rpc.timeout_broadcast_tx_commit"),
        ("rs", "config.toml", "rpc.timeout_broadcast_tx_commit"),
    ],
            f"unexpected timeout config delta: {changed}")
    for node in ("go", "rs"):
        require(normal[node]["config.toml"]["rpc.timeout_broadcast_tx_commit"] == "10s", "normal commit timeout differs")
        require(timeout[node]["config.toml"]["rpc.timeout_broadcast_tx_commit"] == "250ms", "timeout commit timeout differs")


@dataclass
class DiskSample:
    monotonic_ms: int
    free_bytes: int


class DiskFloorWatchdog:
    """Sample free space and terminate the owned process group before the hard floor."""

    def __init__(self, path: pathlib.Path, process_group: int, interval: float = 2.0) -> None:
        self.path = path
        self.process_group = process_group
        self.interval = interval
        self.samples: list[DiskSample] = []
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        require(self._thread is None, "disk watchdog already started")
        require(self.process_group > 1 and self.process_group != os.getpgrp(), "refusing to watch the controller process group")
        self._thread = threading.Thread(target=self._run, name="live-tx-disk-watchdog", daemon=True)
        self._thread.start()

    def _run(self) -> None:
        while not self._stop.is_set():
            free = os.statvfs(self.path).f_bavail * os.statvfs(self.path).f_frsize
            self.samples.append(DiskSample(math.floor(time.monotonic() * 1000), free))
            if free < MIN_FREE_BYTES + ABORT_MARGIN_BYTES:
                os.killpg(self.process_group, signal.SIGTERM)
                self._stop.set()
                return
            self._stop.wait(self.interval)

    def stop(self) -> list[dict[str, int]]:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=self.interval + 1)
        return [sample.__dict__ for sample in self.samples]


def preflight_output(output: pathlib.Path, estimate_bytes: int) -> None:
    require(not output.exists(), "capture output already exists")
    reject_symlink_components(output.parent, "capture output parent")
    parent = output.parent.resolve(strict=True)
    stat = os.statvfs(parent)
    free = stat.f_bavail * stat.f_frsize
    require(estimate_bytes >= 0, "capture estimate is negative")
    require(free - estimate_bytes >= MIN_FREE_BYTES, "capture estimate crosses the 3 GiB free-space floor")


def assert_expected_listeners(endpoints: dict[str, str], listeners: list[dict[str, Any]]) -> None:
    validate_endpoints(endpoints)
    expected = {loopback_endpoint(value, name) for name, value in endpoints.items()}
    actual: set[tuple[str, int]] = set()
    for row in listeners:
        require_fields(row, {"name", "address", "port", "pid_start_ticks", "executable_sha256"}, "listener")
        require(row["name"] in endpoints, "listener name is not configured")
        host, port = loopback_endpoint(f"tcp://{row['address']}:{row['port']}", "listener")
        require((host, port) == loopback_endpoint(endpoints[row["name"]], row["name"]), "listener address differs from named endpoint")
        require(isinstance(row["pid_start_ticks"], int) and row["pid_start_ticks"] > 0, "listener start ticks")
        require(isinstance(row["executable_sha256"], str) and len(row["executable_sha256"]) == 64,
                "listener executable hash")
        actual.add((host, port))
    require(actual == expected, "listener inventory differs from configured endpoints")


def finalize_manifest(root: pathlib.Path, support: dict[str, Any]) -> dict[str, Any]:
    """Hash a completed sanitized capture after rejecting unsafe filesystem entries."""
    reject_symlink_components(root, "capture root")
    root = root.resolve(strict=True)
    entries: dict[str, dict[str, Any]] = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        require(not path.is_symlink(), f"symlink is forbidden: {relative}")
        require(path.is_file() or path.is_dir(), f"nonregular entry is forbidden: {relative}")
        if path.is_file() and relative != "manifest.json":
            data = path.read_bytes()
            entries[relative] = {"sha256": sha256(data), "size": len(data)}
    manifest = {
        "schema": SCHEMA,
        "cases": [
            dict(zip(("name", "transport", "mode", "family", "expected_outcome"), spec, strict=True))
            for spec in CASE_SPECS
        ],
        "files": entries,
        "support": support,
    }
    (root / "manifest.json").write_bytes(canonical_json(manifest))
    return manifest


def unused_tcp_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])

