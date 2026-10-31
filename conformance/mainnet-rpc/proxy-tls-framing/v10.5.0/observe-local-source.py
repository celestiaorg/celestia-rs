#!/usr/bin/env python3
"""Bind the plaintext and TLS tagged-Go nodes to exact app/core identities."""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import pathlib
import ssl
import time
import urllib.request


HERE = pathlib.Path(__file__).parent
CERT = HERE / "tls" / "localhost.crt"
ORACLES = {
    "cert_only": ("http://127.0.0.1:6340", None),
    "relative": ("https://localhost:6350", ssl.create_default_context(cafile=str(CERT))),
}


def get(base: str, route: str, context: ssl.SSLContext | None) -> dict[str, str]:
    with urllib.request.urlopen(f"{base}/{route}", timeout=5, context=context) as response:
        body = response.read(2 << 20)
        if response.read(1):
            raise ValueError(f"{route}: response exceeds 2 MiB")
    return {"body": body.decode(), "sha256": hashlib.sha256(body).hexdigest()}


for name, (endpoint, context) in ORACLES.items():
    for _ in range(20):
        responses = {
            route: get(endpoint, route, context) for route in ("status", "abci_info")
        }
        status = json.loads(responses["status"]["body"])["result"]
        abci = json.loads(responses["abci_info"]["body"])["result"]["response"]
        if status["sync_info"]["latest_block_height"] == abci["last_block_height"]:
            break
        time.sleep(0.1)
    else:
        raise ValueError(f"{name}: status and abci_info did not converge")
    result = {
        "schema_version": 1,
        "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "endpoint": endpoint,
        "network": status["node_info"]["network"],
        "comet_version": status["node_info"]["version"],
        "protocol_app": status["node_info"]["protocol_version"]["app"],
        "height": status["sync_info"]["latest_block_height"],
        "app_version": abci["version"],
        "abci_app_version": abci["app_version"],
        "responses": responses,
    }
    (HERE / f"{name}-identity.json").write_text(json.dumps(result, indent=2) + "\n")
    print(name, result["height"], result["app_version"], result["comet_version"])
