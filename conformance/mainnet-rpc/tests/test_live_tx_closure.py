"""The closing #187 paired live-transaction evidence must remain fail closed."""

import importlib.util
import hashlib
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "conformance/mainnet-rpc/live-tx-closure"
CAPTURE = SOURCE / "v10.5.0/2026-10-31-d2e72cb"
MUTATION_SUITE_SHA256 = "d483cfee46283047578a7e4402fa98bdc11237bc450c393f1b0c8aaf31ed645f"
SPEC = importlib.util.spec_from_file_location("live_tx_closure_verify", SOURCE / "verify.py")
sys.path.insert(0, str(SOURCE))
verify = importlib.util.module_from_spec(SPEC)
OLD_DONT_WRITE_BYTECODE = sys.dont_write_bytecode
try:
    sys.dont_write_bytecode = True
    SPEC.loader.exec_module(verify)
finally:
    sys.dont_write_bytecode = OLD_DONT_WRITE_BYTECODE
    sys.path.pop(0)


class LiveTransactionClosureEvidence(unittest.TestCase):
    def test_complete_capture_passes(self):
        verify.verify(CAPTURE)

    def test_semantic_mutation_suite_passes(self):
        self.assertEqual(
            hashlib.sha256((SOURCE / "test_verify.py").read_bytes()).hexdigest(),
            MUTATION_SUITE_SHA256,
        )
        subprocess.run(
            [sys.executable, "-m", "unittest", str(SOURCE / "test_verify.py")],
            cwd=ROOT,
            check=True,
        )


if __name__ == "__main__":
    unittest.main()
