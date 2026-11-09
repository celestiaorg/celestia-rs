#!/usr/bin/env python3
"""Run one qualification check against immutable, committed source.

This records evidence; a passing check does not certify the other F1 gates.
Example: python3 scripts/quality/qualify.py --out target/qualification/tests \
    -- cargo test --workspace --all-features --locked
"""

import argparse
import binascii
import hashlib
import json
from pathlib import Path
import platform
import struct
import subprocess
import sys
import time


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def source(root):
    # Status honors these index bits even when tracked file contents differ.
    # Preserve NUL-delimited paths (including whitespace/newlines) in evidence.
    index_flags = [entry for entry in git(root, "ls-files", "-v", "-z").split("\0")
                   if entry and (entry[0].islower() or entry[0] == "S")]
    return {
        "revision": git(root, "rev-parse", "HEAD"),
        "tree": git(root, "rev-parse", "HEAD^{tree}"),
        # User Git settings must not hide inputs from qualification evidence.
        "status": git(root, "-c", "core.fsmonitor=false", "status",
                      "--porcelain", "--untracked-files=all",
                      "--ignore-submodules=none"),
        "index_flags": index_flags,
    }


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def protobuf_go_detrand(path):
    """Classify protobuf-go's binary-seeded error prefix for this executable.

    google.golang.org/protobuf/internal/detrand hashes the executable size and
    eight 64-byte samples with FNV-1. Its low bit selects the ordinary or
    nonbreaking space after ``proto:``. The choice can become consensus-visible
    when governance persists a dynamic-message decoding error.
    """
    size = path.stat().st_size
    offset_basis = 14695981039346656037
    prime = 1099511628211
    value = offset_basis
    with path.open("rb") as stream:
        chunks = [struct.pack("<Q", size)]
        for index in range(8):
            stream.seek(index * size // 8)
            sample = stream.read(64)
            # Go returns seed zero if ReadAt cannot fill a complete sample.
            if len(sample) != 64:
                value = 0
                break
            chunks.append(sample)
        else:
            for byte in b"".join(chunks):
                value = value * prime & ((1 << 64) - 1)
                value ^= byte
    return {
        "seed_hex": f"0x{value:016x}",
        "separator": "nonbreaking-space" if value & 1 else "space",
        "separator_utf8_hex": binascii.hexlify("\u00a0".encode() if value & 1 else b" ").decode(),
    }


def tool(*command):
    try:
        result = subprocess.run(command, text=True, capture_output=True, check=False)
        return {"exit_code": result.returncode, "output": result.stdout.strip(),
                "error": result.stderr.strip()}
    except OSError as error:
        return {"exit_code": None, "error": str(error)}


def qualify(root, out, command, binaries):
    before = source(root)
    if before["status"]:
        raise ValueError("qualification requires a clean worktree, including untracked files")
    if before["index_flags"]:
        raise ValueError("qualification requires tracked files without assume-unchanged "
                         "or skip-worktree index flags")
    # Never overwrite evidence from an earlier run or turn a rerun into its pass.
    out.mkdir(parents=True, exist_ok=False)
    inputs = {str(path.resolve()): digest(path) for path in binaries}
    protobuf_prefixes = {
        str(path.resolve()): protobuf_go_detrand(path) for path in binaries
    }
    evidence = {
        "schema_version": 1,
        "source": before,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "command": command,
        "started_at_unix": time.time(),
        "toolchains": {"rustc": tool("rustc", "-Vv"), "cargo": tool("cargo", "-V"),
                       "go": tool("go", "version")},
        "binary_sha256": inputs,
        "protobuf_go_detrand": protobuf_prefixes,
        "status": "running",
        "scope": "one check only; remaining F1 gates require separate evidence",
    }
    manifest = out / "evidence.json"

    def save():
        manifest.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")

    save()
    try:
        with (out / "check.log").open("wb") as log:
            result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT,
                                    check=False)
        evidence["exit_code"] = result.returncode
        after = source(root)
        evidence["source_after"] = after
        evidence["inputs_unchanged"] = all(
            Path(path).is_file() and digest(Path(path)) == value
            for path, value in inputs.items()
        )
        evidence["status"] = "passed" if (
            result.returncode == 0 and after == before and evidence["inputs_unchanged"]
        ) else "failed"
        evidence["log_sha256"] = digest(out / "check.log")
    except OSError as error:
        evidence["status"] = "failed"
        evidence["error"] = str(error)
    finally:
        evidence["finished_at_unix"] = time.time()
        save()
    return evidence["status"] == "passed"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path, action="append", default=[],
                        help="hash an already-built input binary before and after the check")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("supply a command after --")
    root = Path(__file__).resolve().parents[2]
    try:
        passed = qualify(root, args.out.resolve(), command,
                         [path.resolve() for path in args.binary])
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(2, f"qualification: {error}\n")
    print(f"{'passed' if passed else 'failed'}: {args.out / 'evidence.json'}")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
