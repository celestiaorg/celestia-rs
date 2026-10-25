#!/usr/bin/env python3
"""Run one qualification check against immutable, committed source.

This records evidence; a passing check does not certify the other F1 gates.
Example: python3 scripts/quality/qualify.py --out target/qualification/tests \
    -- cargo test --workspace --all-features --locked
"""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys
import time


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def source(root):
    return {
        "revision": git(root, "rev-parse", "HEAD"),
        "tree": git(root, "rev-parse", "HEAD^{tree}"),
        "status": git(root, "status", "--porcelain"),
    }


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


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
    # Never overwrite evidence from an earlier run or turn a rerun into its pass.
    out.mkdir(parents=True, exist_ok=False)
    inputs = {str(path.resolve()): digest(path) for path in binaries}
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
