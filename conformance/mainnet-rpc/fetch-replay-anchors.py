#!/usr/bin/env python3
"""Pin each replay window's pre-state commit info from archive RPC proofs.

For the height before each window, every store root comes from the
multistore proof of an `abci_query` with `prove=true`. The roots must hash to
the window's first header app hash, which the fixture already pins, so the
recorded store set is exact. The script also records the upgrade plan stored
at that height. Windows whose pre-state the RPC no longer retains get null.
"""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import time


FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "mainnet" / "replay-windows.json"
STORES = [
    "acc", "authz", "bank", "blob", "blobstream", "capability", "circuit", "consensus",
    "distribution", "evidence", "feegrant", "fibre", "gov", "hyperlane", "ibc", "icahost",
    "minfee", "mint", "packetfowardmiddleware", "params", "qgb", "signal", "slashing",
    "staking", "transfer", "upgrade", "valaddr", "warp", "zkism",
]
# Cosmos SDK v0.46 also commits memory stores, with an empty commit ID.
MEMORY_STORES = ["mem_capability"]


def uvarint(data, i):
    shift = value = 0
    while True:
        byte = data[i]
        i += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, i
        shift += 7


def fields(data):
    i, out = 0, {}
    while i < len(data):
        tag, i = uvarint(data, i)
        if tag & 7 == 0:
            value, i = uvarint(data, i)
        elif tag & 7 == 2:
            size, i = uvarint(data, i)
            value, i = data[i:i + size], i + size
        else:
            raise ValueError(f"unexpected wire type {tag & 7}")
        out.setdefault(tag >> 3, value)
    return out


def put_uvarint(value):
    out = bytearray()
    while value >= 0x80:
        out.append((value & 0x7F) | 0x80)
        value >>= 7
    out.append(value)
    return bytes(out)


def sha256(data):
    return hashlib.sha256(data).digest()


def merkle_root(leaves):
    if len(leaves) == 1:
        return sha256(b"\0" + leaves[0])
    split = 1
    while split * 2 < len(leaves):
        split *= 2
    return sha256(b"\1" + merkle_root(leaves[:split]) + merkle_root(leaves[split:]))


def commit_hash(roots):
    leaves = []
    for name in sorted(roots):
        value = sha256(bytes.fromhex(roots[name]))
        leaves.append(put_uvarint(len(name)) + name.encode() + put_uvarint(32) + value)
    return merkle_root(leaves).hex().upper()


def query(rpc, store, height, prove):
    url = f'{rpc}/abci_query?path="/store/{store}/key"&data=0x00&height={height}&prove={prove}'
    for attempt in range(6):
        output = subprocess.run(
            ["curl", "--silent", "--show-error", "--max-time", "60", url],
            capture_output=True, check=False,
        ).stdout
        try:
            return json.loads(output)["result"]["response"]
        except (ValueError, KeyError):
            time.sleep(2 * (attempt + 1))
    raise RuntimeError(f"{rpc} did not answer for {store} at {height}")


def store_root(rpc, store, height):
    response = query(rpc, store, height, "true")
    time.sleep(0.2)
    if response["code"] != 0:
        return None
    op = response["proofOps"]["ops"][1]
    proof = fields(fields(base64.b64decode(op["data"]))[1])
    if op["type"] != "ics23:simple" or proof[1].decode() != store:
        raise ValueError(f"unexpected multistore proof for {store} at {height}")
    return proof[2].hex().upper()


def upgrade_plan(rpc, height):
    response = query(rpc, "upgrade", height, "false")
    if response["code"] != 0 or not response["value"]:
        return None
    plan = fields(base64.b64decode(response["value"]))
    return {"name": plan[1].decode(), "height": plan[3]}


def anchor(rpc, height, app_hash):
    roots = {}
    for store in STORES:
        root = store_root(rpc, store, height)
        if root is not None:
            roots[store] = root
    if not roots:
        return None
    if commit_hash(roots) != app_hash:
        roots.update({store: "" for store in MEMORY_STORES})
    if commit_hash(roots) != app_hash:
        raise ValueError(f"store roots at {height} do not reproduce app hash {app_hash}")
    stores = dict(sorted(roots.items()))
    return {"height": height, "upgrade_plan": upgrade_plan(rpc, height), "stores": stores}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc", default="https://rpc.celestia.pops.one")
    args = parser.parse_args()
    fixture = json.loads(FIXTURE.read_text())
    windows = []
    for window in fixture["windows"]:
        height = window["start_height"] - 1
        pre_state = None
        if height > 0:
            pre_state = anchor(args.rpc, height, window["blocks"][0]["app_hash"])
            if pre_state is not None:
                pre_state["rpc"] = args.rpc
        print(f"app v{window['app_version']} pre-state {height}: {pre_state is not None}")
        rest = {k: v for k, v in window.items() if k not in ("app_version", "start_height", "pre_state")}
        windows.append(
            {
                "app_version": window["app_version"],
                "start_height": window["start_height"],
                "pre_state": pre_state,
                **rest,
            }
        )
    fixture["windows"] = windows
    FIXTURE.write_text(json.dumps(fixture, indent=2) + "\n")


if __name__ == "__main__":
    main()
