#!/usr/bin/env python3
"""Paired real-transport driver for the live transaction closure campaign.

The driver owns the simultaneous release barrier and never accepts commands
from the scenario. It invokes one receipt-bound transport client with a fixed
CLI contract for each side and preserves the client's complete structured raw
exchange. Node lifecycle and signed input generation remain in the outer
campaign script; this program only executes already frozen requests.
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import threading
import time
from typing import Any

import harness


CLIENT_FIELDS = {
    "transport", "wire", "http_status", "http_headers", "json", "grpc_status",
    "grpc_message", "initial_metadata", "trailing_metadata", "websocket_frames",
    "websocket_handshake", "client_error", "process_effect",
}


def load(path: pathlib.Path) -> Any:
    return harness.strict_json(path.read_bytes())


def checked_client(path: pathlib.Path, expected_sha256: str) -> pathlib.Path:
    harness.reject_symlink_components(path, "transport client")
    resolved = path.resolve(strict=True)
    harness.require(resolved.is_file() and not resolved.is_symlink(), "transport client is not a regular file")
    harness.require(hashlib.sha256(resolved.read_bytes()).hexdigest() == expected_sha256,
                    "transport client binary hash differs")
    return resolved


def invoke(client: pathlib.Path, case_name: str, endpoint: str, transport: str, mode: str, request: bytes,
           timeout_ms: int, disconnect: bool, ready: threading.Barrier,
           release: threading.Event) -> tuple[int, dict[str, Any]]:
    harness.loopback_endpoint(endpoint, "transport endpoint")
    command = [
        str(client), "--mode", "exact-broadcast", "--endpoint", endpoint,
        "--case-name", case_name, "--transport", transport, "--broadcast-mode", mode,
        "--request-base64", base64.b64encode(request).decode(),
        "--jsonrpc-id", "187", "--timeout-ms", str(timeout_ms),
    ]
    if disconnect:
        command.append("--disconnect-after-write")
    ready.wait(timeout=10)
    harness.require(release.wait(timeout=10), "paired release barrier timed out")
    invoked_ns = time.monotonic_ns()
    completed = subprocess.run(command, capture_output=True, timeout=max(10, timeout_ms / 1000 + 10), check=False)
    harness.require(completed.returncode in {0, 3}, f"transport client exited {completed.returncode}: {completed.stderr[:4096]!r}")
    harness.require(len(completed.stdout) <= 16 * 1024 * 1024, "transport client output oversized")
    result = harness.strict_json(completed.stdout)
    harness.require_fields(result, CLIENT_FIELDS, "transport client result")
    harness.require(result["transport"] == transport, "transport client relabeled transport")
    return invoked_ns, result


def paired(spec: dict[str, Any], client: pathlib.Path) -> dict[str, Any]:
    fields = {"name", "transport", "mode", "request", "go_endpoint", "rs_endpoint", "timeout_ms", "disconnect_after_write"}
    harness.require_fields(spec, fields, "paired request spec")
    harness.require(spec["name"] in harness.CASE_NAMES, "paired request case is not frozen")
    frozen = next(row for row in harness.CASE_SPECS if row[0] == spec["name"])
    harness.require((spec["transport"], spec["mode"]) == (frozen[1], frozen[2]),
                    "paired request transport/mode differs from frozen case")
    harness.require(isinstance(spec["timeout_ms"], int) and 0 < spec["timeout_ms"] <= 60_000,
                    "paired request timeout is outside the frozen bound")
    harness.require(spec["disconnect_after_write"] is (spec["mode"] == "commit-disconnect"),
                    "paired request disconnect policy differs from frozen case")
    request = harness.decode_blob(spec["request"], f"{spec['name']} request")
    if spec["name"] == "rest-invalid-base64":
        harness.assert_invalid_rest_base64_request(request)
    elif spec["name"] == "cosmos-grpc-no-message":
        harness.assert_txraw_no_messages(request)
    elif spec["name"] == "raw-malformed-protobuf":
        harness.assert_malformed_protobuf(request)
    ready = threading.Barrier(3)
    release = threading.Event()
    with concurrent.futures.ThreadPoolExecutor(max_workers=2, thread_name_prefix="paired-broadcast") as pool:
        futures = {
            side: pool.submit(
                invoke, client, spec["name"], spec[f"{side}_endpoint"], spec["transport"], spec["mode"], request,
                spec["timeout_ms"], spec["disconnect_after_write"], ready, release,
            )
            for side in ("go", "rs")
        }
        ready.wait(timeout=10)
        release_ns = time.monotonic_ns()
        release.set()
        invoked_and_results = {side: future.result() for side, future in futures.items()}
    completed_ns = time.monotonic_ns()
    return {
        "name": spec["name"], "barrier_release_monotonic_ns": release_ns,
        "pair_completed_monotonic_ns": completed_ns,
        "timeout_ms": spec["timeout_ms"], "disconnect_after_write": spec["disconnect_after_write"],
        "go_invoked_monotonic_ns": invoked_and_results["go"][0],
        "rs_invoked_monotonic_ns": invoked_and_results["rs"][0],
        "request_sha256": hashlib.sha256(request).hexdigest(),
        "transport_client_sha256": hashlib.sha256(client.read_bytes()).hexdigest(),
        "driver_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "go_result_sha256": hashlib.sha256(harness.canonical_json(invoked_and_results["go"][1])).hexdigest(),
        "rs_result_sha256": hashlib.sha256(harness.canonical_json(invoked_and_results["rs"][1])).hexdigest(),
        "go": invoked_and_results["go"][1], "rs": invoked_and_results["rs"][1],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", type=pathlib.Path, required=True)
    parser.add_argument("--client", type=pathlib.Path, required=True)
    parser.add_argument("--client-sha256", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        harness.reject_symlink_components(args.output.parent, "paired driver output parent")
        harness.require(not args.output.exists(), "paired driver output already exists")
        client = checked_client(args.client, args.client_sha256)
        result = paired(load(args.spec), client)
        descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(harness.canonical_json(result))
    except (OSError, subprocess.SubprocessError, threading.BrokenBarrierError, ValueError, harness.HarnessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
