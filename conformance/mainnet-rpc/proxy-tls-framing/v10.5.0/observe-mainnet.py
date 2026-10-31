#!/usr/bin/env python3
"""Read-only active-release observation used to select the local v10.5.0 oracle."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import time
import urllib.parse
import urllib.request


EXPECTED_CHAIN = "celestia"
EXPECTED_RELEASE = "10.5.0"
ROUTES = ("status", "abci_info")


def get(base: str, route: str) -> dict[str, str]:
    request = urllib.request.Request(
        f"{base}/{route}",
        headers={"Accept": "application/json", "User-Agent": "celestia-rs-conformance/191"},
        method="GET",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        body = response.read(2 << 20)
        if response.read(1):
            raise ValueError(f"{route}: response exceeds 2 MiB")
        if response.status != 200:
            raise ValueError(f"{route}: HTTP {response.status}")
    return {
        "body": body.decode("utf-8"),
        "sha256": hashlib.sha256(body).hexdigest(),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.endpoint)
    if (
        url.scheme != "https"
        or not url.hostname
        or url.username
        or url.password
        or url.query
        or url.fragment
    ):
        parser.error("endpoint must be a public HTTPS base URL without credentials or suffixes")
    base = args.endpoint.rstrip("/")

    for _ in range(10):
        responses = {route: get(base, route) for route in ROUTES}
        status = json.loads(responses["status"]["body"])["result"]
        abci = json.loads(responses["abci_info"]["body"])["result"]["response"]
        if status["sync_info"]["latest_block_height"] == abci["last_block_height"]:
            break
        time.sleep(0.2)
    else:
        raise ValueError("status and abci_info did not converge on one height")

    if status["node_info"]["network"] != EXPECTED_CHAIN:
        raise ValueError("endpoint is not celestia mainnet")
    if abci["version"] != EXPECTED_RELEASE:
        raise ValueError("active application release changed; review oracle selection")
    if status["node_info"]["protocol_version"]["app"] != abci["app_version"]:
        raise ValueError("status and abci_info disagree on application protocol version")

    result = {
        "schema_version": 1,
        "purpose": "oracle selection only; no provider parity claim",
        "endpoint": base,
        "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "chain_id": EXPECTED_CHAIN,
        "app_version": abci["version"],
        "protocol_app": abci["app_version"],
        "height": abci["last_block_height"],
        "responses": responses,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    print(
        f"observed {EXPECTED_CHAIN} app {abci['version']} protocol {abci['app_version']} "
        f"at height {abci['last_block_height']}"
    )


if __name__ == "__main__":
    main()
