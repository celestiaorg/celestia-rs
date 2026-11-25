"""A checkpoint outside the replay windows is verified against a given app hash."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
import zlib

MODULE = Path(__file__).resolve().parents[1] / "verify-replay-checkpoint.py"
SPEC = importlib.util.spec_from_file_location("verify_replay_checkpoint_test", MODULE)
verify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify)


def field(number, value):
    if isinstance(value, int):
        return verify.put_uvarint(number << 3) + verify.put_uvarint(value)
    return verify.put_uvarint(number << 3 | 2) + verify.put_uvarint(len(value)) + value


def item(body):
    return verify.put_uvarint(len(body)) + body


def snapshot(root, height, snapshot_format):
    """One store `acc` with a single leaf, as a one-chunk SDK snapshot."""
    store = item(field(1, field(1, b"acc")))
    leaf = item(field(2, field(1, b"k") + field(2, b"v") + field(3, height)))
    directory = Path(root) / str(height) / str(snapshot_format)
    directory.mkdir(parents=True)
    (directory / "0").write_bytes(zlib.compress(store + leaf))
    return directory


class VerifyAgainstGivenAppHash(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        # 2,371,489 precedes the v1 to v2 upgrade; no replay window starts after it.
        self.directory = snapshot(self.tmp.name, 2371489, 2)
        roots = verify.store_roots(self.directory)
        roots.update({name: "" for name in verify.FORMAT_2_MEMORY_STORES})
        self.app_hash = verify.commit_hash(roots)

    def run_main(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            verify.main([str(self.directory), *args])
        return out.getvalue()

    def test_matching_app_hash_verifies_a_height_without_a_window(self):
        output = self.run_main("--app-hash", self.app_hash)
        self.assertIn(f"checkpoint at 2371489: app hash {self.app_hash}", output)

    def test_lowercase_app_hash_is_accepted(self):
        self.assertIn("Verified", self.run_main("--app-hash", self.app_hash.lower()))

    def test_different_app_hash_fails(self):
        wrong = ("0" if self.app_hash[0] != "0" else "1") + self.app_hash[1:]
        with self.assertRaisesRegex(SystemExit, "app hash at 2371489"):
            self.run_main("--app-hash", wrong)

    def test_malformed_app_hash_is_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_main("--app-hash", "XYZ")

    def test_height_without_window_still_needs_an_app_hash(self):
        with self.assertRaisesRegex(SystemExit, "no replay window starts at 2371490"):
            self.run_main()


if __name__ == "__main__":
    unittest.main()
