"""Fetches a public network's blocks for the `genesis_replay` example, in
parallel (use after fetch-chain.sh fetched the genesis):

  <out>/block/<h>.json        /block            h = first..last+1
  <out>/results/<h>.json      /block_results    only heights with txs
  <out>/validators/<h>.json   /validators       only where the set changes
                                                (header validators_hash)

usage: python3 fetch-chain.py <rpc> <out> <first> <last> [threads]
"""
import concurrent.futures as cf
import json
import os
import sys
import time
import urllib.request

rpc, out, first, last = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
threads = int(sys.argv[5]) if len(sys.argv) > 5 else 8
for d in ("block", "results", "validators"):
    os.makedirs(os.path.join(out, d), exist_ok=True)


def get(path):
    for i in range(10):
        try:
            with urllib.request.urlopen(rpc + path, timeout=60) as r:
                body = r.read()
            if b'"result"' in body:
                return body
        except Exception:  # noqa: BLE001
            pass
        time.sleep(1 + i)
    raise RuntimeError("failed: " + path)


def save(rel, body):
    tmp = os.path.join(out, rel + ".tmp")
    with open(tmp, "wb") as f:
        f.write(body)
    os.replace(tmp, os.path.join(out, rel))


def block(h):
    rel = f"block/{h}.json"
    p = os.path.join(out, rel)
    if not os.path.exists(p):
        save(rel, get(f"/block?height={h}"))
    with open(p) as f:
        return h, json.load(f)["result"]["block"]


def results(h):
    rel = f"results/{h}.json"
    if not os.path.exists(os.path.join(out, rel)):
        save(rel, get(f"/block_results?height={h}"))


def validators(h):
    if os.path.exists(os.path.join(out, f"validators/{h}.json")):
        return
    page, body = 1, get(f"/validators?height={h}&per_page=100&page=1")
    total = int(json.loads(body)["result"]["total"])
    save(f"validators/{h}.json", body)
    while page * 100 < total:
        page += 1
        save(f"validators/{h}.p{page}.json", get(f"/validators?height={h}&per_page=100&page={page}"))


prev_hash = None
with cf.ThreadPoolExecutor(threads) as ex:
    step = 500
    for lo in range(first, last + 2, step):
        hi = min(lo + step - 1, last + 1)
        blocks = dict(ex.map(block, range(lo, hi + 1)))
        jobs = []
        for h in range(lo, hi + 1):
            if h > last:
                continue
            b = blocks[h]
            if b["data"].get("txs"):
                jobs.append(ex.submit(results, h))
            vh = b["header"]["validators_hash"]
            if vh != prev_hash:
                jobs.append(ex.submit(validators, h))
                prev_hash = vh
        for j in jobs:
            j.result()
        print(f"fetched up to {hi}", flush=True)
