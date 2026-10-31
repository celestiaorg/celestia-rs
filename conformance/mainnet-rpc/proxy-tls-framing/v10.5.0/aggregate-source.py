#!/usr/bin/env python3
"""Combine the main transport capture and sequential TLS-config probes."""

import json
import pathlib


HERE = pathlib.Path(__file__).parent
campaign = json.loads((HERE / "campaign.json").read_text())
transport = json.loads((HERE / "transport-capture.json").read_text())
config_names = campaign["categories"]["tls-config"]
config_cases = [json.loads((HERE / f"{name}.json").read_text()) for name in config_names]
cases = transport["cases"] + config_cases
expected = [name for names in campaign["categories"].values() for name in names]
actual = [case["name"] for case in cases]
if actual != expected:
    raise ValueError(f"case order differs from frozen inventory: {actual!r}")
result = {
    "schema_version": 1,
    "captured_at": transport["captured_at"],
    "dynamic_response_headers_excluded_from_stable_projection": transport[
        "dynamic_response_headers_excluded_from_stable_projection"
    ],
    "cases": cases,
}
(HERE / "proxy-tls-framing.json").write_text(json.dumps(result, indent=2) + "\n")
