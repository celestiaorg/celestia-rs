#!/usr/bin/env python3
"""Retain libFuzzer exploration counters without treating them as coverage %."""

import argparse
import json
from pathlib import Path
import re
import subprocess


def parse(log, exit_code):
    completed = re.findall(r"^#(\d+)\s+DONE\b(.*)$", log, re.MULTILINE)
    stats = dict(re.findall(r"^stat::(\w+):\s+(\d+)\s*$", log, re.MULTILINE))
    result = {
        "exit_code": exit_code,
        "completed": False,
        "executions": None,
        "edges": None,
        "features": None,
        "average_exec_per_sec": None,
        "peak_rss_mb": None,
    }
    if completed:
        executions, detail = completed[-1]
        result["executions"] = int(executions)
        for key, label in [("edges", "cov"), ("features", "ft")]:
            match = re.search(rf"\b{label}: (\d+)", detail)
            if match:
                result[key] = int(match[1])
    for key, label in [
        ("average_exec_per_sec", "average_exec_per_sec"),
        ("peak_rss_mb", "peak_rss_mb"),
    ]:
        if label in stats:
            result[key] = int(stats[label])
    result["completed"] = (
        exit_code == 0
        and bool(result["executions"])
        and result["edges"] is not None
        and result["features"] is not None
        and int(stats.get("number_of_executed_units", "0")) > 0
    )
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--budget-seconds", type=int, required=True)
    args = parser.parse_args()
    log = args.out / "fuzz.log"
    status = args.out / "exit-code"
    result = parse(
        log.read_text(errors="replace") if log.exists() else "",
        int(status.read_text()) if status.exists() else None,
    )
    result.update(
        schema_version=1,
        target=args.target,
        budget_seconds=args.budget_seconds,
        revision=subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True
        ).strip(),
        corpus_files=sum(1 for p in args.corpus.glob("*") if p.is_file()),
    )
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "metrics.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n"
    )
    lines = [
        f"## Fuzz: {args.target}",
        "",
        f"Revision: `{result['revision']}`",
        "",
        "Edges and features are exploration counts for this target/build, not a coverage percentage.",
        "",
        "| Metric | Value |",
        "|---|---:|",
    ]
    lines += [
        f"| {key} | {value if value is not None else 'unknown'} |"
        for key, value in result.items()
        if key not in {"schema_version", "target", "revision"}
    ]
    (args.out / "metrics.md").write_text("\n".join(lines) + "\n")
    return not result["completed"]


if __name__ == "__main__":
    raise SystemExit(main())
