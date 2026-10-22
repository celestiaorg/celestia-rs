#!/usr/bin/env python3
"""Turn cargo-llvm-cov JSON into comparable, first-party coverage evidence."""

import argparse
import json
from pathlib import Path
import subprocess
import tomllib

METRICS = ("lines", "regions", "functions")
# Generated protobuf code has a very different denominator from handwritten code.
GENERATED = {"celestia-proto", "celestia-core-proto"}


def empty_counts():
    return {metric: {"count": 0, "covered": 0} for metric in METRICS}


def collect(raw, root):
    root = root.resolve()
    workspace = tomllib.loads((root / "Cargo.toml").read_text())
    crates = {}
    for member in workspace["workspace"]["members"]:
        if member.startswith("crates/tier-") and Path(member).name not in GENERATED:
            crates[member] = {"files": 0, **empty_counts()}
    files = {}
    for unit in raw["data"]:
        for item in unit["files"]:
            path = Path(item["filename"])
            if not path.is_absolute():
                path = root / path
            try:
                relative = path.resolve().relative_to(root).as_posix()
            except ValueError:
                continue
            member = next((m for m in crates if relative.startswith(m + "/src/")), None)
            if member is None:
                continue
            parts = Path(relative).parts
            if (
                "gen" in parts
                or "tests" in parts
                or Path(relative).name in {"tests.rs", "testing.rs"}
                or relative.endswith("_tests.rs")
            ):
                continue
            if relative in files:
                raise ValueError(f"duplicate coverage file: {relative}")
            counts = empty_counts()
            for metric in METRICS:
                source = item["summary"][metric]
                count, covered = source["count"], source["covered"]
                if (
                    type(count) is not int
                    or type(covered) is not int
                    or not 0 <= covered <= count
                ):
                    raise ValueError(f"invalid {metric} counters: {relative}")
                counts[metric] = {"count": count, "covered": covered}
                crates[member][metric]["count"] += count
                crates[member][metric]["covered"] += covered
            files[relative] = counts
            crates[member]["files"] += 1
    if not files or not any(c["lines"]["count"] for c in files.values()):
        raise ValueError("no handwritten production coverage found")
    return crates, files


def check(files, policy):
    failures = []
    for path, budget in policy.items():
        lines = files.get(path, {}).get("lines", {})
        if not lines.get("count"):
            failures.append(f"{path}: missing coverage")
        elif lines["count"] - lines["covered"] > budget["max_uncovered_lines"]:
            failures.append(
                f"{path}: {lines['count'] - lines['covered']} uncovered lines exceeds budget {budget['max_uncovered_lines']}"
            )
    return failures


def cell(counts):
    total, covered = counts["count"], counts["covered"]
    return (
        f"{covered}/{total} ({100 * covered / total:.2f}%)" if total else "not measured"
    )


def render(report):
    lines = [
        "## Test coverage",
        "",
        f"Revision: `{report['revision']}`",
        "",
        f"Suite: `{report['suite']}`; modified worktree: `{report['worktree_dirty']}`.",
        "",
        "Handwritten tier 0–5 source; generated code and separate test files excluded.",
        "Inline test code is included by LLVM. Branch coverage is **not measured**.",
        "",
        "| Crate | Lines | Uncovered lines | Regions | Functions |",
        "|---|---:|---:|---:|---:|",
    ]
    for name, counts in sorted(report["crates"].items()):
        uncovered = (
            counts["lines"]["count"] - counts["lines"]["covered"]
            if counts["lines"]["count"]
            else "unknown"
        )
        lines.append(
            f"| {name.removeprefix('crates/')} | {cell(counts['lines'])} | {uncovered} | {cell(counts['regions'])} | {cell(counts['functions'])} |"
        )
    lines += [
        "",
        "### Largest uncovered files",
        "",
        "| File | Uncovered lines | Line coverage |",
        "|---|---:|---:|",
    ]
    ranked = sorted(
        report["files"].items(),
        key=lambda pair: (
            -(pair[1]["lines"]["count"] - pair[1]["lines"]["covered"]),
            pair[0],
        ),
    )
    for name, counts in ranked[:30]:
        lines.append(
            f"| {name} | {counts['lines']['count'] - counts['lines']['covered']} | {cell(counts['lines'])} |"
        )
    lines += ["", "### Coverage budgets", ""]
    lines += [f"- {failure}" for failure in report["failures"]] or [
        "All configured budgets passed."
    ]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument(
        "--root", type=Path, default=Path(__file__).resolve().parents[2]
    )
    parser.add_argument("--policy", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--require-all", action="store_true")
    parser.add_argument(
        "--suite", required=True, help="Exact cargo-llvm-cov test selection"
    )
    args = parser.parse_args()
    crates, files = collect(json.loads(args.input.read_text()), args.root)
    policy = json.loads(args.policy.read_text())
    failures = check(files, policy)
    if args.require_all:
        failures += [
            f"{name}: missing coverage"
            for name, counts in crates.items()
            if not counts["lines"]["count"]
        ]
    report = {
        "schema_version": 1,
        "revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=args.root, text=True
        ).strip(),
        "worktree_dirty": bool(
            subprocess.check_output(
                ["git", "status", "--porcelain"], cwd=args.root, text=True
            ).strip()
        ),
        "suite": args.suite,
        "crates": crates,
        "files": files,
        "budgets": policy,
        "failures": failures,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "coverage.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n"
    )
    (args.out / "coverage.md").write_text(render(report))
    for failure in failures:
        print(failure)
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
