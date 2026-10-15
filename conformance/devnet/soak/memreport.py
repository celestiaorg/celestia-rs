#!/usr/bin/env python3
"""Summarizes a memory soak (`SOAK_PROFILE=memory soak.sh`): reads
`mem.jsonl` and prints a Markdown table per node (RSS / heap / threads /
fds / disk at the start of steady state and at the end, peak, least-squares
growth over the steady state and over its last half) and optionally writes
an SVG chart of RSS and heap over time.

usage: memreport.py <soak-dir> [--warmup-min 30] [--svg out.svg]
"""

import argparse
import json
from collections import defaultdict


def slope(pts):
    n = len(pts)
    if n < 2:
        return 0.0
    mx = sum(p[0] for p in pts) / n
    my = sum(p[1] for p in pts) / n
    den = sum((p[0] - mx) ** 2 for p in pts)
    return sum((p[0] - mx) * (p[1] - my) for p in pts) / den if den else 0.0


def heap_mb(r):
    """Live heap: jemalloc `allocated` (Rust memstats build) or Go's heap in use."""
    if "heap_allocated_bytes" in r:
        return r["heap_allocated_bytes"] / 2**20
    if r.get("kind") == "go" and "heap_inuse_bytes" in r:  # go_memstats_heap_inuse_bytes
        return r["heap_inuse_bytes"] / 2**20
    return None


def main():
    p = argparse.ArgumentParser()
    p.add_argument("dir")
    p.add_argument("--warmup-min", type=float, default=30)
    p.add_argument("--svg")
    a = p.parse_args()
    rows = defaultdict(list)
    for line in open(f"{a.dir}/mem.jsonl"):
        r = json.loads(line)
        if r.get("rss_mb"):
            rows[r["node"]].append(r)
    out = ["| node | kind | hours | RSS start → end MB | RSS peak MB | RSS growth MB/h (all / last half) | "
           "heap start → end MB | heap growth MB/h (all / last half) | threads | fds | disk MB start → end |",
           "|---|---|---|---|---|---|---|---|---|---|---|"]
    series = {}
    for n in sorted(rows):
        rs = rows[n]
        lives = {r["life"] for r in rs}
        steady = [r for r in rs if r["t"] >= a.warmup_min * 60] or rs
        half = steady[len(steady) // 2:]
        hrs = (steady[-1]["t"] - steady[0]["t"]) / 3600
        rss = [(r["t"] / 3600, r["rss_mb"]) for r in steady]
        heap = [(r["t"] / 3600, heap_mb(r)) for r in steady if heap_mb(r) is not None]
        hhalf = [(r["t"] / 3600, heap_mb(r)) for r in half if heap_mb(r) is not None]
        th = [r.get("threads", 0) for r in steady]
        fd = [r.get("fds", 0) for r in steady]
        kind = rs[0]["kind"] + (" (jemalloc)" if any("heap_allocated_bytes" in r for r in rs) else "")
        heap_s = f"{heap[0][1]:.0f} → {heap[-1][1]:.0f}" if heap else "-"
        heap_g = f"{slope(heap):+.1f} / {slope(hhalf):+.1f}" if heap else "-"
        out.append(
            f"| n{n} | {kind}{'' if len(lives) == 1 else f' ({len(lives)} lifetimes!)'} | {hrs:.1f} | "
            f"{rss[0][1]:.0f} → {rss[-1][1]:.0f} | {max(r['rss_mb'] for r in rs):.0f} | "
            f"{slope(rss):+.1f} / {slope([(r['t'] / 3600, r['rss_mb']) for r in half]):+.1f} | {heap_s} | {heap_g} | "
            f"{min(th)}-{max(th)} | {min(fd)}-{max(fd)} | {steady[0]['disk_mb']:.0f} → {steady[-1]['disk_mb']:.0f} |")
        series[n] = (kind, [(r["t"] / 3600, r["rss_mb"]) for r in rs],
                     [(r["t"] / 3600, heap_mb(r)) for r in rs if heap_mb(r) is not None])
    print("\n".join(out))
    # Rust structure gauges at the end (bounded caches).
    for n in sorted(rows):
        last = rows[n][-1]
        g = {k: v for k, v in last.items() if k.startswith(("kvdb_cache_bytes", "iavl_", "heap_", "mempool"))}
        if g:
            print(f"\nn{n} at the end: " + ", ".join(
                f"{k}={v / 2**20:.0f}MiB" if "bytes" in k else f"{k}={v:.0f}" for k, v in sorted(g.items())))
    if a.svg:
        write_svg(a.svg, series)


def write_svg(path, series):
    w, h, pad = 900, 420, 50
    tmax = max(p[0] for _, s, _ in series.values() for p in s)
    ymax = max(p[1] for _, s, _ in series.values() for p in s) * 1.05
    colors = ["#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2"]
    sx = lambda t: pad + (w - 2 * pad) * t / tmax  # noqa: E731
    sy = lambda v: h - pad - (h - 2 * pad) * v / ymax  # noqa: E731
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" font-family="sans-serif" font-size="12">',
             f'<rect width="{w}" height="{h}" fill="white"/>',
             f'<line x1="{pad}" y1="{h - pad}" x2="{w - pad}" y2="{h - pad}" stroke="#444"/>',
             f'<line x1="{pad}" y1="{pad}" x2="{pad}" y2="{h - pad}" stroke="#444"/>',
             f'<text x="{w / 2}" y="{h - 12}" text-anchor="middle">hours</text>',
             f'<text x="14" y="{h / 2}" transform="rotate(-90 14 {h / 2})" text-anchor="middle">MB</text>']
    step = max(1, round(ymax / 5 / 250) * 250)
    for v in range(0, int(ymax) + 1, step):
        parts.append(f'<line x1="{pad}" y1="{sy(v):.1f}" x2="{w - pad}" y2="{sy(v):.1f}" stroke="#eee"/>')
        parts.append(f'<text x="{pad - 4}" y="{sy(v) + 4:.1f}" text-anchor="end">{v}</text>')
    for t in range(0, int(tmax) + 1):
        parts.append(f'<text x="{sx(t):.1f}" y="{h - pad + 16}" text-anchor="middle">{t}</text>')
    for k, (n, (kind, rss, heap)) in enumerate(sorted(series.items())):
        c = colors[k % len(colors)]
        pts = " ".join(f"{sx(t):.1f},{sy(v):.1f}" for t, v in rss)
        parts.append(f'<polyline fill="none" stroke="{c}" stroke-width="1.5" points="{pts}"/>')
        if heap:
            pts = " ".join(f"{sx(t):.1f},{sy(v):.1f}" for t, v in heap)
            parts.append(f'<polyline fill="none" stroke="{c}" stroke-width="1" stroke-dasharray="4 3" points="{pts}"/>')
        parts.append(f'<text x="{w - pad - 230}" y="{pad + 16 * k}" fill="{c}">n{n} {kind}: RSS (solid), heap (dashed)</text>')
    parts.append("</svg>")
    open(path, "w").write("\n".join(parts) + "\n")


if __name__ == "__main__":
    main()
