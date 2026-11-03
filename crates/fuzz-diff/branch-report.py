#!/usr/bin/env python3
"""Retain measured LLVM branch arms and locations still needing F1.3 review."""

import argparse
import json
from pathlib import Path


def collect(raw, root):
    files = {}
    root = root.resolve()
    for unit in raw["data"]:
        for item in unit["files"]:
            path = Path(item["filename"])
            path = path if path.is_absolute() else root / path
            try:
                relative = path.resolve().relative_to(root).as_posix()
            except ValueError:
                continue
            if (not relative.startswith("crates/tier-") or "/src/" not in relative
                    or "/src/gen/" in relative or "/celestia-proto/" in relative
                    or "/celestia-core-proto/" in relative
                    or path.name in {"tests.rs", "testing.rs"}
                    or path.name.endswith("_tests.rs")):
                continue
            if relative in files:
                raise ValueError(f"duplicate coverage file: {relative}")
            arms, covered, uncovered = 0, 0, []
            for index, region in enumerate(item.get("branches", [])):
                if (len(region) < 6 or any(type(n) is not int or n < 0 for n in region[:6])
                        or any(n == 0 for n in region[:4])):
                    raise ValueError(f"invalid branch region: {relative}")
                counts = region[4:6]
                arms += 2
                covered += sum(n > 0 for n in counts)
                missing = [name for name, n in zip(("true", "false"), counts) if n == 0]
                if missing:
                    uncovered.append({"start": region[:2], "end": region[2:4],
                                      "true_count": counts[0], "false_count": counts[1],
                                      "uncovered_arms": missing, "review": "pending",
                                      "region_index": index, "llvm_region": region})
            summary = item.get("summary", {}).get("branches", {})
            total, hit = summary.get("count"), summary.get("covered")
            if (type(total) is not int or type(hit) is not int
                    or not 0 <= hit <= total or (total and not arms)):
                raise ValueError(f"invalid branch summary: {relative}")
            if "notcovered" in summary and (type(summary["notcovered"]) is not int
                                             or summary["notcovered"] != total - hit):
                raise ValueError(f"invalid uncovered summary: {relative}")
            # LLVM can export regions of uninstantiated functions and multiple
            # instantiations which its summary does not count the same way.
            files[relative] = {"count": total, "covered": hit,
                               "exported_region_arms": {"count": arms, "covered": covered},
                               "uncovered": uncovered}
    count = sum(f["count"] for f in files.values())
    if not count:
        raise ValueError("no measured handwritten branch arms; branch coverage is pending")
    return {"schema_version": 1, "count": count,
            "covered": sum(f["covered"] for f in files.values()), "files": files,
            "scope": "LLVM branch arms in handwritten tier source; inline tests included",
            "inventory_scope": "exported regions include instantiations/uncalled functions; their denominator differs from LLVM summaries",
            "qualification": "pending: uncovered arms require review and reasons; other F1 gates remain"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    report = collect(json.loads(args.input.read_text()), args.root)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"LLVM branch arms: {report['covered']}/{report['count']}; uncovered-arm review pending")


if __name__ == "__main__":
    main()
