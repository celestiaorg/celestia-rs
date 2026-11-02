import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("restart_reproduction", Path(__file__).with_name("verify_reproduction.py"))
reproduction = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reproduction)


class ReproductionTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.data = Path(temporary.name) / "capture"
        shutil.copytree(reproduction.DATA, self.data)
        override = patch.object(reproduction, "DATA", self.data)
        override.start()
        self.addCleanup(override.stop)

    def test_original_failed_run_is_preserved(self):
        reproduction.verify()

    def test_cannot_relabel_failed_run(self):
        path = self.data / "manifest.json"
        value = json.loads(path.read_text())
        value["outcome"] = "parity-pass"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "relabelled"):
            reproduction.verify()

    def test_tampered_response_is_rejected(self):
        path = self.data / "retain-kv/before-restart.json"
        value = json.loads(path.read_text())
        value["cases"].pop()
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "changed reproduction"):
            reproduction.verify()


if __name__ == "__main__":
    unittest.main()
