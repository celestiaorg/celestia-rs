#!/usr/bin/env python3
"""Sampled data-root campaign over Celestia mainnet history.

Block sync accepts a committed block of app versions 1 to 8 only if the
data square rebuilt from its txs reproduces the header's data hash
(`app::proposal::verify_committed_block_data`); v9 and later blocks pass
ProcessProposal, which checks the same root. This script picks heights
across every era, fetches the blocks over public RPC, and feeds them to
`cargo run -p app --example data_root_check`, which rebuilds each square.

  campaign.py plan OUT.json [--even N] [--edge N] [--windows N]
      Heights per era: N evenly spaced, the first and last --edge blocks,
      and hard cases from --windows random /blockchain windows of 20
      block metas each (largest blocks, most txs, empty blocks).
  campaign.py fetch HEIGHTS...|FILE
      Writes one JSON line per block (height, app_version, data_hash,
      square_size, txs) to stdout, for data_root_check.
  campaign.py run PLAN.json RESULTS.jsonl
      fetch | data_root_check for every planned height, appending results;
      heights already in RESULTS.jsonl are skipped (resumable).
  campaign.py report PLAN.json RESULTS.jsonl
      Per-era counts, mismatches and hard-case coverage.
  campaign.py harvest PLAN.json RESULTS.jsonl CASES.json BODIES.jsonl.gz
      Picks each era's hard cases from the results (era edges, largest
      square, most blobs, share version 1 blobs, PFB-only, empty, longest
      tx), refetches them, and writes their heights, reasons and committed
      data hash and square size to CASES.json. Blocks whose txs total at
      most 256 KiB also go to BODIES.jsonl.gz for the offline CI test.

Uses only the standard library. RPCs alternate and back off on 429/5xx.
"""

import json
import os
import random
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

RPCS = ["https://rpc.celestia.pops.one", "https://celestia-archive-rpc.polkachu.com"]

# First height of each app version on mainnet (header version.app).
ERAS = [
    (1, 1),
    (2, 2_371_495),
    (3, 2_993_220),
    (4, 6_680_340),
    (5, 6_748_822),
    (6, 8_662_013),
    (8, 10_960_600),
    (9, 11_771_699),
    (10, 15_116_659),
]

_lock = threading.Lock()
_turn = [0]


def get(path, tries=8):
    """GET path from the RPCs in turn, backing off on rate limits."""
    delay = 1.0
    last = None
    for attempt in range(tries):
        with _lock:
            base = RPCS[_turn[0] % len(RPCS)]
            _turn[0] += 1
        req = urllib.request.Request(base + path, headers={"User-Agent": "data-root-campaign"})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return json.load(r)["result"]
        except urllib.error.HTTPError as e:
            last = f"{base}{path}: HTTP {e.code}"
            if e.code not in (429, 500, 502, 503, 504):
                raise RuntimeError(last) from e
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
            last = f"{base}{path}: {e}"
        time.sleep(delay + random.random())
        delay = min(delay * 2, 60)
    raise RuntimeError(f"giving up: {last}")


def tip():
    return int(get("/status")["sync_info"]["latest_block_height"])


def eras(last):
    """(app_version, first, last) for every era up to `last`."""
    out = []
    for i, (v, first) in enumerate(ERAS):
        end = ERAS[i + 1][1] - 1 if i + 1 < len(ERAS) else last
        out.append((v, first, end))
    return out


def metas(lo, hi):
    """Block metas of heights lo..hi (at most 20)."""
    res = get(f"/blockchain?minHeight={lo}&maxHeight={hi}")
    return [
        {
            "height": int(m["header"]["height"]),
            "app": int(m["header"]["version"]["app"]),
            "size": int(m["block_size"]),
            "txs": int(m["num_txs"]),
        }
        for m in res["block_metas"]
    ]


def plan(out_path, even=300, edge=50, windows=150, seed=1):
    rng = random.Random(seed)
    last = tip()
    plan = {"tip": last, "eras": []}
    for v, first, end in eras(last):
        n = end - first + 1
        heights = {first + (n - 1) * i // (even - 1) for i in range(even)} if n > even else set(range(first, end + 1))
        edges = set(range(first, min(first + edge, end + 1))) | set(range(max(end - edge + 1, first), end + 1))
        seen = []
        starts = sorted(rng.randrange(first, max(first + 1, end - 19)) for _ in range(windows))
        with ThreadPoolExecutor(4) as ex:
            for ms in ex.map(lambda s: metas(s, min(s + 19, end)), starts):
                seen.extend(ms)
        for m in seen:
            assert m["app"] == v, (v, m)
        hard = {}
        for key, label, k in [("size", "largest", 15), ("txs", "most_txs", 10)]:
            for m in sorted(seen, key=lambda m: -m[key])[:k]:
                hard.setdefault(m["height"], label)
        for m in [m for m in seen if m["txs"] == 0][:5]:
            hard.setdefault(m["height"], "empty")
        plan["eras"].append(
            {
                "app_version": v,
                "first": first,
                "last": end,
                "even": sorted(heights),
                "edges": sorted(edges),
                "hard": {str(h): label for h, label in sorted(hard.items())},
                "metas_scanned": len(seen),
                "max_block_size": max((m["size"] for m in seen), default=0),
            }
        )
        print(f"v{v}: {first}..{end} even={len(heights)} edges={len(edges)} hard={len(hard)} "
              f"scanned={len(seen)} max_size={plan['eras'][-1]['max_block_size']}", file=sys.stderr)
    with open(out_path, "w") as f:
        json.dump(plan, f, indent=1)


def block(height):
    r = get(f"/block?height={height}")["block"]
    return {
        "height": int(r["header"]["height"]),
        "app_version": int(r["header"]["version"]["app"]),
        "data_hash": r["header"]["data_hash"],
        "square_size": int(r["data"].get("square_size") or 0),
        "txs": r["data"]["txs"] or [],
    }


def fetch(heights, sink, workers=4):
    with ThreadPoolExecutor(workers) as ex:
        for b in ex.map(block, heights):
            sink.write(json.dumps(b) + "\n")
            sink.flush()


def planned(plan):
    for era in plan["eras"]:
        hs = set(era["even"]) | set(era["edges"]) | {int(h) for h in era["hard"]}
        yield era, sorted(hs)


def done(results_path):
    out = {}
    if os.path.exists(results_path):
        with open(results_path) as f:
            for line in f:
                if line.strip():
                    r = json.loads(line)
                    if "height" in r:
                        out[r["height"]] = r
    return out


def run(plan_path, results_path, checker):
    plan = json.load(open(plan_path))
    have = done(results_path)
    todo = [h for _, hs in planned(plan) for h in hs if h not in have]
    print(f"{len(have)} done, {len(todo)} to go", file=sys.stderr)
    with open(results_path, "a") as res:
        proc = subprocess.Popen([checker], stdin=subprocess.PIPE, stdout=res, text=True)
        try:
            for i in range(0, len(todo), 40):
                fetch(todo[i : i + 40], proc.stdin)
                print(f"fetched {min(i + 40, len(todo))}/{len(todo)}", file=sys.stderr)
        except BrokenPipeError:
            pass  # the checker exited; its status is reported below
        finally:
            try:
                proc.stdin.close()
            except BrokenPipeError:
                pass
            code = proc.wait()
    if code != 0:
        raise SystemExit(f"{checker} exited with {code}; {results_path} may be incomplete")


def report(plan_path, results_path):
    plan = json.load(open(plan_path))
    have = done(results_path)
    bad_input = 0
    for era, hs in planned(plan):
        rs = [have[h] for h in hs if h in have]
        bad = [r for r in rs if not r["ok"]]
        shapes = [r["shape"] for r in rs if "shape" in r]
        cover = {
            "empty": sum(s["txs"] == 0 for s in shapes),
            "pfb_only": sum(s["txs"] > 0 and s["txs"] == s["blob_txs"] for s in shapes),
            "share_v1": sum(s["share_v1_blobs"] > 0 for s in shapes),
            "many_blobs(>=50)": sum(s["blobs"] >= 50 for s in shapes),
            "long_tx": sum(s["long_txs"] > 0 for s in shapes),
            "max_square": max((s["square_size"] for s in shapes), default=0),
            "max_blobs": max((s["blobs"] for s in shapes), default=0),
        }
        pp = [r for r in rs if r.get("stateless_process_proposal")]
        print(f"v{era['app_version']} [{era['first']}..{era['last']}]: checked {len(rs)}/{len(hs)}, "
              f"mismatches {len(bad)}, stateless ProcessProposal rejects {len(pp)}; {cover}")
        for r in bad[:20]:
            print(f"   MISMATCH {r['height']}: {r['error']}")
        for r in pp[:20]:
            print(f"   PROCESS-PROPOSAL REJECT {r['height']}: {r['stateless_process_proposal']}")
    bad_input = sum(1 for r in have.values() if "app_version" not in r)
    if bad_input:
        print(f"{bad_input} rows without a block (bad input)")


BODY_LIMIT = 256 * 1024


def harvest(plan_path, results_path, cases_path, bodies_path):
    import base64
    import gzip

    plan = json.load(open(plan_path))
    have = done(results_path)
    picks = {}
    for era, hs in planned(plan):
        rs = [have[h] for h in hs if h in have and have[h].get("ok")]

        def pick(reason, rows, key=None):
            rows = list(rows)
            if rows:
                r = max(rows, key=key) if key else rows[0]
                picks.setdefault(r["height"], []).append(reason)

        sh = lambda r: r["shape"]
        pick("era_first", [r for r in rs if r["height"] == era["first"]])
        pick("era_last", [r for r in rs if r["height"] == era["last"]])
        pick("largest_square", rs, key=lambda r: (sh(r)["square_size"], sh(r)["blobs"]))
        pick("most_blobs", rs, key=lambda r: sh(r)["blobs"])
        pick("most_txs", rs, key=lambda r: sh(r)["txs"])
        pick("share_v1", [r for r in rs if sh(r)["share_v1_blobs"] > 0], key=lambda r: sh(r)["share_v1_blobs"])
        pick("pfb_only", [r for r in rs if 0 < sh(r)["txs"] == sh(r)["blob_txs"]], key=lambda r: sh(r)["blobs"])
        pick("empty", [r for r in rs if sh(r)["txs"] == 0])
        pick("longest_tx", rs, key=lambda r: sh(r)["largest_tx"])
        # The smallest non-empty block with blobs keeps one cheap blob case per era.
        pick("small_blob_block", [r for r in rs if sh(r)["blobs"] > 0], key=lambda r: -sh(r)["square_size"] * 1000 + sh(r)["blobs"])
    cases = []
    with gzip.open(bodies_path, "wt") as bodies:
        for h in sorted(picks):
            b = block(h)
            size = sum(len(base64.b64decode(t)) for t in b["txs"])
            entry = {
                "height": h,
                "app_version": b["app_version"],
                "data_hash": b["data_hash"],
                "square_size": b["square_size"],
                "tx_bytes": size,
                "reasons": picks[h],
                "shape": have[h]["shape"],
                "body_in_fixture": size <= BODY_LIMIT,
            }
            cases.append(entry)
            if size <= BODY_LIMIT:
                bodies.write(json.dumps(b) + "\n")
            print(f"{h} v{b['app_version']} {size} bytes {picks[h]}", file=sys.stderr)
    with open(cases_path, "w") as f:
        json.dump({"source": "Celestia mainnet, via " + " and ".join(RPCS), "cases": cases}, f, indent=1)
        f.write("\n")


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "plan":
        args = argv[2:]
        kw = {}
        for flag, key in [("--even", "even"), ("--edge", "edge"), ("--windows", "windows")]:
            if flag in args:
                i = args.index(flag)
                kw[key] = int(args[i + 1])
                del args[i : i + 2]
        plan(args[0], **kw)
    elif cmd == "fetch":
        args = argv[2:]
        if len(args) == 1 and os.path.exists(args[0]):
            args = open(args[0]).read().split()
        fetch([int(h) for h in args], sys.stdout)
    elif cmd == "run":
        checker = os.environ.get("DATA_ROOT_CHECK", "target/release/examples/data_root_check")
        run(argv[2], argv[3], checker)
    elif cmd == "report":
        report(argv[2], argv[3])
    elif cmd == "harvest":
        harvest(argv[2], argv[3], argv[4], argv[5])
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
