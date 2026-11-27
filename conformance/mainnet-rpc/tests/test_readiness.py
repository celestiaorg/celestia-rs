"""The readiness record binds frozen evidence, not files that ordinary pull requests edit."""
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("rpc_readiness_test", HERE / "readiness.py")
readiness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(readiness)

LIVING = {
    ".github/workflows/ci.yml",
    "conformance/mainnet-rpc/READINESS.md",
    "conformance/mainnet-rpc/readiness.py",
    "conformance/mainnet-rpc/scope-audit.json",
    "conformance/mainnet-rpc/scope_audit.py",
}


class Readiness(unittest.TestCase):
    def test_living_files_are_not_hash_bound(self):
        bindings = json.loads((HERE / "readiness.json").read_text())["bindings"]
        self.assertEqual(LIVING & set(bindings), set())
        self.assertTrue(bindings)
        # Today every frozen binding is a captured log or a manifest. Widen
        # this list when another kind of frozen artifact is bound.
        for name in bindings:
            self.assertTrue(name.endswith((".log", "manifest.json")), name)

    def test_record_verifies(self):
        readiness.verify()


if __name__ == "__main__":
    unittest.main()
