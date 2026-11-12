#!/usr/bin/env python3
"""Collect read-only, exact-process validator evidence on macOS.

One process observes one node. A restart or executable change ends the run.
Receipts retain failures, independent tip samples, execution log lines,
transaction comparisons and independent commit participation. They do not
verify reference signatures cryptographically or certify qualification.
"""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from urllib.request import urlopen


def utc():
    return datetime.now(timezone.utc).isoformat()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def rpc(endpoint, route):
    with urlopen(endpoint.rstrip("/") + route, timeout=8) as response:
        data = json.load(response)
    if not isinstance(data, dict) or data.get("error") is not None:
        raise ValueError("RPC error response")
    return data["result"]


def results(value, height):
    if int(value["height"]) != height:
        raise ValueError("result height mismatch")
    rows = value.get("txs_results")
    if rows is None:
        rows = []
    if not isinstance(rows, list):
        raise ValueError("invalid transaction result list")
    normalized = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("invalid transaction result")
        item = {}
        for field, lower, upper in (("code", 0, 2**32 - 1),
                                    ("gas_wanted", -(2**63), 2**63 - 1),
                                    ("gas_used", -(2**63), 2**63 - 1)):
            number = row.get(field)
            if number is None:
                number = 0
            if type(number) not in (int, str) or not re.fullmatch(r"-?(0|[1-9][0-9]*)", str(number)):
                raise ValueError("invalid " + field)
            number = int(number)
            if not lower <= number <= upper:
                raise ValueError("out of range " + field)
            item[field] = str(number)
        data = row.get("data")
        if data is None:
            data = ""
        if not isinstance(data, str):
            raise ValueError("invalid result data")
        base64.b64decode(data, validate=True)
        item["data"] = data
        normalized.append(item)
    return normalized


def participation(commit, address):
    signatures = commit["signatures"]
    found = [s for s in signatures if s.get("validator_address") == address]
    if len(found) > 1:
        raise ValueError("duplicate validator signature")
    if not found:
        return {"outcome": "absent_or_not_in_set"}
    signature = found[0]
    flag = signature["block_id_flag"]
    if flag not in (2, 3) or not signature.get("signature"):
        raise ValueError("invalid validator signature entry")
    if len(base64.b64decode(signature["signature"], validate=True)) != 64:
        raise ValueError("invalid validator signature length")
    return {"outcome": "signed" if flag == 2 else "nil",
            "signature_sha256": hashlib.sha256(base64.b64decode(signature["signature"])).hexdigest(),
            "timestamp": signature["timestamp"]}


def identity(config):
    """Tie a configured PID/home to the installed executable's inode and bytes."""
    binary = Path(config["binary"])
    pid = str(config["pid"])
    command = subprocess.check_output(["ps", "-p", pid, "-o", "command="], text=True).strip()
    if command != f"{binary} start --home {config['home']}":
        raise ValueError("configured PID no longer runs the expected node")
    loaded = subprocess.check_output(["lsof", "-a", "-p", pid, "-d", "txt", "-Fni"], text=True)
    inode = None
    found = False
    for line in loaded.splitlines():
        if line.startswith("i"):
            inode = int(line[1:])
        if line == "n" + str(binary) and inode == binary.stat().st_ino:
            found = True
    if not found:
        raise ValueError("loaded and installed executable inode mismatch")
    stat = binary.stat()
    return {"pid": config["pid"], "inode": stat.st_ino, "device": stat.st_dev,
            "size": stat.st_size, "mtime_ns": stat.st_mtime_ns}


class Observer:
    def __init__(self, config, output, request=rpc, identify=identity):
        self.config, self.output = config, Path(output)
        self.request, self.identify = request, identify
        self.output.mkdir(parents=True, exist_ok=False)
        self.initial = identify(config)
        binary = Path(config["binary"])
        version = subprocess.check_output([str(binary), "version", "--long"], text=True)
        commit = re.search(r"^commit: ([0-9a-f]{40})$", version, re.M)
        if not commit or commit[1] != config["candidate_commit"]:
            raise ValueError("unexpected candidate commit")
        with binary.open("rb") as source:
            binary_hash = hashlib.file_digest(source, "sha256").hexdigest()
        self.provenance = {**self.initial, "candidate_commit": commit[1],
                           "binary_sha256": binary_hash, "version": version}
        if identify(config) != self.initial:
            raise ValueError("executable changed during initial measurement")
        self.log = Path(config["log"]).open()
        self.log_inode = Path(config["log"]).stat().st_ino
        self.log.seek(0, 2)
        self.log_partial = ""
        self.executed = {}
        self.next_height = None
        self.pool = ThreadPoolExecutor(max_workers=4)
        self.emit("events", {"event": "start", "config": config, "identity": self.provenance})

    def emit(self, kind, row):
        row = {"time_utc": utc(), "chain_id": self.config["chain_id"],
               "candidate_commit": self.config["candidate_commit"], **row}
        with (self.output / (kind + ".jsonl")).open("a") as target:
            target.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
            target.flush()

    def read_log(self):
        if Path(self.config["log"]).stat().st_ino != self.log_inode:
            raise ValueError("execution log rotated; new observation required")
        if Path(self.config["log"]).stat().st_size < self.log.tell():
            raise ValueError("execution log truncated")
        text = self.log_partial + self.log.read()
        lines = text.split("\n")
        self.log_partial = lines.pop()
        for line in lines:
            match = re.search(r"committed block height=(\d+) hash=([A-Fa-f0-9]{64}) app_hash=([A-Fa-f0-9]{64})", line)
            if match:
                height = int(match[1])
                if self.next_height is not None and height >= self.next_height:
                    value = {"block_hash": match[2].upper(), "app_hash": match[3].upper(), "log_line": line}
                    if height in self.executed and self.executed[height] != value:
                        raise ValueError("contradictory execution log entries")
                    self.executed[height] = value
        if len(self.executed) > 10000:
            raise ValueError("execution backlog exceeds 10000 heights; observation ended")

    def step(self):
        if self.identify(self.config) != self.initial:
            raise ValueError("process or executable changed; observation ended")
        with Path(self.config["binary"]).open("rb") as source:
            measured_hash = hashlib.file_digest(source, "sha256").hexdigest()
        if measured_hash != self.provenance["binary_sha256"]:
            raise ValueError("binary bytes changed; observation ended")
        measured_at = utc()
        local = self.request(self.config["local_rpc"], "/status")
        if local["node_info"]["network"] != self.config["chain_id"]:
            raise ValueError("wrong local chain")
        if local["validator_info"]["address"] != self.config["validator_address"]:
            raise ValueError("wrong local validator")
        height = int(local["sync_info"]["latest_block_height"])
        if self.next_height is None:
            self.next_height = height + 1
            self.emit("events", {"event": "baseline", "first_required_height": self.next_height,
                                 "local_status": local})
        sample = {"local_status": local, "local_status_time_utc": utc(),
                  "binary_sha256": measured_hash, "binary_measured_at_utc": measured_at,
                  "identity": self.initial,
                  "reference_rpc": self.config["reference_rpc"]}
        try:
            reference = self.request(self.config["reference_rpc"], "/status")
            if reference["node_info"]["network"] != self.config["chain_id"]:
                raise ValueError("wrong reference chain")
            target = int(reference["sync_info"]["latest_block_height"])
            sample.update(reference_status=reference, reference_status_time_utc=utc(),
                          reference_height_minus_local=target-height,
                          outcome="observed")
        except Exception as error:
            self.emit("samples", {**sample, "outcome": "unavailable", "error": str(error)})
            self.read_log()
            return
        self.emit("samples", sample)
        self.read_log()
        # Bound catch-up work per pass so independent status observations
        # continue during a slow or unavailable provider. Never skip heights.
        for h in range(self.next_height, min(height, target, self.next_height + 10)):
            execution = self.executed.get(h)
            if execution is None:
                raise ValueError(f"missing local execution log at {h}")
            queries = [("local_results", self.config["local_rpc"], f"/block_results?height={h}"),
                       ("reference_results", self.config["reference_rpc"], f"/block_results?height={h}"),
                       ("header", self.config["reference_rpc"], f"/block?height={h+1}"),
                       ("commit", self.config["reference_rpc"], f"/commit?height={h}")]
            futures = {name: self.pool.submit(self.request, endpoint, route) for name, endpoint, route in queries}
            row = {"executed_height": h, "execution": execution,
                   "local_app_hash": execution["app_hash"],
                   "reference_rpc": self.config["reference_rpc"], "identity": self.initial}
            try:
                data, errors = {}, {}
                for name, future in futures.items():
                    try:
                        data[name] = future.result()
                    except Exception as error:
                        errors[name] = str(error)
                if errors:
                    raise ValueError(json.dumps(errors, sort_keys=True))
                header = data["header"]["block"]["header"]
                commit = data["commit"]["signed_header"]
                if header["chain_id"] != self.config["chain_id"] or int(header["height"]) != h+1:
                    raise ValueError("wrong reference header chain/height")
                if commit["header"]["chain_id"] != self.config["chain_id"] or int(commit["commit"]["height"]) != h:
                    raise ValueError("wrong reference commit chain/height")
                if not re.fullmatch(r"[0-9A-Fa-f]{64}", header["app_hash"]) or not re.fullmatch(r"[0-9A-Fa-f]{64}", commit["commit"]["block_id"]["hash"]):
                    raise ValueError("invalid reference hash")
                ours, theirs = results(data["local_results"], h), results(data["reference_results"], h)
                signed = participation(commit["commit"], self.config["validator_address"])
                if signed["outcome"] == "absent_or_not_in_set":
                    addresses = []
                    for page in range(1, 11):
                        validators = self.request(self.config["reference_rpc"], f"/validators?height={h}&per_page=100&page={page}")
                        if int(validators["block_height"]) != h:
                            raise ValueError("wrong validator set height")
                        addresses.extend(v["address"] for v in validators["validators"])
                        if len(addresses) >= int(validators["total"]):
                            break
                    else:
                        raise ValueError("validator set exceeds collection bound")
                    signed = {"outcome": "missed" if self.config["validator_address"] in addresses else "not_in_set"}
                row.update(reference_chain_id=header["chain_id"],
                           reference_header_height=h+1, reference_app_hash=header["app_hash"],
                           reference_block_hash=commit["commit"]["block_id"]["hash"],
                           local_results_sha256=digest(ours), reference_results_sha256=digest(theirs),
                           local_tx_count=len(ours), reference_tx_count=len(theirs),
                           compared_fields=["code", "data", "gas_wanted", "gas_used"],
                           participation=signed)
                matches = (ours == theirs and execution["app_hash"] == header["app_hash"].upper()
                           and execution["block_hash"] == row["reference_block_hash"].upper())
                row["outcome"] = "match" if matches else "mismatch"
            except Exception as error:
                row.update(outcome="unavailable", error=str(error))
            if self.identify(self.config) != self.initial:
                raise ValueError("process changed during comparison")
            self.emit("heights", row)
            if row["outcome"] == "mismatch":
                raise ValueError(f"parity mismatch at {h}; inspect retained receipt")
            if row["outcome"] != "match":
                # Retry the same height; keep every failed attempt. Later
                # successes cannot erase unknown intervals from an audit.
                return
            del self.executed[h]
            self.next_height = h+1


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--seconds", type=int, default=73*3600)
    parser.add_argument("--interval", type=int, default=5)
    args = parser.parse_args()
    if args.seconds < 1 or args.interval < 1:
        parser.error("seconds and interval must be positive")
    config = json.loads(Path(args.config).read_text())
    observer = Observer(config, args.output)
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f"received signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    deadline = time.monotonic() + args.seconds
    try:
        while time.monotonic() < deadline:
            started = time.monotonic()
            try:
                observer.step()
            except (OSError, KeyError, json.JSONDecodeError) as error:
                observer.emit("events", {"event": "unavailable", "error": str(error)})
            time.sleep(max(0, args.interval - (time.monotonic()-started)))
        observer.emit("events", {"event": "deadline", "next_height": observer.next_height})
    except BaseException as error:
        observer.emit("events", {"event": "stopped", "error": str(error), "next_height": observer.next_height})
        raise
    finally:
        observer.pool.shutdown(wait=True)
        observer.log.close()


if __name__ == "__main__":
    main()
