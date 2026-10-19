#!/usr/bin/env python3
"""Message table of a coverage run (MILESTONES.md F1.3): sums the `TYPE`
and `CODE` lines `deliver` prints over every `<out-dir>/*.out` and writes,
per `sdk.Msg` type URL, how many generated txs Go executed, how many
succeeded, and each error (codespace/code) seen. Nested messages (inside
`MsgExec` or a proposal) are prefixed `>`.

With the recorded-scenario table (`fuzz-diff coverage > rec.tsv`) as the
second argument, its code-0 / other counts of the replayed scenario txs
(top level and nested) are added as columns and count as executions.

usage: crates/fuzz-diff/coverage-table.py <out-dir> [rec.tsv] > table.md
"""
import collections
import glob
import sys

out_dir = sys.argv[1]
delivered = collections.Counter()
ok = collections.Counter()
check_rejected = collections.Counter()
codes = collections.defaultdict(collections.Counter)
for path in sorted(glob.glob(f"{out_dir}/*.out")):
    with open(path, errors="replace") as f:
        for line in f:
            if line.startswith("TYPE "):
                parts = line.split()
                kv = dict(p.split("=", 1) for p in parts[2:] if "=" in p)
                t = parts[1]
                delivered[t] += int(kv.get("delivered", 0))
                ok[t] += int(kv.get("go_ok", 0))
                check_rejected[t] += int(kv.get("check_rejected", 0))
            elif line.startswith("CODE "):
                parts = line.split(maxsplit=4)
                if len(parts) >= 4:
                    codes[parts[2]][parts[3]] += int(parts[1])

rec = {}
if len(sys.argv) > 2:
    with open(sys.argv[2]) as f:
        header = f.readline().rstrip("\n").split("\t")
        for line in f:
            row = dict(zip(header, line.rstrip("\n").split("\t")))
            if "type" in row:
                rec[row["type"]] = row


def rec_n(t, col):
    return int(rec.get(t, {}).get(col, 0) or 0)


types = sorted(set(delivered) | set(codes) | set(rec))
print(
    "| message | delivered | Go code 0 | CheckTx rejected | Go errors (codespace/code: txs)"
    " | recorded ok / fail | recorded nested ok / fail |"
)
print("|---|---:|---:|---:|---|---|---|")
for t in types:
    errs = ", ".join(f"{c}: {n}" for c, n in sorted(codes[t].items()))
    print(
        f"| `{t}` | {delivered[t]} | {ok[t]} | {check_rejected[t]} | {errs}"
        f" | {rec_n(t, 'recorded_ok')} / {rec_n(t, 'recorded_fail')}"
        f" | {rec_n(t, 'nested_ok')} / {rec_n(t, 'nested_fail')} |"
    )
# Authority messages succeed only inside a passed proposal (`>` entries).
never_ok = [
    t
    for t in types
    if not t.startswith(">")
    and t != "<undecodable>"
    and ok[t] == 0
    and ok[">" + t] == 0
    and rec_n(t, "recorded_ok") == 0
    and rec_n(t, "nested_ok") == 0
]
print()
print(f"{len(types)} entries; {len(never_ok)} message types never executed successfully")
print("(neither at top level nor nested):")
for t in never_ok:
    print(f"- `{t}`")
