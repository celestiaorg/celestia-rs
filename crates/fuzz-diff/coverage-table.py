#!/usr/bin/env python3
"""Message table of a coverage run (MILESTONES.md F1.3): sums the `TYPE`
and `CODE` lines `deliver` prints over every `<out-dir>/*.out` and writes,
per `sdk.Msg` type URL, how many generated txs Go executed, how many
succeeded, transaction errors by first message, SDK-indexed top-level handler errors,
and messages Go actually
executed in passed proposals (`GOV_EXECUTED`). Nested messages (inside
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
gov_executed = collections.Counter()
codes = collections.defaultdict(collections.Counter)
handler_codes = collections.defaultdict(collections.Counter)
unattributed = collections.Counter()
deliver_outputs = measured_outputs = 0
measured_types = set()
for path in sorted(glob.glob(f"{out_dir}/*.out")):
    measured = has_delivery = has_handler = False
    file_types = set()
    tx_errors = indexed_errors = 0
    with open(path, errors="replace") as f:
        for line in f:
            if line.startswith("ERROR_ACCOUNTING "):
                if measured or line.strip() != "ERROR_ACCOUNTING sdk-message-index-v1":
                    raise ValueError(f"unsupported/duplicate handler-error accounting: {path}")
                measured = has_delivery = True
            elif line.startswith("HANDLER_ERROR "):
                _, n, t, code = line.split()
                if int(n) < 0:
                    raise ValueError(f"negative handler-error count: {path}")
                handler_codes[t][code] += int(n)
                indexed_errors += int(n)
                file_types.add(t)
                has_handler = True
            elif line.startswith("UNATTRIBUTED_ERROR "):
                _, n, code = line.split()
                if int(n) < 0:
                    raise ValueError(f"negative unattributed-error count: {path}")
                unattributed[code] += int(n)
                indexed_errors += int(n)
                has_handler = True
            elif line.startswith("TYPE "):
                has_delivery = True
                parts = line.split()
                kv = dict(p.split("=", 1) for p in parts[2:] if "=" in p)
                t = parts[1]
                file_types.add(t)
                delivered[t] += int(kv.get("delivered", 0))
                ok[t] += int(kv.get("go_ok", 0))
                check_rejected[t] += int(kv.get("check_rejected", 0))
            elif line.startswith("GOV_EXECUTED "):
                _, t, n = line.split()
                gov_executed[t] += int(n)
            elif line.startswith("CODE "):
                has_delivery = True
                parts = line.split(maxsplit=4)
                count = int(parts[1])
                if count < 0:
                    raise ValueError(f"negative transaction-error count: {path}")
                tx_errors += count
                if len(parts) >= 4:
                    codes[parts[2]][parts[3]] += int(parts[1])
    if has_handler and not measured:
        raise ValueError(f"handler-error counters missing their version marker: {path}")
    if measured and tx_errors != indexed_errors:
        raise ValueError(f"incomplete handler-error accounting: {path}: {tx_errors} transaction errors != {indexed_errors} indexed/unattributed errors")
    deliver_outputs += has_delivery
    measured_outputs += measured
    if measured:
        measured_types.update(file_types)

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


types = sorted(set(delivered) | set(codes) | set(handler_codes) | set(rec) | set(gov_executed))
print(f"SDK-indexed handler errors measured in {measured_outputs}/{deliver_outputs} delivery outputs; older outputs are not measured.")
print("Unattributed finalized failures (includes ante/decoding/panics and unknown log forms): " +
      ((", ".join(f"{code}: {n}" for code, n in sorted(unattributed.items())) or "none seen") if measured_outputs else "not measured"))
print()
print(
    "| message | delivered | Go code 0 | CheckTx rejected | Tx errors by first message | SDK-indexed top-level handler errors"
    " | passed proposal executions (generated / recorded) | recorded ok / fail | recorded nested ok / fail |"
)
print("|---|---:|---:|---:|---|---|---:|---|---|")
for t in types:
    errs = ", ".join(f"{c}: {n}" for c, n in sorted(codes[t].items()))
    handler_errs = ((", ".join(f"{c}: {n}" for c, n in sorted(handler_codes[t].items())) or "none seen")
                    if t in measured_types else "not measured")
    print(
        f"| `{t}` | {delivered[t]} | {ok[t]} | {check_rejected[t]} | {errs} | {handler_errs}"
        f" | {gov_executed[t]} / {rec_n(t, 'proposal_executed')} | {rec_n(t, 'recorded_ok')} / {rec_n(t, 'recorded_fail')}"
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
    and gov_executed[t] == 0
    and rec_n(t, "proposal_executed") == 0
    and rec_n(t, "recorded_ok") == 0
    and rec_n(t, "nested_ok") == 0
]
print()
print(f"{len(types)} entries; {len(never_ok)} message types never executed successfully")
print("(neither at top level nor nested):")
for t in never_ok:
    print(f"- `{t}`")
