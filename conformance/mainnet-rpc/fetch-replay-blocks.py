#!/usr/bin/env python3
"""Fetch the mainnet replay windows and their successors as gzip JSON Lines."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures" / "mainnet"


def fetch(rpc, chain_id, expected):
    height = expected["height"]
    response = subprocess.check_output(
        [
            "curl", "--fail", "--silent", "--show-error", "--retry", "2",
            "--max-time", "45", f"{rpc}/block?height={height}",
        ]
    )
    result = json.loads(response)["result"]
    header = result["block"]["header"]
    if (
        header["chain_id"] != chain_id
        or int(header["height"]) != height
        or result["block_id"]["hash"] != expected["block_hash"]
        or header["app_hash"] != expected["app_hash"]
        or ("data_hash" in expected and header["data_hash"] != expected["data_hash"])
        or int(header["version"]["app"]) != expected["app_version"]
    ):
        raise ValueError(f"archive RPC commitments differ at height {height}")
    return {"height": height, "result": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=FIXTURES / "blocks.jsonl.gz")
    args = parser.parse_args()
    fixture = json.loads((FIXTURES / "replay-windows.json").read_text())
    expected = []
    for window in fixture["windows"]:
        version = window["app_version"]
        expected.extend(dict(block, app_version=version) for block in window["blocks"])
        last = window["blocks"][-1]
        expected.append(
            {
                "height": last["height"] + 1,
                "block_hash": last["next_block_hash"],
                "app_hash": last["post_app_hash"],
                "app_version": version,
            }
        )

    with ThreadPoolExecutor(max_workers=5) as pool:
        records = list(
            pool.map(lambda row: fetch(fixture["rpc"], fixture["chain_id"], row), expected)
        )

    lines = (json.dumps(record, sort_keys=True, separators=(",", ":")) for record in records)
    data = ("\n".join(lines) + "\n").encode("utf-8")
    compressed = subprocess.check_output(["gzip", "-n", "-9", "-c"], input=data)
    args.output.write_bytes(compressed)


if __name__ == "__main__":
    main()
