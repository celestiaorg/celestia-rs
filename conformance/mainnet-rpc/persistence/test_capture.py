import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("persistence_capture", Path(__file__).with_name("capture.py"))
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class CaptureTest(unittest.TestCase):
    def run_capture(self, mutation=None):
        def fetch(base, method, params):
            if method == "status":
                value = {"result": {"node_info": {"network": "local-test"},
                                    "sync_info": {"latest_block_height": "20"}}}
            elif method == "block":
                value = {"result": {"block": {"header": {"app_hash": "AA"}}}}
            else:
                value = {"result": {"height": "10", "value": ""}}
            if mutation and base == "http://localhost:2":
                mutation(method, params, value)
            return {"status": 200, "content_type": "application/json", "body": json.dumps(value)}

        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary) / "capture.json"
            argv = ["capture.py", "--go", "http://localhost:1", "--rs", "http://localhost:2",
                    "--height", "10", "--sender", "celestia1test", "--tx", "AB" * 32,
                    "--chain-id", "local-test", "--out", str(out)]
            with patch("sys.argv", argv), patch.object(capture, "fetch", side_effect=fetch):
                code = capture.main()
            return code, json.loads(out.read_text())

    def test_matched_capture(self):
        code, report = self.run_capture()
        self.assertEqual(code, 0)
        self.assertEqual(len(report["cases"]), 40)
        self.assertTrue(all(case["outcome"] == "match" for case in report["cases"]))

    def test_wrong_chain_prevents_comparison(self):
        def mutation(method, params, value):
            if method == "status":
                value["result"]["node_info"]["network"] = "wrong"
        code, report = self.run_capture(mutation)
        self.assertEqual(code, 1)
        self.assertIn("wrong chain", report["errors"][0])
        self.assertEqual(report["cases"], [])

    def test_different_tip_prevents_comparison(self):
        def mutation(method, params, value):
            if method == "status":
                value["result"]["sync_info"]["latest_block_height"] = "21"
        code, report = self.run_capture(mutation)
        self.assertEqual(code, 1)
        self.assertIn("tips differ", report["errors"][0])
        self.assertEqual(report["cases"], [])

    def test_different_committed_root_prevents_comparison(self):
        def mutation(method, params, value):
            if method == "block":
                value["result"]["block"]["header"]["app_hash"] = "BB"
        code, report = self.run_capture(mutation)
        self.assertEqual(code, 1)
        self.assertIn("roots differ", report["errors"][0])
        self.assertEqual(report["cases"], [])

    def test_mismatch_is_preserved_and_fails_campaign(self):
        def mutation(method, params, value):
            if method == "abci_query":
                value["result"]["value"] = None
        code, report = self.run_capture(mutation)
        self.assertEqual(code, 1)
        row = next(row for row in report["cases"] if row["method"] == "abci_query")
        self.assertEqual(row["outcome"], "mismatch")
        self.assertIsNone(json.loads(row["rs"]["body"])["result"]["value"])

    def test_network_targets_are_rejected(self):
        for url in ("https://rpc.celestia.pops.one", "http://example.com", "http://user@localhost"):
            with self.subTest(url=url), self.assertRaisesRegex(ValueError, "isolated local"):
                capture.fetch(url, "status", {})


if __name__ == "__main__":
    unittest.main()
