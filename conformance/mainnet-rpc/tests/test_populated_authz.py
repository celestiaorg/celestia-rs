"""Frozen authz sources remain verifiable after dependency changes."""

import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "populated_authz_verify", ROOT / "conformance/mainnet-rpc/populated-authz/verify.py"
)
verify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify)


class PopulatedAuthzProvenance(unittest.TestCase):
    def setUp(self):
        self.metadata = json.loads((verify.OUT / "capture/metadata.json").read_text())

    def test_preserved_sources_verify_after_lockfile_update(self):
        verify.verify_sources(self.metadata)

    def test_changed_recorded_lockfile_hash_is_rejected(self):
        changed = copy.deepcopy(self.metadata)
        changed["source_sha256"]["Cargo.lock"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "source hash mismatch: Cargo.lock"):
            verify.verify_sources(changed)

    def test_missing_lockfile_receipt_is_rejected(self):
        del self.metadata["source_sha256"]["Cargo.lock"]
        with self.assertRaisesRegex(ValueError, "missing lockfile source receipt"):
            verify.verify_sources(self.metadata)

    def test_capture_commit_cannot_be_rebound(self):
        self.metadata["source_commit"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "capture source commit changed"):
            verify.verify_sources(self.metadata)

    def test_unavailable_preserved_source_fails_closed(self):
        with mock.patch.object(
            verify.subprocess, "run",
            side_effect=verify.subprocess.CalledProcessError(128, ["git", "show"]),
        ):
            with self.assertRaisesRegex(ValueError, "captured source unavailable"):
                verify.verify_sources(self.metadata)


if __name__ == "__main__":
    unittest.main()
