#!/usr/bin/env python3
"""Explicit read-only oracle selection; never run by the offline CI ledger."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import urlopen

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--endpoint", required=True, help="Public mainnet HTTPS Comet RPC base URL")
parser.add_argument("--out", required=True, type=Path)
args = parser.parse_args()
url = urlsplit(args.endpoint)
if url.scheme != "https" or not url.hostname or url.username or url.password or url.query or url.fragment:
    parser.error("use a public HTTPS base URL without credentials, query or fragment")
base = args.endpoint.rstrip("/")
responses = {}
for route in ("status", "abci_info"):
    with urlopen(base + "/" + route, timeout=20) as response:
        body = response.read(2 << 20)
        if response.read(1):
            raise ValueError("mainnet response exceeds 2 MiB")
    responses[route] = {"sha256": hashlib.sha256(body).hexdigest(), "body": body.decode("utf-8")}
status = json.loads(responses["status"]["body"])["result"]
info = json.loads(responses["abci_info"]["body"])["result"]["response"]
policy = json.loads(Path(__file__).with_name("policy.json").read_text())
if status["node_info"]["network"] != "celestia":
    raise ValueError("endpoint is not mainnet (chain ID celestia)")
if info["version"] != policy["oracle"]["celestia_app"].removeprefix("v"):
    raise ValueError("active mainnet version differs from policy: review oracle selection before capturing")
result = {"endpoint": base, "observed_at": datetime.now(timezone.utc).isoformat(),
          "version": info["version"], "chain_id": "celestia", "responses": responses}
args.out.parent.mkdir(parents=True, exist_ok=True)
args.out.write_text(json.dumps(result, indent=2) + "\n")
print(f"Observed celestia-app {info['version']} on celestia; this selects an oracle, not provider parity")
