"""Waits for a tx to be committed by scanning blocks over CometBFT RPC
(the devnet node has tx indexing disabled).

usage: wait_tx.py <rpc http url> <tx hash>
prints: <height> <code> <log prefix> (code "?" when results are not kept)
"""
import base64
import hashlib
import json
import sys
import time
import urllib.request

rpc, want = sys.argv[1], sys.argv[2].upper()


def get(path):
    return json.load(urllib.request.urlopen(rpc + path))["result"]


h = int(get("/status")["sync_info"]["latest_block_height"])
deadline = time.time() + 60
while time.time() < deadline:
    try:
        b = get(f"/block?height={h}")
    except Exception:
        time.sleep(0.3)
        continue
    for i, t in enumerate(b["block"]["data"]["txs"] or []):
        if hashlib.sha256(base64.b64decode(t)).hexdigest().upper() == want:
            try:
                r = get(f"/block_results?height={h}")["txs_results"][i]
                print(h, r.get("code", 0), (r.get("log") or "")[:160])
            except Exception:  # ABCI responses are not kept by the node
                print(h, "?")
            sys.exit(0)
    h += 1
sys.exit(1)
