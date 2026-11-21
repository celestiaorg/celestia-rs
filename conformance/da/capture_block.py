"""Capture a private Go consensus/bridge control block for offline Rust replay."""
import argparse
import base64
import json
from pathlib import Path
from urllib.request import Request, urlopen

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--consensus", required=True)
parser.add_argument("--bridge", required=True)
parser.add_argument("--token-file", type=Path, required=True)
parser.add_argument("--height", type=int, required=True)
parser.add_argument("--namespace-id", action="append", default=[])
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()


def call(method, params):
    request = Request(
        args.bridge,
        data=json.dumps(dict(jsonrpc="2.0", id=1, method=method, params=params)).encode(),
        headers={"Authorization": "Bearer " + args.token_file.read_text().strip(),
                 "Content-Type": "application/json"},
    )
    with urlopen(request, timeout=30) as response:
        obj = json.load(response)
    if "error" in obj:
        raise RuntimeError(obj["error"])
    return obj["result"]


with urlopen(f"{args.consensus}/block?height={args.height}", timeout=30) as response:
    block = json.load(response)["result"]
header = call("header.GetByHeight", [args.height])
assert header["commit"]["block_id"]["hash"] == block["block_id"]["hash"]
namespaces = {}
for ns_id in args.namespace_id:
    ns = base64.b64encode(bytes.fromhex(ns_id).rjust(29, bytes(1))).decode()
    namespaces[ns] = call("share.GetNamespaceData", [args.height, ns])
fixture = dict(header=header, txs=block["block"]["data"]["txs"] or [],
               eds=call("share.GetEDS", [args.height]), namespaces=namespaces)
args.output.write_text(json.dumps(fixture, indent=2) + "\n")
