#!/usr/bin/env python3
"""Bind local tagged-Go Comet and application identities at one height."""

import datetime
import hashlib
import json
import pathlib
import time
import urllib.request


WORK = pathlib.Path("/workspace/scratch/rpc-181/comet-http")


def get(port: int, route: str) -> str:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/{route}", timeout=5) as response:
        return response.read().decode()


for name, port in (("zero", 6320), ("custom", 6330)):
    for _ in range(20):
        status_body = get(port, "status")
        abci_body = get(port, "abci_info")
        status = json.loads(status_body)["result"]
        abci = json.loads(abci_body)["result"]["response"]
        if status["sync_info"]["latest_block_height"] == abci["last_block_height"]:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError(f"{name}: status and ABCI heights did not converge")
    result = {
        "schema_version": 1,
        "observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "endpoint": f"http://127.0.0.1:{port}",
        "network": status["node_info"]["network"],
        "comet_version": status["node_info"]["version"],
        "protocol_app": status["node_info"]["protocol_version"]["app"],
        "height": status["sync_info"]["latest_block_height"],
        "app_version": abci["version"],
        "abci_app_version": abci["app_version"],
        "responses": {
            "status": {
                "body": status_body,
                "sha256": hashlib.sha256(status_body.encode()).hexdigest(),
            },
            "abci_info": {
                "body": abci_body,
                "sha256": hashlib.sha256(abci_body.encode()).hexdigest(),
            },
        },
    }
    (WORK / f"{name}-identity.json").write_text(json.dumps(result, indent=2) + "\n")
    print(name, result["height"], result["app_version"], result["comet_version"])
