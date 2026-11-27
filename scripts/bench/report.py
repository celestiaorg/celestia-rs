"""Markdown table of the block-range headline metrics from a collected run.

    python3 scripts/bench/report.py DIR

DIR is the output of `cloud.sh collect`: one directory per VM
(`bench-<campaign>-<run>-p<N>-<rs|go>/summary.json`) and `pairs.txt`.
Only pairs with a `valid` verdict enter the table and the medians. The
median row is the ratio of the two medians, not the median of the ratios.
"""

import json
from pathlib import Path
import re
import statistics
import sys

GIB = 1024 ** 3

# label, value from a summary (None if missing), divisor, decimals
METRICS = (
    ("Time to verified tip (min)", lambda s: num(s.get("tip_t")), 60, 1),
    ("Peak cgroup memory over range (GiB)", lambda s: num(rng(s).get("mem_peak_B")), GIB, 2),
    ("Peak process RSS over range (GiB)", lambda s: kib(num(rng(s).get("rss_peak_KiB"))), GIB, 2),
    ("Node home after shutdown (GiB)", lambda s: num(rng(s).get("home_bytes")), GIB, 1),
)


def num(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def kib(value):
    return None if value is None else value * 1024


def rng(summary):
    return summary.get("range") or {}


def load(root):
    pairs = {}
    for path in sorted(root.glob("*/summary.json")):
        match = re.search(r"-p(\d+)-(rs|go)$", path.parent.name)
        if not match:
            continue
        try:
            summary = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        pairs.setdefault(int(match.group(1)), {})[match.group(2)] = summary
    verdicts = {}
    pairs_file = root/"pairs.txt"
    if pairs_file.exists():
        for line in pairs_file.read_text().splitlines():
            match = re.match(r"pair p(\d+): (.*)$", line)
            if match:
                verdicts[int(match.group(1))] = match.group(2)
    return pairs, verdicts


def fmt(value, divisor, decimals):
    return "n/a" if value is None else f"{value / divisor:.{decimals}f}"


def ratio(rs, go):
    return "n/a" if rs is None or not go else f"{rs / go:.2f}"


def render(root):
    root = Path(root)
    pairs, verdicts = load(root)
    valid = sorted(p for p, v in verdicts.items() if v == "valid" and {"rs", "go"} <= pairs.get(p, {}).keys())
    lines = [f"# Operator comparison: {root.name}", ""]
    any_summary = next((s for side in pairs.values() for s in side.values()), {})
    host = any_summary.get("host") or {}
    window = rng(any_summary)
    if host:
        flags = ", ".join(f for f in ("gfni", "sha_ni", "avx512f", "vaes") if host.get(f)) or "none of gfni/sha_ni"
        lines.append(f"Machine `{host.get('machine_type', '?')}`, {host.get('vcpus', '?')} vCPUs, "
                     f"{fmt(num(host.get('mem_KiB')) and num(host.get('mem_KiB')) * 1024, GIB, 1)} GiB RAM, "
                     f"CPU {host.get('cpu_model', '?')} ({flags}), data disk `{host.get('disk_layout', '?')}`.")
    if window:
        lines.append(f"Window: {window.get('blocks')} blocks from scheduled height {window.get('scheduled_start')}, "
                     f"snapshot {any_summary.get('snapshot', '?')}.")
    lines.append("")
    for p in sorted(set(pairs) | set(verdicts)):
        lines.append(f"- p{p}: {verdicts.get(p, 'no verdict')}")
    lines.append("")
    if not valid:
        lines.append("No valid pair.")
        return "\n".join(lines) + "\n"
    lines += ["| Metric | Pair | Go | celestia-rs | celestia-rs / Go |", "| --- | --- | ---: | ---: | ---: |"]
    for label, get, divisor, decimals in METRICS:
        go_values, rs_values = [], []
        for p in valid:
            go, rs = get(pairs[p]["go"]), get(pairs[p]["rs"])
            lines.append(f"| {label} | p{p} | {fmt(go, divisor, decimals)} | {fmt(rs, divisor, decimals)} | {ratio(rs, go)} |")
            if go is not None and rs is not None:
                go_values.append(go)
                rs_values.append(rs)
        if go_values:
            go_m, rs_m = statistics.median(go_values), statistics.median(rs_values)
            lines.append(f"| {label} | median | {fmt(go_m, divisor, decimals)} | {fmt(rs_m, divisor, decimals)} | {ratio(rs_m, go_m)} |")
    lines.append("")
    lines.append("Range rows cover the sampled window from the first poll at or above H to the first at or above H+N "
                 "(about 5 s sampling). Node home is `du -sB1` after a graceful stop at the halt height.")
    for p in valid:
        r, g = rng(pairs[p]["rs"]), rng(pairs[p]["go"])
        lines.append(f"p{p} final committed height: celestia-rs {r.get('final_app_height')}, Go {g.get('final_app_height')}; "
                     f"range elapsed {r.get('elapsed_s')} s and {g.get('elapsed_s')} s.")
    return "\n".join(lines) + "\n"


def main(argv):
    if len(argv) != 2:
        sys.exit(__doc__)
    sys.stdout.write(render(argv[1]))


if __name__ == "__main__":
    main(sys.argv)
