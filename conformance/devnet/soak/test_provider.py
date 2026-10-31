"""Provider harness configuration, accounting and uncertain-admission regressions."""

import json
import copy
import datetime
import hashlib
import pathlib
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch
import tomllib

import provider


class ProviderTests(unittest.TestCase):
    def observation(self):
        now = datetime.datetime(2026, 10, 31, 1, 0, tzinfo=datetime.timezone.utc)
        bodies = {"status": {"result": {"node_info": {"network": "celestia"}}},
                  "abci_info": {"result": {"response": {"version": "10.5.0"}}}}
        observation = {"endpoint": "https://rpc.celestia.pops.one", "observed_at": now.isoformat(),
                       "chain_id": "celestia", "version": "10.5.0", "responses": {}}
        for name, body in bodies.items():
            text = json.dumps(body)
            observation["responses"][name] = {"body": text, "sha256": hashlib.sha256(text.encode()).hexdigest()}
        return observation, now

    def test_mainnet_observation_accepts_fresh_hashed_matching_identity(self):
        observation, now = self.observation()
        self.assertEqual(provider.validate_mainnet_observation(observation, now), observation)

    def test_mainnet_observation_rejects_stale_future_naive_and_raw_abci_only(self):
        observation, now = self.observation()
        for timestamp in ((now - datetime.timedelta(hours=25)).isoformat(),
                          (now + datetime.timedelta(seconds=1)).isoformat(),
                          now.replace(tzinfo=None).isoformat()):
            with self.subTest(timestamp=timestamp):
                invalid = {**observation, "observed_at": timestamp}
                with self.assertRaises(ValueError):
                    provider.validate_mainnet_observation(invalid, now)
        with self.assertRaises(ValueError):
            provider.validate_mainnet_observation({"result": {"response": {"version": "10.5.0"}}}, now)

    def test_mainnet_observation_requires_raw_hashes_and_matching_chain_version(self):
        observation, now = self.observation()
        variants = []
        for field, value in (("chain_id", "arabica-11"), ("version", "10.4.0"),
                             ("endpoint", "http://rpc.celestia.pops.one"), ("endpoint", "https://127.0.0.1:1234")):
            variants.append({**observation, field: value})
        for name in ("status", "abci_info"):
            invalid = copy.deepcopy(observation)
            invalid["responses"][name]["sha256"] = "0" * 64
            variants.append(invalid)
            invalid = copy.deepcopy(observation)
            del invalid["responses"][name]
            variants.append(invalid)
        for name in ("status", "abci_info"):
            invalid = copy.deepcopy(observation)
            body = json.loads(invalid["responses"][name]["body"])
            if name == "status":
                body["result"]["node_info"]["network"] = "arabica-11"
            else:
                body["result"]["response"]["version"] = "10.4.0"
            text = json.dumps(body)
            invalid["responses"][name] = {"body": text, "sha256": hashlib.sha256(text.encode()).hexdigest()}
            variants.append(invalid)
        for index, invalid in enumerate(variants):
            with self.subTest(index=index):
                with self.assertRaises(ValueError):
                    provider.validate_mainnet_observation(invalid, now)

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
