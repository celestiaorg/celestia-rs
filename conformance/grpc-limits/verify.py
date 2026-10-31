#!/usr/bin/env python3
"""Validate frozen gRPC limit evidence and its provenance hashes."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(path: Path) -> list[dict[str, object]]:
    data = json.loads(path.read_text())
    assert isinstance(data, list) and data, f"{path}: expected nonempty result list"
    return data


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture", nargs="?", type=Path, default=Path(__file__).parent / "v10.5.0")
    capture = parser.parse_args().capture.resolve()
    meta = json.loads((capture / "meta.json").read_text())

    build = meta["oracle_build"]
    assert build["module"] == "github.com/celestiaorg/celestia-app/v10@v10.5.0"
    assert build["source_commit"] == "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    assert build["build_tags"] == ["ledger"]
    for name, expected in meta["sha256"].items():
        assert digest(capture / name) == expected, f"artifact hash mismatch: {name}"
    for name, expected in meta["capture_source_sha256"].items():
        assert digest(ROOT / name) == expected, f"capture source hash mismatch: {name}"

    assert json.loads((capture / "mainnet-abci-info.json").read_text())["result"]["response"]["version"] == "10.5.0"
    assert json.loads((capture / "mainnet-status.json").read_text())["result"]["node_info"]["network"] == "celestia"

    json_results = list(capture.glob("go-*.json"))
    json_results += list(capture.glob("go-*.json.gz"))
    for path in json_results:
        result_rows = json.loads(
            gzip.decompress(path.read_bytes()) if path.suffix == ".gz" else path.read_bytes()
        )
        if not isinstance(result_rows, list):
            continue
        assert result_rows, f"{path}: no results"
        for row in result_rows:
            message = str(row.get("message", "")).lower()
            assert "connection refused" not in message, f"{path}: listener readiness failure"

    send_five = rows(capture / "go-sdk-send-5.json")
    by_name = {str(row["name"]): row for row in send_five}
    for size in (63, 64):
        row = by_name[f"bytes-{size}-invalid-height-false"]
        assert row["code"] == 8
        assert row["message"] == "grpc: trying to send message larger than max (6 vs. 5)"
        assert row["headers"] is not None and row["trailers"] == {}
    oversized = by_name["bytes-65-invalid-height-false"]
    assert oversized["code"] == 8
    assert oversized["message"] == "grpc: received message larger than max (65 vs. 64)"
    assert oversized["headers"] is None and oversized["trailers"] is not None


if __name__ == "__main__":
    main()
