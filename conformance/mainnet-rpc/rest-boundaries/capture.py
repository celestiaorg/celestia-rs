#!/usr/bin/env python3
"""Capture REST height parsing and metadata from a local Go oracle."""

import argparse
import datetime
import hashlib
import http.client
import json
import pathlib


VARIANTS = [
    ("absent", []),
    ("empty", [""]),
    ("zero", ["0"]),
    ("negative", ["-1"]),
    ("future", ["99999"]),
    ("overflow", ["9223372036854775808"]),
    ("invalid", ["x"]),
    ("space", [" 1"]),
    ("plus", ["+1"]),
    ("leading-zero", ["01"]),
    ("comma", ["1,2"]),
    ("repeated", ["1", "2"]),
    ("invalid-then-valid", ["x", "1"]),
    ("valid-then-invalid", ["1", "x"]),
]
ROUTES = [
    ("params", "/cosmos/bank/v1beta1/params"),
    ("handler-error", "/cosmos/bank/v1beta1/balances/bad"),
    ("unknown", "/not-a-route"),
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=1317, type=int)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    args = parser.parse_args()
    cases = []
    for route_name, path in ROUTES:
        for variant, heights in VARIANTS:
            connection = http.client.HTTPConnection(args.host, args.port, timeout=10)
            connection.putrequest("GET", path)
            for value in heights:
                connection.putheader("x-cosmos-block-height", value)
            connection.endheaders()
            response = connection.getresponse()
            body = response.read(1 << 20)
            headers = response.getheaders()
            connection.close()
            cases.append(
                {
                    "name": f"{route_name}:{variant}",
                    "path": path,
                    "heights": heights,
                    "status": response.status,
                    "headers": headers,
                    "body": body.decode(),
                }
            )
    source_hash = hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
    artifact = {
        "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_sha256": source_hash,
        "cases": cases,
    }
    args.out.write_text(json.dumps(artifact, indent=2) + "\n")


if __name__ == "__main__":
    main()
