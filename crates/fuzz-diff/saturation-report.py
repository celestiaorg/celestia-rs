#!/usr/bin/env python3
"""Summarize retained nightly artifacts for F1.2, grouped by source revision.

Usage: python3 crates/fuzz-diff/saturation-report.py <findings-dir> [...]
Only successful, clean-worktree runs with zero consensus mismatches count.
Identical scenario / seed / size / flags at one revision count once, even
when artifacts are downloaded twice. This is volume evidence, not proof
of the separate 3-night or scenario-coverage gates.
"""

import collections
import json
from pathlib import Path
import re
import sys

PINNED_GO = next(
    line.split()[1]
    for line in (Path(__file__).resolve().parents[2] / "conformance/devnet-oracle/go.mod")
    .read_text().splitlines()
    if line.startswith("toolchain ")
)


def collect(roots):
    runs = {}
    excluded = collections.Counter()
    for root in roots:
        for err in sorted(Path(root).rglob("*.err")):
            lines = [s for s in err.read_text(errors="replace").splitlines()
                     if s.startswith("DELIVER_STATS ")]
            if not lines:
                continue  # exec / pp / decode harness, or an incomplete run
            try:
                if len(lines) != 1:
                    raise ValueError("multiple summaries")
                st = json.loads(lines[0].removeprefix("DELIVER_STATS "))
                revision = (err.parent / "revision.txt").read_text().strip()
                status = (err.parent / "worktree.status").read_text().strip()
                patch = (err.parent / "worktree.patch").read_text().strip()
                build_info = (err.parent / "go-build-info.txt").read_text()
                toolchains = re.findall(r": (go\d+\.\d+\.\d+)\s*$", build_info, re.MULTILINE)
                if toolchains != [PINNED_GO, PINNED_GO]:
                    excluded["unpinned Go toolchain"] += 1
                    continue
                rc = int(err.with_suffix(".exit").read_text())
                flags = tuple(sorted(err.with_suffix(".env").read_text().splitlines()))
                if not revision or status or patch:
                    excluded["uncommitted source"] += 1
                    continue
                if rc != 0 or st["mismatches"] != 0 or st["split"]:
                    excluded["failed or mismatched"] += 1
                    continue
                if st["blocks"] != st["requested_blocks"]:
                    excluded["incomplete block count"] += 1
                    continue
                for key in ("seed", "blocks", "requested_blocks", "txs_per_block",
                            "delivered", "delivered_ok", "mismatches"):
                    if type(st[key]) is not int or st[key] < 0:
                        raise ValueError("invalid counter")
                if st["delivered_ok"] > st["delivered"]:
                    raise ValueError("invalid success count")
                # Binary paths differ between runners. Their pinned app
                # versions are fixed by nightly.sh; the scenario names
                # distinguish v9, v10 and the upgrade.
                flags = tuple(f for f in flags if not f.startswith(("GO_APPD=", "GO_APPD_NEXT=")))
                identity = (revision, st["scenario"], st["seed"], st["requested_blocks"],
                            st["txs_per_block"], flags)
                if identity in runs:
                    if runs[identity] != st:
                        raise ValueError(f"conflicting duplicate run: {err}")
                    excluded["duplicate"] += 1
                else:
                    runs[identity] = st
            except (OSError, KeyError, TypeError, json.JSONDecodeError) as exc:
                raise ValueError(f"invalid evidence {err}: {exc}") from exc
    return runs, excluded


def report(roots):
    runs, excluded = collect(roots)
    totals = collections.defaultdict(lambda: [0, 0, 0, 0])
    for identity, st in runs.items():
        row = totals[(identity[0], st["scenario"])]
        for i, n in enumerate((1, st["blocks"], st["delivered"], st["delivered_ok"])):
            row[i] += n
    print("| revision | scenario | unique runs | generated blocks | finalized txs | Go code 0 |")
    print("|---|---|---:|---:|---:|---:|")
    for (revision, scenario), row in sorted(totals.items()):
        print(f"| `{revision}` | {scenario} | " + " | ".join(map(str, row)) + " |")
    print(f"\nCounted {len(runs)} unique clean runs; "
          f"{sum(st['delivered'] for st in runs.values())} finalized txs across all revisions.")
    print("Assess the F1.2 volume gate per revision, after the last fix; "
          "this report does not establish 14 clean nights or complete scenario coverage.")
    print(f"Both Go oracles must have been built with {PINNED_GO}.")
    if excluded:
        print("Excluded: " + ", ".join(f"{n} {why}" for why, n in sorted(excluded.items())) + ".")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    report(sys.argv[1:])
