"""Offline checks for the operator-facing mainnet replay commands."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]


class MainnetWrappers(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.home = Path(temp.name)
        (self.home / "config").mkdir()
        (self.home / "config/genesis.json").touch()
        (self.home / "snap/5/3").mkdir(parents=True)
        self.binary = self.home / "celestia-rs"
        self.binary.write_text("""#!/bin/bash
# Simulate a home configured for JSON and a caller suppressing INFO.
case " $* " in *' --log_format plain '*) format=plain;; *) format=json;; esac
echo "$RUST_LOG $format" > "$CELESTIA_RS_HOME/settings"
if [[ $RUST_LOG = info && $format = plain ]]; then
    printf '%s\\n' "$MOCK_RECORD"
fi
exit "$MOCK_STATUS"
""")
        self.binary.chmod(0o755)

    def invoke(self, mode, stop, record="", status=0):
        env = dict(os.environ, RUST_LOG="warn", CELESTIA_RS_BIN=str(self.binary),
                   CELESTIA_RS_HOME=str(self.home), ARCHIVE_PEERS="peer",
                   RPC="http://localhost", MOCK_RECORD=record, MOCK_STATUS=str(status))
        if mode == "follow":
            args = [SCRIPTS / "mainnet-snapshot-follow.sh", self.home / "snap", "5", stop]
        else:
            args = [SCRIPTS / "mainnet-genesis-blocksync.sh", stop]
        return subprocess.run(["bash", *args], env=env, capture_output=True, text=True)

    def test_follow(self):
        for record, expected in [
            ("stop height reached height=8 verified_header_height=9", 0),
            ("follow finished first=6 last=8 blocks=3", 1),
            ("stop height reached height=8 verified_header_height=8", 1),
        ]:
            with self.subTest(record=record):
                result = self.invoke("follow", "008", "INFO celestia_rs::follow: " + record)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual((self.home / "settings").read_text().strip(), "info plain")

    def test_block_sync(self):
        for record, expected, height in [
            ("block sync finished height=8 blocks_synced=3 reason=Stopped", 0, 8),
            ("stop height reached height=8", 0, 8),
            ("stop height reached height=10", 0, 10),
            ("block sync finished height=7 blocks_synced=2 reason=Stopped", 1, None),
            ("stop height reached height=7", 1, None),
            ("block sync finished height=8 blocks_synced=3 reason=CaughtUp", 1, None),
            ("", 1, None),
        ]:
            with self.subTest(record=record):
                result = self.invoke("p2p", "008", "2026-11-28T00:00:00Z  INFO celestia_rs::node: " + record)
                self.assertEqual(result.returncode, expected, result.stderr)
                if height:
                    self.assertIn(f"its header commits the app hash for height {height - 1}", result.stdout)

    def test_unbounded_runs_and_process_errors(self):
        for mode in ("follow", "p2p"):
            with self.subTest(mode=mode):
                self.assertEqual(self.invoke(mode, "0").returncode, 0)
                self.assertEqual((self.home / "settings").read_text().strip(), "warn json")
                self.assertEqual(self.invoke(mode, "8", status=13).returncode, 13)


if __name__ == "__main__":
    unittest.main()
