#!/usr/bin/env python3
"""Capture controller for the paired live-transaction closure campaign.

The transport driver remains a separate reviewed executable. This controller
owns output-directory creation, process-group isolation, the disk watchdog and
manifest finalization so the runtime driver cannot silently weaken those gates.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import signal
import subprocess
import sys
import time
from typing import Any

import harness


EXPECTED_PAYLOADS = {
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


def process_group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return False
    return True


def drain_process_group(process_group: int) -> dict[str, bool]:
    """Boundedly drain descendants left by the isolated capture driver."""
    term_sent = process_group_exists(process_group)
    if term_sent:
        os.killpg(process_group, signal.SIGTERM)
    deadline = time.monotonic() + 5
    while process_group_exists(process_group) and time.monotonic() < deadline:
        time.sleep(0.05)
    kill_sent = process_group_exists(process_group)
    if kill_sent:
        os.killpg(process_group, signal.SIGKILL)
    deadline = time.monotonic() + 5
    while process_group_exists(process_group) and time.monotonic() < deadline:
        time.sleep(0.05)
    harness.require(not process_group_exists(process_group), "capture process group did not drain")
    return {"process_group_drained": True, "process_group_term_sent": term_sent, "process_group_kill_sent": kill_sent}


def load(path: pathlib.Path) -> Any:
    return harness.strict_json(path.read_bytes())


def run_driver(output: pathlib.Path, estimate: int, command: list[str]) -> int:
    harness.require(command, "capture driver command is empty")
    campaign = pathlib.Path(__file__).with_name("campaign.py").resolve(strict=True)
    campaign_sha = hashlib.sha256(campaign.read_bytes()).hexdigest()
    harness.require(len(command) == 6 and pathlib.Path(command[0]).resolve(strict=True) == pathlib.Path(sys.executable).resolve(strict=True) and
                    pathlib.Path(command[1]).resolve(strict=True) == campaign and command[2] == "--plan" and
                    command[4:] == ["--source-sha256", campaign_sha],
                    "capture controller accepts only the receipt-bound outer campaign")
    harness.preflight_output(output, estimate)
    parent_stat = os.statvfs(output.parent.resolve(strict=True))
    preflight_free = parent_stat.f_bavail * parent_stat.f_frsize
    output.mkdir(mode=0o700)
    controller_started = int(time.monotonic() * 1000)
    process = subprocess.Popen(command, start_new_session=True, env={**os.environ, "LIVE_TX_CAPTURE_ROOT": str(output)})
    watchdog = harness.DiskFloorWatchdog(output, process.pid)
    watchdog.start()
    returncode = process.wait()
    group = drain_process_group(process.pid)
    disk_samples = watchdog.stop()
    controller_completed = int(time.monotonic() * 1000)
    sample_path = output / "capture-controller.partial.json"
    sample_path.write_bytes(harness.canonical_json({
        "disk_samples": disk_samples,
        "preflight_estimate_bytes": estimate,
        "preflight_free_bytes": preflight_free,
        "controller_started_monotonic_ms": controller_started,
        "controller_completed_monotonic_ms": controller_completed,
        **group,
    }))
    if returncode != 0:
        raise harness.HarnessError(f"capture driver exited {returncode}; output is diagnostic only")
    return returncode


def finalize(root: pathlib.Path, support_path: pathlib.Path) -> None:
    harness.reject_symlink_components(root, "capture root")
    root = root.resolve(strict=True)
    partial = root / "capture-controller.partial.json"
    harness.require(partial.is_file() and not partial.is_symlink(), "capture controller record is missing")
    controller = load(partial)
    harness.require_fields(controller, {"disk_samples", "preflight_estimate_bytes", "preflight_free_bytes", "controller_started_monotonic_ms", "controller_completed_monotonic_ms", "process_group_drained", "process_group_term_sent", "process_group_kill_sent"}, "capture controller record")
    resources_path = root / "resources.json"
    resources = load(resources_path)
    harness.require_fields(resources, {"processes", "samples", "generation_intervals"}, "driver resources")
    resources.update(controller)
    resources_path.write_bytes(harness.canonical_json(resources))
    partial.unlink()
    actual = {path.relative_to(root).as_posix() for path in root.iterdir() if path.is_file()}
    harness.require(actual == EXPECTED_PAYLOADS, f"capture payload inventory differs: {sorted(actual ^ EXPECTED_PAYLOADS)}")
    support = load(support_path)
    harness.require_fields(
        support,
        {"capture_started", "capture_completed", "candidate_commit", "normalizations", "intentional_differences", "exclusions"},
        "manifest support",
    )
    harness.finalize_manifest(root, support)


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run")
    run.add_argument("--output", type=pathlib.Path, required=True)
    run.add_argument("--estimate-bytes", type=int, required=True)
    run.add_argument("command", nargs=argparse.REMAINDER)
    done = sub.add_parser("finalize")
    done.add_argument("--root", type=pathlib.Path, required=True)
    done.add_argument("--support", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "run":
            command = args.command[1:] if args.command[:1] == ["--"] else args.command
            run_driver(args.output, args.estimate_bytes, command)
        else:
            finalize(args.root, args.support)
    except (OSError, subprocess.SubprocessError, harness.HarnessError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
