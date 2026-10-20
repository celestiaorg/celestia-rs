#!/usr/bin/env python3
"""Markdown tables from conformance/bench/run.sh results (Go vs Rust)."""

import json
import os
import platform
import subprocess
import sys

d = sys.argv[1]


def load(name):
    p = os.path.join(d, name)
    return json.load(open(p)) if os.path.exists(p) else None


def cpu():
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    try:
        return subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip()
    except OSError:
        return "?"


def ram_gb():
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemTotal"):
                return round(int(line.split()[1]) / 2**20, 1)
    except OSError:
        pass
    try:
        out = subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True).stdout
        return round(int(out) / 2**30, 1)
    except (OSError, ValueError):
        return "?"


def ratio(go, rs, lower_is_better=True):
    try:
        r = rs / go if lower_is_better else go / rs
        return f"{r:.2f}x"
    except (TypeError, ZeroDivisionError):
        return "-"


out = [f"Machine: {platform.system()} {platform.machine()}, {cpu()}, {os.cpu_count()} CPUs, {ram_gb()} GiB RAM", ""]
g, r = load("replay-go.json"), load("replay-rs.json")
if g and r:
    gs, rs = g["summary"], r["summary"]
    out += [f"### A. App-only replay ({gs['heights']} heights, {gs['txs']} txs, {gs['heavy_blocks']} blocks over 2 MiB, "
            f"max block {gs['max_block_bytes'] / 2**20:.1f} MiB)", "",
            "Rust/Go ratio: < 1 means Rust is faster / smaller.", "",
            "| metric | Go celestia-appd | celestia-rs | Rust/Go |", "|---|---|---|---|"]

    def row(label, key, field, unit="ms"):
        a, b = gs[key][field], rs[key][field]
        out.append(f"| {label} | {a:.1f} {unit} | {b:.1f} {unit} | {ratio(a, b)} |")

    for label, key in [("FinalizeBlock+Commit, all blocks", "finalize_commit"),
                       ("FinalizeBlock+Commit, light blocks", "finalize_commit_light"),
                       ("FinalizeBlock+Commit, heavy blocks", "finalize_commit_heavy"),
                       ("FinalizeBlock", "finalize"), ("Commit", "commit"),
                       ("PrepareProposal, max-size squares", "prepare_heavy"),
                       ("ProcessProposal, max-size squares", "process_heavy"),
                       ("PrepareProposal, light blocks", "prepare_light"),
                       ("ProcessProposal, light blocks", "process_light")]:
        if gs[key]["n"] and rs[key]["n"]:
            row(f"{label} p50", key, "p50_ms")
            row(f"{label} p99", key, "p99_ms")
    out.append(f"| execution throughput | {gs['exec_blocks_per_s']:.2f} blocks/s ({gs['exec_txs_per_s']:.0f} tx/s) | "
               f"{rs['exec_blocks_per_s']:.2f} blocks/s ({rs['exec_txs_per_s']:.0f} tx/s) | "
               f"{ratio(gs['exec_blocks_per_s'], rs['exec_blocks_per_s'], False)} |")
    for label, key in [("CheckTx (new)", "checktx_new"), ("CheckTx (recheck)", "checktx_recheck")]:
        a, b = gs[key]["tx_per_s"], rs[key]["tx_per_s"]
        out.append(f"| {label} throughput ({gs[key]['txs']} txs) | {a:.0f} tx/s | {b:.0f} tx/s | {ratio(a, b, False)} |")
    a, b = gs["end_memory"], rs["end_memory"]
    out.append(f"| app RSS at end (peak) | {a['rss_mb']} MB ({a['hwm_mb']}) | {b['rss_mb']} MB ({b['hwm_mb']}) | "
               f"{ratio(a['rss_mb'], b['rss_mb'])} |")
    out.append(f"| app state on disk at end | {gs['state_disk_mb']} MB | {rs['state_disk_mb']} MB | "
               f"{ratio(gs['state_disk_mb'], rs['state_disk_mb'])} |")
    out.append(f"| app hash mismatches vs recording | {gs['app_hash_mismatches']} / {gs['app_hash_compared']} | "
               f"{rs['app_hash_mismatches']} / {rs['app_hash_compared']} | |")
    out.append("")
    alt = load("replay-rs-alt.json")
    if alt:
        al = alt["summary"]
        out += ["Second Rust build (`CELESTIA_RS_ALT`) on the same replay:", "",
                "| metric | celestia-rs | celestia-rs (alt build) |", "|---|---|---|"]
        for label, key in [("FinalizeBlock+Commit p50", ("finalize_commit", "p50_ms")),
                           ("FinalizeBlock+Commit p99", ("finalize_commit", "p99_ms")),
                           ("PrepareProposal max-size p50", ("prepare_heavy", "p50_ms")),
                           ("ProcessProposal max-size p50", ("process_heavy", "p50_ms"))]:
            a, b = rs[key[0]][key[1]], al[key[0]][key[1]]
            out.append(f"| {label} | {a:.1f} ms | {b:.1f} ms |")
        out.append(f"| CheckTx (new) | {rs['checktx_new']['tx_per_s']:.0f} tx/s | {al['checktx_new']['tx_per_s']:.0f} tx/s |")
        out.append(f"| app RSS at end (peak) | {rs['end_memory']['rss_mb']} MB ({rs['end_memory']['hwm_mb']}) | "
                   f"{al['end_memory']['rss_mb']} MB ({al['end_memory']['hwm_mb']}) |")
        out.append("")
rows = []
b_g, b_r = load("blocksync-go.json"), load("blocksync-rs.json")
if b_g and b_r:
    a, b = b_g["heights"] / b_g["secs"], b_r["heights"] / b_r["secs"]
    rows.append(f"| B. block sync of {b_g['heights']} heights | {b_g['secs']:.1f} s ({a:.1f} blocks/s) | "
                f"{b_r['secs']:.1f} s ({b:.1f} blocks/s) | {ratio(b_g['secs'], b_r['secs'])} |")
    rows.append(f"| B. full node RSS after block sync (peak) | {b_g['memory']['rss_mb']} MB ({b_g['memory']['hwm_mb']}) | "
                f"{b_r['memory']['rss_mb']} MB ({b_r['memory']['hwm_mb']}) | {ratio(b_g['memory']['rss_mb'], b_r['memory']['rss_mb'])} |")
    rows.append(f"| B. full node data/ after block sync | {b_g['disk_mb']} MB | {b_r['disk_mb']} MB | "
                f"{ratio(b_g['disk_mb'], b_r['disk_mb'])} |")
s_g, s_r = load("startup-go.json"), load("startup-rs.json")
if s_g and s_r:
    rows.append(f"| C. restart: RPC up | {s_g['rpc_up_s']:.2f} s | {s_r['rpc_up_s']:.2f} s | {ratio(s_g['rpc_up_s'], s_r['rpc_up_s'])} |")
    rows.append(f"| C. restart: first new height committed | {s_g['first_new_height_s']:.2f} s | {s_r['first_new_height_s']:.2f} s | "
                f"{ratio(s_g['first_new_height_s'], s_r['first_new_height_s'])} |")
d_g, d_r = load("statesync-go.json"), load("statesync-rs.json")
if d_g and d_r:
    rows.append(f"| D. state sync (snapshot {d_g['snapshot_height']}): restored | {d_g['restored_s']:.1f} s | {d_r['restored_s']:.1f} s | "
                f"{ratio(d_g['restored_s'], d_r['restored_s'])} |")
    rows.append(f"| D. state sync: first block after the snapshot | {d_g['first_height_s']:.1f} s | {d_r['first_height_s']:.1f} s | "
                f"{ratio(d_g['first_height_s'], d_r['first_height_s'])} |")
    rows.append(f"| D. RSS after state sync | {d_g['memory']['rss_mb']} MB | {d_r['memory']['rss_mb']} MB | "
                f"{ratio(d_g['memory']['rss_mb'], d_r['memory']['rss_mb'])} |")
if rows:
    out += ["### B-D. Full node (Go celestia-appd vs celestia-rs start)", "",
            "| metric | Go | Rust | Rust/Go |", "|---|---|---|---|"] + rows
print("\n".join(out))
