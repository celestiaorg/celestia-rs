#!/usr/bin/env python3
"""Verify only the captured ordered-restart Comet matrix, with exact scope."""
import hashlib
import importlib.util
import json
from datetime import datetime, timedelta
from pathlib import Path
import tomllib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DATA = HERE / "ordered-evidence"
PROFILES = ("retain-kv", "prune-kv", "retain-null", "prune-null")
PHASES = ("before-restart", "after-restart", "after-pruning-boundary")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load(path):
    return json.loads(path.read_text())


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def verify():
    manifest = load(DATA / "manifest.json")
    require(manifest["scope"] == "ordered-restart-comet-only", "matrix was promoted beyond captured scope")
    require(set(manifest["profiles"]) == set(PROFILES), "missing profile")
    actual = {str(p.relative_to(DATA)) for p in DATA.rglob("*") if p.is_file() and p.name != "manifest.json"}
    require(set(manifest["artifacts"]) == actual, "incomplete matrix inventory")
    for name, expected in manifest["artifacts"].items():
        path = (DATA / name).resolve()
        require(path.is_relative_to(DATA), "artifact escapes capture")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, f"changed matrix artifact: {name}")
    parser_path = ROOT / "conformance/mainnet-rpc/populated-governance/verify.py"
    require(hashlib.sha256(parser_path.read_bytes()).hexdigest() == manifest["archive_parser_sha256"], "archive parser changed")
    parser = module("matrix_archive", parser_path)
    capture = module("matrix_capture", HERE / "capture.py")
    dumper = load(DATA / "dump-receipt.json")
    require("go1.26.9" in dumper["build_info"] and "v0.42.3" in dumper["build_info"], "wrong archive generator build")
    for name, expected in dumper["source_sha256"].items():
        require(hashlib.sha256((DATA / "dump-sources" / name).read_bytes()).hexdigest() == expected, "changed archive generator source")
    sampling = load(DATA / "first-batch/prune-kv/after-restart.json")
    require(not sampling["cases"] and any("tips differ" in error for error in sampling["errors"]), "original sampling failure was erased")
    total = 0
    for profile in PROFILES:
        batch = DATA / manifest["profiles"][profile]
        directory = batch / profile
        receipts = load(batch / "build-receipts.json")
        for name, expected_module, commit in (
            ("celestia-appd", "github.com/celestiaorg/celestia-app/v10@v10.5.0", "6013e758d5dceb75827ad191c8d739d7c43cfa04"),
            ("cometbft", "github.com/celestiaorg/celestia-core@v0.42.3", "a5537d84a008d566abc503d2ea41e30f2301403d"),
        ):
            oracle = load(batch / f"{name}.build.json")
            require(oracle["module"] == expected_module and oracle["source_commit"] == commit, "wrong oracle source")
            require(oracle["compiler"] == "go version go1.26.9 linux/amd64", "wrong oracle compiler")
        observation = load(batch / "mainnet-observation.json")
        observed = datetime.fromisoformat(observation["observed_at"])
        decoded = {}
        for route in ("status", "abci_info"):
            raw = observation["responses"][route]["body"].encode()
            require(hashlib.sha256(raw).hexdigest() == observation["responses"][route]["sha256"], "unbound mainnet observation")
            decoded[route] = json.loads(raw)["result"]
        require(decoded["status"]["node_info"]["network"] == "celestia" and decoded["abci_info"]["response"]["version"] == "10.5.0", "wrong observed release")
        for source, archived in (
            ("conformance/devnet/rpc-persistence.sh", "capture-source.sh"),
            ("conformance/devnet/lib-mixed.sh", "mixed-source.sh"),
            ("conformance/mainnet-rpc/persistence/capture.py", "capture-source.py"),
        ):
            require(hashlib.sha256((batch / archived).read_bytes()).hexdigest() == receipts["sources"][source], "unbound capture source")
        require((batch / "capture-source.py").read_bytes() == (HERE / "capture.py").read_bytes(), "review changed comparison implementation")
        for node in range(3):
            config = tomllib.loads((directory / f"config-{node}.toml").read_text())
            app = tomllib.loads((directory / f"app-{node}.toml").read_text())
            require(config["tx_index"]["indexer"] == ("null" if profile.endswith("null") else "kv"), "wrong index configuration")
            require(config["storage"]["discard_abci_responses"] is False, "responses were discarded")
            require(app["pruning"] == ("custom" if profile.startswith("prune") else "nothing"), "wrong pruning strategy")
            require(app["min-retain-blocks"] == 0, "unexpected block-store pruning")
            if profile.startswith("prune"):
                require(int(app["pruning-keep-recent"]) == int(app["pruning-interval"]) == 10, "wrong custom pruning boundary")
        frames = parser.archive_frames(directory / "validator-blocks.bin.gz")
        require(len(frames) % 2 == 0, "truncated validator archive")
        headers = {}
        for index in range(0, len(frames), 2):
            header = parser.block_header(frames[index])
            require(header["height"] == index // 2 + 1 and parser.commit_height(frames[index + 1]) == header["height"], "archive height gap")
            headers[header["height"]] = header
        tx = load(directory / "tx-103.json")["txhash"]
        previous_tip = 0
        for phase in PHASES:
            result = load(directory / f"{phase}.json")
            require(timedelta(0) <= datetime.fromisoformat(result["captured_at"]) - observed <= timedelta(hours=24), "stale oracle selection")
            require(not result["errors"] and len(result["cases"]) == 40, f"incomplete {profile}/{phase}")
            require(result["comparison"] == "decoded-json-exact", "comparison policy changed")
            query = next(row for row in result["cases"] if row["method"] == "tx_search")
            sender = json.loads(query["params"]["query"]).removeprefix("message.sender='").removesuffix("'")
            expected = {(method, json.dumps(params, sort_keys=True)) for method, params in capture.cases(result["height"], sender, tx)}
            actual_cases = {(row["method"], json.dumps(row["params"], sort_keys=True)) for row in result["cases"]}
            require(actual_cases == expected and len(actual_cases) == 40, "missing or duplicated request branch")
            require(headers[result["height"] + 1]["app_hash"].upper() == result["matched_app_hash"].upper(), "matched root is not in validator archive")
            for row in result["cases"]:
                require(row["outcome"] == "match", "recorded mismatch")
                for role in ("go", "rs"):
                    require(hashlib.sha256(row[role]["body"].encode()).hexdigest() == row[role]["body_sha256"], "unbound response")
                require(capture.comparable(row["go"]) == capture.comparable(row["rs"]), "unequal response pair")
            statuses = [json.loads(result[role + "_status"]["body"])["result"] for role in ("go", "rs")]
            tips = [int(status["sync_info"]["latest_block_height"]) for status in statuses]
            require(tips[0] == tips[1] and tips[0] > previous_tip, "followers did not advance together")
            require(all(status["node_info"]["network"] == result["chain_id"] for status in statuses), "wrong runtime chain")
            require(headers[result["height"] + 1]["chain_id"] == result["chain_id"], "archive chain differs")
            previous_tip = tips[0]
            if phase == "after-pruning-boundary":
                history = next(row for row in result["cases"] if row["method"] == "abci_query" and row["params"]["height"] == result["height"])
                response = json.loads(history["go"]["body"])["result"]["response"]
                require((response["code"] != 0) == profile.startswith("prune"), "pruning boundary was not exercised")
            total += len(result["cases"])
    require(total == 480, "incomplete paired matrix")
    print("Verified 480 ordered-restart Comet pairs across four pruning/index profiles. Simultaneous restart, other transports, state sync and pre-v10 multiplexer support remain outside this result.")


if __name__ == "__main__":
    verify()
