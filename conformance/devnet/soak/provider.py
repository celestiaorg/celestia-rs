"""Opt-in, loopback-only transport traffic for the mixed-node soak.

The Go client emits bounded diagnostics; these are availability exercises, not
byte-for-byte Go/Rust parity evidence or proof-validity assertions.
"""

import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

TRANSPORTS = ("jsonrpc", "uri", "websocket", "cosmos-grpc", "rest", "core-grpc")
OPERATIONS = (
    "comet-http-status", "comet-jsonrpc-history", "comet-jsonrpc-proof",
    "comet-ws-subscribe", "comet-ws-slow-reconnect", "core-grpc-status",
    "core-grpc-history", "core-grpc-slow-subscribe",
    "cosmos-grpc-pagination-history", "cosmos-grpc-block",
    "rest-pagination-history", "rest-node-info",
)
DISABLED_SCENARIOS = [
    "TLS and reverse proxies", "external provider client SDKs",
    "proof cryptographic validity", "multi-page cursor traversal",
    "all supported message families through every transport",
    "subscription quota saturation", "unbounded slow readers",
    "exact Go/Rust response equivalence at matched state",
]


def _section_value(text, section, key, value):
    parts = re.split(r"(?m)^(\[[^\n]+\])\s*$", text)
    for i in range(1, len(parts), 2):
        if parts[i] != f"[{section}]":
            continue
        pattern = rf"(?m)^{re.escape(key)}\s*=.*$"
        if not re.search(pattern, parts[i + 1]):
            parts[i + 1] = f"\n{key} = {value}\n" + parts[i + 1]
        else:
            parts[i + 1] = re.sub(pattern, f"{key} = {value}", parts[i + 1])
        return "".join(parts)
    raise ValueError(f"missing [{section}] section")


def endpoints(offset, index):
    delta = offset + 10 * index
    ports = {"rpc": 26657 + delta, "core": 26658 + delta,
             "sdk": 9090 + delta, "rest": 1317 + delta}
    if any(not 1 <= port <= 65535 for port in ports.values()):
        raise ValueError("provider listener port outside 1..65535")
    return {name: ("http://" if name in ("rpc", "rest") else "") + f"127.0.0.1:{port}"
            for name, port in ports.items()}


def configure_network(directory, offset):
    """Set explicit listener values after make_net shifted the generated config."""
    for home in sorted(pathlib.Path(directory).glob("n[0-9]*")):
        if not (home / "kind").is_file():
            continue
        index = int(home.name[1:])
        values = endpoints(offset, index)
        config = home / "config/config.toml"
        text = config.read_text()
        text = _section_value(text, "rpc", "laddr", json.dumps(values["rpc"].replace("http://", "tcp://")))
        text = _section_value(text, "rpc", "grpc_laddr", json.dumps("tcp://" + values["core"]))
        config.write_text(text)
        app = home / "config/app.toml"
        text = app.read_text()
        for section, address in (("api", values["rest"].replace("http://", "tcp://")), ("grpc", values["sdk"])):
            text = _section_value(text, section, "enable", "true")
            text = _section_value(text, section, "address", json.dumps(address))
        # The grpc-web listener is outside this workload and would add a listener
        # whose coverage could be mistaken for native gRPC support.
        if "[grpc-web]" in text:
            text = _section_value(text, "grpc-web", "enable", "false")
        app.write_text(text)
    return {home.name: endpoints(offset, int(home.name[1:]))
            for home in pathlib.Path(directory).glob("n[0-9]*") if (home / "kind").is_file()}


def provenance(directory, rust_binary, go_binary, client, observation):
    """Capture public artifacts only; node/keyring databases never enter this file."""
    root = pathlib.Path(directory)
    observation = pathlib.Path(observation)
    raw = json.loads(observation.read_text())
    if "result" not in raw or raw["result"].get("response", {}).get("version") != "10.5.0":
        raise ValueError("this soak oracle is pinned to independently observed celestia-app 10.5.0")
    binaries = {}
    for label, binary in (("celestia_rs", rust_binary), ("celestia_app", go_binary), ("provider_client", client)):
        path = pathlib.Path(binary).resolve()
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        binaries[label] = {"path": str(path), "sha256": digest}
    info = subprocess.check_output(["go", "version", "-m", go_binary, client], text=True)
    version = subprocess.check_output([rust_binary, "version", "--long"], text=True)
    stamp = pathlib.Path(go_binary + ".module").read_text().strip()
    if stamp != "github.com/celestiaorg/celestia-app/v10@v10.5.0":
        raise ValueError("Go binary lacks successful current-release build stamp")
    source = subprocess.check_output(["git", "-C", str(pathlib.Path(__file__).resolve().parents[3]), "rev-parse", "HEAD"], text=True).strip()
    document = {
        "captured_at_unix": time.time(), "source_commit": source,
        "binaries": binaries, "go_build_info": info, "rs_version": version,
        "go_source_module": stamp, "mainnet_observation": raw,
        "mainnet_observation_file_mtime_unix": observation.stat().st_mtime,
        "genesis_sha256": hashlib.sha256((root / "n0/config/genesis.json").read_bytes()).hexdigest(),
        "coverage_limitations": DISABLED_SCENARIOS,
        "capture_sources_sha256": {
            str(path.relative_to(pathlib.Path(__file__).resolve().parents[3])): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [pathlib.Path(__file__).resolve(), pathlib.Path(__file__).with_name("soak.py"),
                         pathlib.Path(__file__).resolve().parents[1] / "soak.sh",
                         pathlib.Path(__file__).resolve().parents[2] / "provider-client/main.go",
                         pathlib.Path(__file__).resolve().parents[2] / "provider-client/go.mod",
                         pathlib.Path(__file__).resolve().parents[2] / "provider-client/go.sum"]
        },
    }
    recipe = pathlib.Path(go_binary + ".build.json")
    if not recipe.is_file():
        raise ValueError("provider soak requires the oracle builder's .build.json recipe, not a module stamp alone")
    recipe = json.loads(recipe.read_text())
    if (recipe.get("binary_sha256") != binaries["celestia_app"]["sha256"]
            or recipe.get("module") != stamp
            or recipe.get("source_commit") != "6013e758d5dceb75827ad191c8d739d7c43cfa04"
            or "ledger" not in recipe.get("build_tags", [])):
        raise ValueError("provider oracle recipe does not match pinned source, ledger build, and binary content")
    document["go_build_recipe"] = recipe
    (root / "provider-provenance.json").write_text(json.dumps(document, indent=2) + "\n")


class ProviderError(RuntimeError):
    """A client failure with an admission outcome that must not be guessed."""


class Workload:
    def __init__(self, run):
        self.run = run
        self.enabled = os.environ.get("SOAK_PROVIDER_APIS") == "1"
        self.client = os.environ.get("SOAK_PROVIDER_CLIENT", "")
        self.blocked = {}
        if self.enabled and not os.path.isfile(self.client):
            raise ValueError("SOAK_PROVIDER_CLIENT must name the compiled provider-client")
        run.metrics.setdefault("provider", {"enabled": self.enabled, "operations": {}, "disabled_scenarios": DISABLED_SCENARIOS})
        if run.metrics["provider"]["enabled"] != self.enabled:
            raise ValueError("resuming a soak must preserve SOAK_PROVIDER_APIS")
        if self.enabled:
            for key in ("included", "failed_exec", "lost"):
                run.metrics["bank"].setdefault(key, 0)

    def command(self, nd):
        command = [self.client]
        for name, value in endpoints(self.run.off, nd.i).items():
            command += ["--" + name, value]
        return command

    def count(self, nd, operation, outcome):
        key = f"n{nd.i}:{operation}"
        record = self.run.metrics["provider"]["operations"].setdefault(
            key, {"requested": 0, "completed": 0, "errors": 0, "admission_unknown": 0})
        record[outcome] = record.get(outcome, 0) + 1

    def broadcast(self, nd, tx, number):
        transport = TRANSPORTS[number % len(TRANSPORTS)]
        operation = "broadcast:" + transport
        self.count(nd, operation, "requested")
        try:
            out = subprocess.run(self.command(nd) + ["--mode", "broadcast", "--transport", transport],
                                 input=tx, capture_output=True, timeout=15, check=True)
            result = json.loads(out.stdout)
        except Exception:
            self.count(nd, operation, "errors")
            self.count(nd, operation, "admission_unknown")
            raise
        if result.get("error"):
            self.count(nd, operation, "errors")
            if result.get("admission_unknown"):
                self.count(nd, operation, "admission_unknown")
            raise ProviderError(json.dumps({"transport": transport, **result}))
        self.count(nd, operation, "completed")
        self.count(nd, operation, "accepted" if result["code"] == 0 else "rejected")
        if result.get("tx_result_code"):
            self.count(nd, operation, "execution_failed")
        self.run.event("provider-broadcast", node=nd.i, node_kind=nd.kind, transport=transport, **result)
        return result

    def block(self, address, sequence):
        # A timed-out core BroadcastTx may already have been admitted. Do not
        # guess, advance the local sequence, or resubmit until committed state
        # proves advancement. Other workload accounts continue independently.
        self.blocked[address] = sequence

    def can_send(self, nd, address, account):
        if address not in self.blocked:
            return True
        try:
            number, sequence = __import__("cosmostx").query_account(nd.url, address)
            if sequence > self.blocked[address]:
                account[0], account[1] = number, sequence
                del self.blocked[address]
                return True
        except Exception as error:
            self.run.client_error("provider", "uncertain-account-query", error, nd, address=address)
        return False

    def loop(self):
        address = __import__("cosmostx").Key.from_seed("soak-load-0").address
        while not self.run.stop.is_set():
            for nd in self.run.up_nodes():
                if self.run.stop.is_set():
                    return
                height = max(1, (nd.height() or 1) - 2)
                for operation in OPERATIONS:
                    self.count(nd, operation, "requested")
                try:
                    out = subprocess.run(self.command(nd) + ["--height", str(height), "--address", address],
                                         capture_output=True, text=True, timeout=20, check=True)
                    samples = json.loads(out.stdout)
                    if sorted(s["operation"] for s in samples) != sorted(OPERATIONS):
                        raise ProviderError("provider client returned incomplete or duplicate operation results")
                    for sample in samples:
                        self.count(nd, sample["operation"], "completed" if sample["ok"] else "errors")
                        self.run.event("provider-result", node=nd.i, node_kind=nd.kind, query_height=height, **sample)
                        if not sample["ok"]:
                            self.run.client_error("provider", sample["operation"], ProviderError(json.dumps(sample)), nd,
                                                  query_height=height, disruption_observed=not nd.expected_up or nd.sync_pending is not None)
                except Exception as error:
                    for operation in OPERATIONS:
                        self.count(nd, operation, "errors")
                    self.run.client_error("provider", "probe-process", error, nd, query_height=height)
            self.run.stop.wait(5)

    def finalize(self):
        """A zero-traffic or failed transport cannot be presented as a passing soak."""
        if not self.enabled:
            return
        if self.blocked:
            self.run.fail(f"{len(self.blocked)} provider workload accounts have unresolved admission outcomes")
        for nd in self.run.nodes:
            for operation in OPERATIONS + tuple("broadcast:" + t for t in TRANSPORTS):
                metrics = self.run.metrics["provider"]["operations"].get(f"n{nd.i}:{operation}", {})
                if not metrics.get("completed"):
                    self.run.fail(f"provider operation never completed on n{nd.i}: {operation}")
        errors = sum(row["errors"] for row in self.run.metrics["provider"]["operations"].values())
        if errors:
            self.run.fail(f"provider operations recorded {errors} errors; inspect client_errors/events.log (not attributed to node faults)")


if __name__ == "__main__":
    if sys.argv[1] == "configure":
        pathlib.Path(sys.argv[2], "provider-endpoints.json").write_text(json.dumps(configure_network(sys.argv[2], int(sys.argv[3])), indent=2) + "\n")
    elif sys.argv[1] == "provenance":
        provenance(*sys.argv[2:])
    else:
        raise SystemExit("expected configure or provenance")
