"""Failure injection for the soak's independent RPC client diagnostics."""

import contextlib
import http.server
import io
import json
import threading
import tempfile
import types
import unittest
from unittest.mock import patch

import soak


class Stop:
    """Make workload pacing immediate; failures still stop the real loop."""

    def __init__(self):
        self.stopped = False

    def is_set(self):
        return self.stopped

    def wait(self, _seconds):
        return self.stopped


def harness(url):
    run = soak.Soak.__new__(soak.Soak)
    run.a = types.SimpleNamespace(load_accounts=1, tx_rate=100)
    run.chain_id = "failure-injection"
    run.t0 = soak.now()
    run.max_height = 42
    run.metrics = {"bank": {"ok": 0, "rejected": {}, "errors": 0, "resyncs": 0}}
    run.stop = Stop()
    run.provider = types.SimpleNamespace(enabled=False)
    run.events = io.StringIO()
    run.ev_lock = threading.Lock()
    node = types.SimpleNamespace(i=2, kind="rs", url=url)
    run.tx_node = lambda: node
    run.up_nodes = lambda: [node]
    return run, node


class ClientErrors(unittest.TestCase):
    def test_broadcast_invalid_json_retains_actual_exception_and_request_context(self):
        run, _ = harness("")

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                self.server.request_json = request
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"not-json")
                self.wfile.flush()
                run.stop.stopped = True

            def log_message(self, *_args):
                pass

        with http.server.HTTPServer(("127.0.0.1", 0), Handler) as server:
            run.up_nodes()[0].url = f"http://127.0.0.1:{server.server_port}"
            thread = threading.Thread(target=server.handle_request)
            thread.start()
            key = types.SimpleNamespace(address="celestia1sender")
            try:
                with patch.object(soak.ctx.Key, "from_seed", return_value=key), \
                     patch.object(soak.ctx, "query_account", return_value=(7, 9)), \
                     patch.object(soak.ctx, "sign_tx", return_value=b"signed-public-tx"), \
                     contextlib.redirect_stdout(io.StringIO()):
                    run.bank_loop()
            finally:
                thread.join(timeout=5)
            self.assertFalse(thread.is_alive())
            self.assertEqual(server.request_json["method"], "broadcast_tx_sync")
        self.assertEqual(run.metrics["bank"]["errors"], 1)
        self.assertEqual(run.metrics["bank"]["ok"], 0)
        record = run.metrics["client_errors"]["bank:broadcast_tx_sync"]
        self.assertEqual(record["count"], 1)
        sample = record["first"]
        self.assertEqual(sample["exception_type"], "json.decoder.JSONDecodeError")
        self.assertIn("Expecting value", sample["message"])
        self.assertIn("cosmostx.py", sample["traceback"])
        self.assertEqual((sample["node"], sample["node_kind"]), (2, "rs"))
        self.assertEqual((sample["account_number"], sample["sequence"]), (7, 9))
        self.assertEqual(sample["tx_hash"], soak.hashlib.sha256(b"signed-public-tx").hexdigest().upper())
        event = json.loads(run.events.getvalue())
        self.assertEqual(event["ev"], "client-error")
        self.assertEqual(event["tx_hash"], sample["tx_hash"])
        self.assertNotIn("signed-public-tx", run.events.getvalue())

    def test_repeated_unique_failures_keep_fixed_size_persistable_summary(self):
        run, node = harness("http://127.0.0.1:1")
        with contextlib.redirect_stdout(io.StringIO()):
            for index in range(100):
                run.client_error("bank", "account-resync", RuntimeError(str(index) + "x" * 12000), node)
        record = run.metrics["client_errors"]["bank:account-resync"]
        self.assertEqual(record["count"], 100)
        self.assertTrue(record["first"]["message"].startswith("0x"))
        self.assertTrue(record["last"]["message"].startswith("99x"))
        self.assertEqual(len(record["last"]["message"]), 4096)
        self.assertLessEqual(len(record["last"]["traceback"]), 8192)
        self.assertLess(len(json.dumps(record)), 26000)
        self.assertEqual(len(run.events.getvalue().splitlines()), 100)

    def test_unavailable_reference_is_recorded_and_reports_include_diagnostics(self):
        run, _ = harness("")
        with contextlib.redirect_stdout(io.StringIO()):
            run.client_error("bank", "account-resync", RuntimeError("reference unavailable"), None)
        samples = run.metrics["client_errors"]["bank:account-resync"]
        self.assertIsNone(samples["first"]["node"])
        self.assertIsNone(samples["first"]["endpoint"])
        # Exercise the actual report writer with an otherwise empty network.
        run.blocks = {"txs": 0, "max_bytes": 0, "over_8mb": 0}
        run.proposed = {}
        run.nodes = []
        run.stalls = []
        run.failures = []
        run.log_hits = {}
        run.segment = 1
        for name in ("pfb", "staking", "gov", "authz", "feegrant", "hyperlane", "utf8",
                     "precommit_stop", "sign_refusals", "wal", "disk", "rss"):
            run.metrics[name] = {}
        run.metrics["chaos"] = []
        run.metrics["valset"] = []
        with tempfile.TemporaryDirectory() as directory:
            run.dir = directory
            run.write_report(final=True)
            with open(f"{directory}/report.json") as report:
                self.assertEqual(json.load(report)["metrics"]["client_errors"]["bank:account-resync"], samples)
            with open(f"{directory}/report.md") as report:
                text = report.read()
            self.assertIn("## Client exceptions", text)
            self.assertIn("reference unavailable", text)


if __name__ == "__main__":
    unittest.main()
