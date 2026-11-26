#!/usr/bin/env python3
"""Fetch the mainnet blocks that carry evidence, with what verifying it needs.

Usage: python3 -I fetch.py [RPC]  (default https://rpc.celestia.pops.one)

Heights were found with block_search for slash.reason='double_sign'.
Writes <height>.json: the block (txs dropped), the header at the evidence height, the
header of the previous block (state.last_block_time), the validator set at
the evidence height and the consensus params at the block height.
"""
import json
import sys
import time
import urllib.request

HEIGHTS = [8218923, 9217879]
RPC = sys.argv[1] if len(sys.argv) > 1 else "https://rpc.celestia.pops.one"


def get(path):
    wait = 1.0
    while True:
        try:
            with urllib.request.urlopen(RPC + path, timeout=60) as r:
                return json.load(r)["result"]
        except Exception as e:  # noqa: BLE001 - retry 429 and transient errors
            if wait > 64:
                raise
            print(f"{path}: {e}; retry in {wait}s", file=sys.stderr)
            time.sleep(wait)
            wait *= 2


for h in HEIGHTS:
    block = get(f"/block?height={h}")
    # Evidence checks do not read the txs; drop them to keep the fixture small.
    block["block"]["data"]["txs"] = []
    ev = block["block"]["evidence"]["evidence"]
    ev_height = int(ev[0]["value"]["vote_a"]["height"])
    vals, page = [], 1
    while True:
        r = get(f"/validators?height={ev_height}&page={page}&per_page=100")
        vals += r["validators"]
        if len(vals) >= int(r["total"]):
            break
        page += 1
    out = {
        "height": h,
        "evidence_height": ev_height,
        "block": block["block"],
        "evidence_header": get(f"/header?height={ev_height}")["header"],
        "previous_header": get(f"/header?height={h - 1}")["header"],
        "validators": vals,
        "consensus_params": get(f"/consensus_params?height={h}")["consensus_params"],
    }
    with open(f"{h}.json", "w") as f:
        json.dump(out, f, separators=(",", ":"), sort_keys=True)
        f.write("\n")
    print(h, ev_height, len(vals), "validators")
