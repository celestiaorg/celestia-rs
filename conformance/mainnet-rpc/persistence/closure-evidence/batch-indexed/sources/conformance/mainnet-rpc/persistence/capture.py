#!/usr/bin/env python3
"""Capture exact decoded Comet RPC pairs without hiding observed differences."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlencode, urlsplit
from urllib.request import urlopen


def fetch(base, method, params):
    parsed = urlsplit(base)
    if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost") or parsed.username or parsed.password:
        raise ValueError("capture requires an isolated local HTTP node")
    url = base.rstrip("/") + "/" + method + "?" + urlencode(params)
    try:
        response = urlopen(url, timeout=15)
    except HTTPError as error:
        response = error
    with response:
        body = response.read(8 << 20)
        if response.read(1):
            raise ValueError("response exceeds 8 MiB")
        return {"status": response.status, "content_type": response.headers.get("Content-Type"),
                "body": body.decode(), "body_sha256": hashlib.sha256(body).hexdigest()}


def comparable(response):
    # Object-key order and JSON whitespace are immaterial; arrays, scalar types,
    # error messages, proof fields, pagination and empty/null values remain exact.
    return response["status"], response["content_type"], json.loads(response["body"])


def cases(height, sender, tx):
    for h in (0, 1, height, 1_000_000_000):
        for method in ("block", "block_results", "commit", "validators"):
            yield method, {"height": h}
    for page in (1, 2, 100):
        for order in ("asc", "desc"):
            yield "block_search", {"query": json.dumps(f"block.height <= {height}"),
                                   "page": page, "per_page": 2, "order_by": json.dumps(order)}
            yield "tx_search", {"query": json.dumps(f"message.sender='{sender}'"),
                                "page": page, "per_page": 2, "order_by": json.dumps(order), "prove": "true"}
    for query in ("tx.height=0", "invalid query ("):
        yield "tx_search", {"query": json.dumps(query)}
    for tx_hash in (tx, "00" * 32):
        yield "tx", {"hash": "0x" + tx_hash, "prove": "true"}
    encoded = sender.encode()
    request = b"\x0a" + bytes([len(encoded)]) + encoded
    for h in (0, 1, height, 1_000_000_000):
        yield "abci_query", {"path": json.dumps("/cosmos.bank.v1beta1.Query/AllBalances"),
                             "data": "0x" + request.hex(), "height": h, "prove": "false"}
        # Cosmos bank's supply prefix 0x00 followed by the denomination.
        yield "abci_query", {"path": json.dumps("/store/bank/key"), "data": "0x0075746961", "height": h, "prove": "true"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("go", "rs", "sender", "tx", "chain-id"):
        parser.add_argument("--" + key, required=True)
    parser.add_argument("--height", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    report = {"captured_at": datetime.now(timezone.utc).isoformat(),
              "chain_id": args.chain_id, "height": args.height,
              "comparison": "decoded-json-exact", "cases": [], "errors": [],
              "exclusions": ["HTTP Date, X-Server-Time and derived Content-Length",
                             "Cosmos gRPC, REST, WebSocket lifecycle, state sync and pre-v10 multiplexer"]}
    try:
        for role in ("go", "rs"):
            base = getattr(args, role)
            status = fetch(base, "status", {})
            info = json.loads(status["body"])["result"]
            if info["node_info"]["network"] != args.chain_id or int(info["sync_info"]["latest_block_height"]) < args.height + 1:
                raise ValueError(f"{role}: wrong chain or insufficient height")
            report[role + "_status"] = status
        if json.loads(report["go_status"]["body"])["result"]["sync_info"]["latest_block_height"] != json.loads(report["rs_status"]["body"])["result"]["sync_info"]["latest_block_height"]:
            raise ValueError("Go/Rust tips differ; stop the validator and wait for catch-up before capture")
        roots = {}
        for role in ("go", "rs"):
            response = fetch(getattr(args, role), "block", {"height": args.height + 1})
            report[role + "_committing_block"] = response
            roots[role] = json.loads(response["body"])["result"]["block"]["header"]["app_hash"]
        if roots["go"] != roots["rs"]:
            raise ValueError("Go/Rust committed app roots differ")
        report["matched_app_hash"] = roots["go"]
        for method, params in cases(args.height, args.sender, args.tx):
            row = {"method": method, "params": params}
            report["cases"].append(row)
            for role in ("go", "rs"):
                row[role] = fetch(getattr(args, role), method, params)
            row["outcome"] = "match" if comparable(row["go"]) == comparable(row["rs"]) else "mismatch"
    except Exception as error:
        report["errors"].append(str(error))
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    failed = bool(report["errors"]) or any(row.get("outcome") == "mismatch" for row in report["cases"])
    print(f"{args.out}: {len(report['cases'])} pairs, {sum(row.get('outcome') == 'mismatch' for row in report['cases'])} mismatches, errors={report['errors']}")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
