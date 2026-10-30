"""Provider harness configuration, accounting and uncertain-admission regressions."""

import json
import pathlib
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch
import tomllib

import provider


class ProviderTests(unittest.TestCase):
    def test_configures_unique_loopback_sockets_without_changing_p2p(self):
        with tempfile.TemporaryDirectory() as directory:
            for i in range(2):
                home = pathlib.Path(directory, f"n{i}")
                (home / "config").mkdir(parents=True)
                (home / "kind").write_text("go" if i == 0 else "rs")
                (home / "config/config.toml").write_text('[p2p]\nladdr = "tcp://0.0.0.0:26656"\n[rpc]\nladdr = "tcp://0.0.0.0:26657"\ngrpc_laddr = ""\n')
                (home / "config/app.toml").write_text('[api]\nenable = false\naddress = "tcp://localhost:1317"\n[grpc]\nenable = false\naddress = "localhost:9090"\n[grpc-web]\nenable = true\n')
            values = provider.configure_network(directory, 1000)
            self.assertEqual(len({v for node in values.values() for v in node.values()}), 8)
            for i in range(2):
                home = pathlib.Path(directory, f"n{i}")
                config = tomllib.loads((home / "config/config.toml").read_text())
                app = tomllib.loads((home / "config/app.toml").read_text())
                self.assertEqual(config["p2p"]["laddr"], "tcp://0.0.0.0:26656")
                self.assertEqual(config["rpc"]["grpc_laddr"], f"tcp://127.0.0.1:{27658 + 10*i}")
                self.assertTrue(app["api"]["enable"])
                self.assertTrue(app["grpc"]["enable"])
                self.assertFalse(app["grpc-web"]["enable"])
                self.assertEqual(app["grpc"]["address"], f"127.0.0.1:{10090 + 10*i}")

    def workload(self):
        run = types.SimpleNamespace(off=1000, metrics={"bank": {}}, failures=[], nodes=[types.SimpleNamespace(i=0)])
        run.fail = run.failures.append
        run.event = lambda *_args, **_kwargs: None
        run.client_error = lambda *_args, **_kwargs: None
        with patch.dict("os.environ", {"SOAK_PROVIDER_APIS": "1", "SOAK_PROVIDER_CLIENT": __file__}):
            return provider.Workload(run)

    def test_timeout_is_counted_and_account_does_not_retry_unknown_admission(self):
        workload = self.workload()
        nd = types.SimpleNamespace(i=0, url="http://127.0.0.1:27657")
        with patch.object(subprocess, "run", side_effect=subprocess.TimeoutExpired("provider-client", 15)):
            with self.assertRaises(subprocess.TimeoutExpired):
                workload.broadcast(nd, b"tx", 5)
        stats = workload.run.metrics["provider"]["operations"]["n0:broadcast:core-grpc"]
        self.assertEqual(stats, {"requested": 1, "completed": 0, "errors": 1, "admission_unknown": 1})
        workload.block("address", 9)
        account = [3, 9]
        with patch("cosmostx.query_account", return_value=(3, 9)):
            self.assertFalse(workload.can_send(nd, "address", account))
        with patch("cosmostx.query_account", return_value=(3, 10)):
            self.assertTrue(workload.can_send(nd, "address", account))
        self.assertEqual(account, [3, 10])

    def test_checktx_rejection_is_completed_transport_not_client_error(self):
        workload = self.workload()
        nd = types.SimpleNamespace(i=0, kind="rs")
        response = types.SimpleNamespace(stdout=json.dumps({"code": 32, "log": "sequence mismatch", "hash": "AB"}).encode())
        with patch.object(subprocess, "run", return_value=response):
            self.assertEqual(workload.broadcast(nd, b"tx", 0)["code"], 32)
        stats = workload.run.metrics["provider"]["operations"]["n0:broadcast:jsonrpc"]
        self.assertEqual(stats["completed"], 1)
        self.assertEqual(stats["errors"], 0)

    def test_zero_or_failed_traffic_cannot_pass_final_gate(self):
        workload = self.workload()
        workload.finalize()
        self.assertEqual(len(workload.run.failures), len(provider.OPERATIONS) + len(provider.TRANSPORTS))
        self.assertTrue(all("never completed" in error for error in workload.run.failures))


if __name__ == "__main__":
    unittest.main()
