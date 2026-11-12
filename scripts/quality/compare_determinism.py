#!/usr/bin/env python3
"""Compare retained platform traces only after verifying exact-source receipts."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


CASES = ("blob", "ibc", "hyperlane", "gov", "v9-staking", "upgrade")


def compare(directories):
    sources, traces, receipts, platforms = [], {}, {}, {}
    expected = {case+"-default.trace" for case in CASES}
    if len(directories) < 2:
        raise ValueError("require at least two platform artifacts")
    if len({Path(path).resolve() for path in directories}) != len(directories):
        raise ValueError("platform artifact directories must be distinct")
    for directory in map(Path, directories):
        source_receipts = {}
        for profile in ("release", "dist"):
            path = directory/"qualification"/profile/"evidence.json"
            raw = path.read_bytes()
            evidence = json.loads(raw)
            if (evidence.get("status") != "passed" or evidence.get("exit_code") != 0
                    or evidence.get("inputs_unchanged") is not True
                    or evidence.get("source") != evidence.get("source_after")):
                raise ValueError(f"{directory.name}/{profile}: qualification failed or source/inputs changed")
            source = evidence["source"]
            if source.get("status") != "" or source.get("index_flags") != []:
                raise ValueError(f"{directory.name}/{profile}: source was not clean")
            if not all(isinstance(source.get(key), str) and re.fullmatch(r"[0-9a-f]{40}", source[key])
                       for key in ("revision", "tree")):
                raise ValueError("invalid source revision/tree")
            hashes = evidence.get("binary_sha256")
            if not isinstance(hashes, dict) or len(hashes) != 3 or not all(
                    isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) for value in hashes.values()):
                raise ValueError("require hashes of the Rust harness and both Go oracles")
            platform = (evidence["machine"], evidence["platform"])
            if directory.name in platforms and platforms[directory.name] != platform:
                raise ValueError("profiles report different platforms")
            platforms[directory.name] = platform
            with (path.parent/"check.log").open("rb") as log:
                if hashlib.file_digest(log, "sha256").hexdigest() != evidence.get("log_sha256"):
                    raise ValueError("qualification log digest differs")
            sources.append((source["revision"], source["tree"]))
            source_receipts[profile] = hashlib.sha256(raw).hexdigest()
        receipts[directory.name] = source_receipts
        files = {path.name: path for path in directory.glob("*.trace")}
        if set(files) != expected:
            raise ValueError(f"{directory.name}: missing or unexpected trace files")
        for name, path in files.items():
            data = path.read_bytes()
            if not data:
                raise ValueError(f"{directory.name}/{name}: empty trace")
            if name in traces and data != traces[name]:
                raise ValueError(f"{directory.name}/{name}: trace differs across platforms")
            traces[name] = data
    if len(set(sources)) != 1:
        raise ValueError("platforms/profiles ran different source revisions or trees")
    if len(set(platforms.values())) != len(directories):
        raise ValueError("require distinct qualified platforms")
    return {
        "schema_version": 1, "scope": "exact-source cross-platform deterministic trace comparison",
        "source": {"revision": sources[0][0], "tree": sources[0][1]},
        "platforms": {name: {"machine": value[0], "platform": value[1]} for name, value in platforms.items()},
        "profiles": ["release", "dist"],
        "qualification_receipt_sha256": receipts,
        "traces": {name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
                          "heights": len(data.splitlines())} for name, data in sorted(traces.items())},
        "identical": True,
        "limitations": ["This is a bounded seeded matrix, not complete fuzz/replay or live-node qualification.",
                        "Platform Go oracle executables differ; their separate provenance must be retained."],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", nargs="+")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        result = compare(args.directories)
    except (ValueError, KeyError, OSError, TypeError) as error:
        parser.exit(1, str(error)+"\n")
    report = json.dumps(result, indent=2, sort_keys=True)+"\n"
    if args.out:
        args.out.write_text(report)
    print(report, end="")


if __name__ == "__main__":
    sys.exit(main())
