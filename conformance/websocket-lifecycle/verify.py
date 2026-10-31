#!/usr/bin/env python3
"""Verify the WebSocket evidence chain and relational capture invariants."""

from __future__ import annotations

import hashlib
import json
import pathlib
from datetime import datetime, timedelta

ROOT = pathlib.Path(__file__).resolve().parent
EXPECTED_ARTIFACTS = {
    "README.md", "go.mod", "go.sum", "main.go", "main_test.go", "record.sh",
    "verify.py", "testdata/go-v10.5.0.json",
    "testdata/go-v10.5.0-cors-disabled.json",
    "testdata/go-v10.5.0.build-info.txt", "testdata/release.json",
    "testdata/mainnet-status.json", "testdata/mainnet-abci-info.json",
    "testdata/mainnet-observation.json", "testdata/oracle/genesis.json",
    "testdata/oracle/config-cors-enabled.toml",
    "testdata/oracle/config-cors-disabled.toml", "testdata/oracle/app.toml",
}
SUPPORT_FILES = {
    "closure-review.json", "verify-closure.py", "testdata/manifest.json",
}


def load(name: str, root: pathlib.Path = ROOT):
    return json.loads((root / name).read_text())


def tree_inventory(root: pathlib.Path) -> set[str]:
    """Return regular files without following or accepting any symlink."""
    if root.is_symlink() or not root.is_dir():
        raise ValueError("WebSocket evidence root must be a real directory")
    files = set()

    def visit(directory: pathlib.Path) -> None:
        for entry in directory.iterdir():
            name = entry.relative_to(root).as_posix()
            if entry.is_symlink():
                raise ValueError(f"symlinked WebSocket evidence path: {name}")
            if entry.is_dir():
                visit(entry)
            elif entry.is_file():
                files.add(name)
            else:
                raise ValueError(f"non-regular WebSocket evidence path: {name}")

    visit(root)
    return files


def verify_inventory(root: pathlib.Path = ROOT) -> dict[str, str]:
    files = tree_inventory(root)
    expected_files = EXPECTED_ARTIFACTS | SUPPORT_FILES
    if files != expected_files:
        missing = sorted(expected_files - files)
        extra = sorted(files - expected_files)
        raise ValueError(f"WebSocket evidence inventory changed; missing={missing}, extra={extra}")
    manifest = load("testdata/manifest.json", root)
    if set(manifest) != {"schema", "artifacts_sha256"} or manifest["schema"] != 1:
        raise ValueError("unexpected WebSocket evidence manifest schema")
    artifacts = manifest["artifacts_sha256"]
    if set(artifacts) != EXPECTED_ARTIFACTS:
        missing = sorted(EXPECTED_ARTIFACTS - set(artifacts))
        extra = sorted(set(artifacts) - EXPECTED_ARTIFACTS)
        raise ValueError(f"WebSocket manifest inventory changed; missing={missing}, extra={extra}")
    for name, want in artifacts.items():
        got = hashlib.sha256((root / name).read_bytes()).hexdigest()
        if got != want:
            raise ValueError(f"{name}: {got} != {want}")
    return artifacts


def instant(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def main() -> None:
    artifacts = verify_inventory()

    full = load("testdata/go-v10.5.0.json")
    disabled = load("testdata/go-v10.5.0-cors-disabled.json")
    for capture in (full, disabled):
        assert instant(capture["captured_at_start"]) <= instant(capture["captured_at_end"])
        baseline = capture["baseline"]
        assert baseline["celestia_app"] == "v10.5.0"
        assert baseline["celestia_app_commit"] == "6013e758d5dceb75827ad191c8d739d7c43cfa04"
        assert baseline["celestia_core"] == "v0.42.3"
        assert baseline["cosmos_sdk"] == "v0.52.12"
        assert len(capture["upgrade"]) == 3
        for upgrade in capture["upgrade"]:
            assert upgrade["status"] == 101 and upgrade["active_block_seen"]
            assert "Access-Control-Allow-Origin" not in upgrade["headers"]
            assert "Vary" not in upgrade["headers"]
    full_start = instant(full["captured_at_start"])
    full_end = instant(full["captured_at_end"])
    disabled_start = instant(disabled["captured_at_start"])
    assert full_end <= disabled_start <= full_end + timedelta(minutes=10)

    quota = full["quota"]
    assert quota["clients_opened"] > 0 and quota["reclaimed_after_disconnect"]
    per_client = quota["subscriptions_per_client"]
    assert len(per_client) == 8
    assert all("result" in frame for frame in per_client[:5])
    assert "max_subscriptions_per_client 5 reached" in per_client[5]["error"]["data"]
    assert "result" in per_client[6] and "result" in per_client[7]
    assert quota["goroutines_after_cleanup"] <= quota["goroutines_before"] + 10
    assert quota["heap_bytes_after_cleanup"] <= quota["heap_bytes_before"] + 32 * 1024 * 1024

    observation = load("testdata/mainnet-observation.json")
    started = instant(observation["started_at"])
    status_done = instant(observation["status_completed_at"])
    finished = instant(observation["finished_at"])
    assert started <= status_done <= finished
    assert finished <= full_start <= finished + timedelta(minutes=10)
    status = load("testdata/mainnet-status.json")["result"]
    abci = load("testdata/mainnet-abci-info.json")["result"]["response"]
    assert status["node_info"]["network"] == "celestia"
    assert status["node_info"]["version"] == "0.38.17"
    assert status["node_info"]["protocol_version"]["app"] == "10"
    assert abci["version"] == "10.5.0" and abci["app_version"] == "10"
    status_height = int(status["sync_info"]["latest_block_height"])
    abci_height = int(abci["last_block_height"])
    assert status_height <= abci_height <= status_height + 5
    assert instant(status["sync_info"]["latest_block_time"]) <= finished

    release = load("testdata/release.json")
    commit = "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    assert release["latest_release"] == "v10.5.0"
    assert release["target_commitish"] == commit == release["tag_object"]
    release_verified = instant(release["verified_at"])
    assert release_verified <= full_start <= release_verified + timedelta(hours=1)
    build = (ROOT / "testdata/go-v10.5.0.build-info.txt").read_text()
    assert "github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd" in build
    assert "eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38" in build
    genesis = load("testdata/oracle/genesis.json")
    assert genesis["chain_id"] == "ws190-105-1"
    enabled = (ROOT / "testdata/oracle/config-cors-enabled.toml").read_text()
    disabled_cfg = (ROOT / "testdata/oracle/config-cors-disabled.toml").read_text()
    for text in (enabled, disabled_cfg):
        assert "max_subscription_clients = 3" in text
        assert "max_subscriptions_per_client = 5" in text
        assert "experimental_subscription_buffer_size = 100" in text
        assert "experimental_websocket_write_buffer_size = 100" in text
        assert "experimental_close_on_slow_client = true" in text
        assert "max_body_bytes = 1000000" in text
    assert 'cors_allowed_origins = ["https://allowed.example"]' in enabled
    assert "cors_allowed_origins = []" in disabled_cfg
    print(f"verified {len(artifacts)} hashed artifacts and WebSocket lifecycle invariants")


if __name__ == "__main__":
    main()
