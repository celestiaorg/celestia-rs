"""Consistency-loop regressions for transient real RPC responses."""

import types
import unittest
from unittest.mock import patch

import soak


class ConsistencyTests(unittest.TestCase):
    def setUp(self):
        self.run = soak.Soak.__new__(soak.Soak)
        self.run.ref_checked = 10
        self.run.ref_cache = {
            9: {"hash": "9999", "app_hash": "8888", "proposer": ""},
            10: {"hash": "ABCD", "app_hash": "1234", "proposer": ""},
        }
        self.failures = []
        self.run.fail = self.failures.append

    def node(self, checked=9, top=10):
        return types.SimpleNamespace(
            i=2,
            kind="rs",
            url="http://127.0.0.1:1",
            checked=checked,
            status=lambda: (top, 1, False),
        )

    @staticmethod
    def response(height=10, block_hash="ABCD", app_hash="1234", commit=True):
        return {
            "result": {
                "signed_header": {
                    "header": {"height": str(height), "app_hash": app_hash},
                    "commit": {"block_id": {"hash": block_hash}} if commit else None,
                }
            }
        }

    def test_null_tip_commit_is_retried_without_advancing_or_failing(self):
        node = self.node()
        with patch.object(soak, "get_json", return_value=self.response(commit=False)):
            self.run.check_node(node)
        self.assertEqual(node.checked, 9)
        self.assertEqual(self.failures, [])

        with patch.object(soak, "get_json", return_value=self.response()):
            self.run.check_node(node)
        self.assertEqual(node.checked, 10)
        self.assertEqual(self.failures, [])

    def test_final_null_tip_fails_and_reports_incomplete_coverage(self):
        node = self.node()
        with patch.object(soak, "get_json", return_value=self.response(commit=False)):
            self.run.check_node(node, final=True)
        self.assertEqual(node.checked, 9)
        self.assertTrue(any("malformed commit" in failure for failure in self.failures))
        self.assertTrue(any("coverage stopped" in failure for failure in self.failures))

    def test_null_historical_commit_fails(self):
        node = self.node(checked=8)
        with patch.object(soak, "get_json", return_value=self.response(height=9, commit=False)):
            self.run.check_node(node)
        self.assertEqual(node.checked, 8)
        self.assertTrue(any("height 9: malformed commit" in failure for failure in self.failures))

    def test_malformed_signed_header_fails(self):
        node = self.node()
        response = {"result": {"signed_header": []}}
        with patch.object(soak, "get_json", return_value=response):
            self.run.check_node(node)
        self.assertEqual(node.checked, 9)
        self.assertTrue(any("malformed signed_header" in failure for failure in self.failures))

    def test_missing_signed_header_fields_fail_during_live_pass(self):
        node = self.node()
        with patch.object(soak, "get_json", return_value={"result": {}}):
            self.run.check_node(node)
        self.assertEqual(node.checked, 9)
        self.assertTrue(any("malformed signed_header fields" in failure for failure in self.failures))

    def test_missing_commit_is_not_treated_as_null_tip(self):
        node = self.node()
        response = self.response()
        del response["result"]["signed_header"]["commit"]
        with patch.object(soak, "get_json", return_value=response):
            self.run.check_node(node)
        self.assertEqual(node.checked, 9)
        self.assertTrue(any("missing commit field" in failure for failure in self.failures))

    def test_missing_final_status_fails(self):
        node = self.node()
        node.status = lambda: None
        self.run.check_node(node, final=True)
        self.assertTrue(any("status unavailable" in failure for failure in self.failures))

    def test_malformed_commit_fields_fail(self):
        node = self.node()
        response = self.response()
        response["result"]["signed_header"]["commit"] = {}
        with patch.object(soak, "get_json", return_value=response):
            self.run.check_node(node)
        self.assertEqual(node.checked, 9)
        self.assertTrue(any("malformed commit fields" in failure for failure in self.failures))

    def test_hash_mismatch_still_fails_and_advances(self):
        node = self.node()
        with patch.object(soak, "get_json", return_value=self.response(block_hash="DEAD")):
            self.run.check_node(node)
        self.assertEqual(node.checked, 10)
        self.assertTrue(any("!= ref" in failure for failure in self.failures))


if __name__ == "__main__":
    unittest.main()
