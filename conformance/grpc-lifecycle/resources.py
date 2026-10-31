#!/usr/bin/env python3
"""Run one recorder and retain a bounded Linux process-resource trace."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


POLL_SECONDS = 0.25
SETTLE_SECONDS = 2.0
MAX_SAMPLES = 2_000


def sample(pid: int, started: float) -> dict[str, int]:
    proc = Path("/proc") / str(pid)
    values = {}
    for line in (proc / "status").read_text().splitlines():
        key, separator, value = line.partition(":")
        if separator and key in {"VmRSS", "Threads"}:
            values[key] = int(value.split()[0])
    assert set(values) == {"VmRSS", "Threads"}
    return {
        "elapsed_ms": round((time.monotonic() - started) * 1_000),
        "rss_kib": values["VmRSS"],
        "threads": values["Threads"],
        "fd_count": len(list((proc / "fd").iterdir())),
    }


def process_identity(pid: int) -> dict[str, int | str]:
    proc = Path("/proc") / str(pid)
    fields = (proc / "stat").read_text().rsplit(") ", 1)[1].split()
    executable = (proc / "exe").read_bytes()
    return {
        "pid": pid,
        "start_ticks": int(fields[19]),
        "executable_sha256": hashlib.sha256(executable).hexdigest(),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node-pid", type=int, required=True)
    parser.add_argument("--implementation", choices=("go", "rs"), required=True)
    parser.add_argument("--listener", choices=("core", "sdk"), required=True)
    parser.add_argument("--quota", type=int, choices=(0, 2))
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    assert command, "recorder command is required"
    assert (args.listener == "core") == (args.quota is not None)
    assert args.node_pid > 1 and Path(f"/proc/{args.node_pid}/status").is_file()
    assert not args.capture.exists() and not args.resources.exists()

    started_wall = datetime.now(timezone.utc)
    started = time.monotonic()
    samples = [sample(args.node_pid, started)]
    initial_identity = process_identity(args.node_pid)
    with tempfile.NamedTemporaryFile(dir=args.capture.parent, delete=False) as output:
        temporary = Path(output.name)
        child = subprocess.Popen(command, stdout=output)
        try:
            while child.poll() is None:
                time.sleep(POLL_SECONDS)
                samples.append(sample(args.node_pid, started))
                assert len(samples) <= MAX_SAMPLES, "resource trace exceeded sample bound"
            if child.returncode != 0:
                raise subprocess.CalledProcessError(child.returncode, command)
            time.sleep(SETTLE_SECONDS)
            final_identity = process_identity(args.node_pid)
            samples.append(sample(args.node_pid, started))
        except BaseException:
            child.terminate()
            child.wait(timeout=10)
            temporary.unlink(missing_ok=True)
            raise
    os.replace(temporary, args.capture)
    value = {
        "schema_version": 1,
        "implementation": args.implementation,
        "listener": args.listener,
        "quota": args.quota,
        "started_at": started_wall.isoformat(),
        "completed_at": datetime.now(timezone.utc).isoformat(),
        "poll_ms": round(POLL_SECONDS * 1_000),
        "settle_ms": round(SETTLE_SECONDS * 1_000),
        "monitor": {
            "python_version": platform.python_version(),
            "system": platform.system(),
            "kernel_release": platform.release(),
            "clock_ticks_per_second": os.sysconf("SC_CLK_TCK"),
            "page_size": os.sysconf("SC_PAGE_SIZE"),
        },
        "process_identity": {
            "initial": initial_identity,
            "final": final_identity,
        },
        "samples": samples,
    }
    temporary = args.resources.with_suffix(args.resources.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    os.replace(temporary, args.resources)


if __name__ == "__main__":
    main()
