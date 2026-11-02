"""Historical populated-governance sources remain bound to their capture tree."""

import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[3]
MODULE = ROOT / "conformance/mainnet-rpc/populated-governance/verify.py"
METADATA = ROOT / "conformance/fixtures/devnet/v105-populated-governance/capture/metadata.json"
SPEC = importlib.util.spec_from_file_location("populated_governance_verify", MODULE)
verify = importlib.util.module_from_spec(SPEC)
OLD_DONT_WRITE_BYTECODE = sys.dont_write_bytecode
try:
    sys.dont_write_bytecode = True
    SPEC.loader.exec_module(verify)
finally:
    sys.dont_write_bytecode = OLD_DONT_WRITE_BYTECODE


class PopulatedGovernanceProvenance(unittest.TestCase):
    def setUp(self):
        self.metadata = json.loads(METADATA.read_text())

    def test_recorded_sources_match_capture_commit(self):
        verify.verify_hashes(self.metadata)

    def test_wrong_recorded_source_hash_fails(self):
        changed = copy.deepcopy(self.metadata)
        changed["source_sha256"]["crates/tier-4-runtime/api-server/src/rest.rs"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "source hash mismatch"):
            verify.verify_hashes(changed)

    def test_unavailable_capture_object_fails_closed(self):
        with mock.patch.object(
            verify.subprocess,
            "run",
            side_effect=verify.subprocess.CalledProcessError(128, ["git", "show"]),
        ):
            with self.assertRaisesRegex(ValueError, "captured source unavailable"):
                verify.verify_hashes(self.metadata)

    def test_missing_capture_path_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "captured source unavailable"):
            verify.captured_source_digest("missing/capture-source")


if __name__ == "__main__":
    unittest.main()
