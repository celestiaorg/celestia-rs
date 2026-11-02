"""The closing #187 paired live-transaction evidence must remain fail closed."""

import importlib.util
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
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


class LockfileProvenance(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repository = Path(temporary.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Evidence test")
        self.git("config", "user.email", "evidence@example.invalid")
        self.lockfile = self.repository / "Cargo.lock"
        self.lockfile.write_text("recorded dependencies\n")
        self.digest = hashlib.sha256(self.lockfile.read_bytes()).hexdigest()
        self.git("add", "Cargo.lock")
        self.git("-c", "commit.gpgsign=false", "commit", "-qm", "Recorded build")
        self.commit = self.git("rev-parse", "HEAD").strip()
        self.lockfile.write_text("updated dependencies\n")

    def git(self, *arguments):
        return subprocess.run(
            ["git", *arguments], cwd=self.repository, check=True,
            capture_output=True, text=True,
        ).stdout

    def test_dependency_update_preserves_historical_evidence(self):
        verify.verify_lockfile(self.repository, self.commit, self.digest)

    def test_rewritten_receipt_hash_is_rejected(self):
        with self.assertRaisesRegex(verify.VerificationError, "recorded Cargo.lock hash differs"):
            verify.verify_lockfile(self.repository, self.commit, "0" * 64)

    def test_missing_recorded_commit_fails_closed(self):
        with self.assertRaisesRegex(verify.VerificationError, "recorded Cargo.lock unavailable"):
            verify.verify_lockfile(self.repository, "0" * 40, self.digest)

    def test_unchanged_lockfile_needs_no_historical_object(self):
        current = hashlib.sha256(self.lockfile.read_bytes()).hexdigest()
        verify.verify_lockfile(self.repository, "0" * 40, current)


if __name__ == "__main__":
    unittest.main()
