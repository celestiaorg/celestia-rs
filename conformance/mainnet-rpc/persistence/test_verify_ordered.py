import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("ordered_capture", Path(__file__).with_name("verify_ordered.py"))
ordered = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ordered)


class OrderedCaptureTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.data = Path(temporary.name) / "capture"
        shutil.copytree(ordered.DATA, self.data)
        override = patch.object(ordered, "DATA", self.data)
        override.start()
        self.addCleanup(override.stop)

    def test_complete_scoped_matrix(self):
        ordered.verify()

    def test_missing_profile_cannot_pass(self):
        path = self.data / "manifest.json"
        value = json.loads(path.read_text())
        value["profiles"].pop("prune-null")
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "missing profile"):
            ordered.verify()

    def test_cannot_promote_to_universal_readiness(self):
        path = self.data / "manifest.json"
        value = json.loads(path.read_text())
        value["scope"] = "all-api-ready"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "beyond captured scope"):
            ordered.verify()

    def test_rebound_http_mismatch_still_fails(self):
        name = "first-batch/retain-kv/before-restart.json"
        path = self.data / name
        value = json.loads(path.read_text())
        value["cases"][0]["rs"]["status"] = 503
        path.write_text(json.dumps(value))
        manifest_path = self.data / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        manifest["artifacts"][name] = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "unequal response pair"):
            ordered.verify()


if __name__ == "__main__":
    unittest.main()
