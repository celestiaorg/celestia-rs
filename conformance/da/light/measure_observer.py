"""Measure a private mixed-network read workload on Linux. Emits a JSON receipt."""

import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request


def cpu_seconds(pid):
    # The parenthesized command may contain whitespace. utime and stime are
    # fields 14 and 15, starting at field 3 after the final parenthesis.
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def peak_rss_kib(pid):
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmHWM:"):
            return int(line.split()[1])
    raise ValueError("kernel did not report peak resident memory")


def rpc_height(url, token):
    if token:
        payload = {"jsonrpc": "2.0", "id": 1, "method": "header.LocalHead", "params": []}
        request = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={
            "Content-Type": "application/json", "Authorization": f"Bearer {token}"
        })
    else:
        request = url + "/status"
    with urllib.request.urlopen(request, timeout=5) as response:
        result = json.load(response)["result"]
    return int(result["header"]["height"] if token else result["sync_info"]["latest_block_height"])


def main():
    probe, height, paid_blob, directory, consensus, bridge, observer = sys.argv[1:]
    processes = {"go_validator": (consensus, "consensus"), "go_bridge": (bridge, "bridge"),
                 "rust_observer_bridge": (observer, "follower")}
    endpoints = {"go_bridge": ("http://127.0.0.1:26658", os.environ["GO_READ_TOKEN"]),
                 "rust_observer_bridge": ("http://127.0.0.1:37658", os.environ["DA_TEST_READ_TOKEN"])}
    source = "http://127.0.0.1:36657"
    ticks = {role: cpu_seconds(pid) for role, (pid, _) in processes.items()}
    initial_height = rpc_height(source, None)
    start = time.monotonic()
    seconds = int(os.environ.get("DA_QUALIFICATION_SECONDS", "10"))
    if seconds < 10:
        raise ValueError("DA_QUALIFICATION_SECONDS must be at least 10")
    last_observed = initial_height
    last_progress = start
    longest_interval = 0.0
    rounds = 0
    max_lag = dict.fromkeys(endpoints, 0)

    def read(endpoint):
        url, token = endpoint
        try:
            result = subprocess.run([probe, url, height, "0103", paid_blob], stdout=subprocess.DEVNULL,
                                    env={**os.environ, "DA_TEST_READ_TOKEN": token}, timeout=40)
        except subprocess.TimeoutExpired:
            raise RuntimeError(f"paid read on {url} exceeded 40 s after {rounds} paired rounds, "
                               f"last observed Go validator height {last_observed}") from None
        if result.returncode:
            raise RuntimeError(f"paid read failed on {url} with status {result.returncode}")

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        while time.monotonic() - start < seconds or rounds < 3:
            list(pool.map(read, endpoints.values()))
            rounds += 1
            reference = rpc_height(source, None)
            now = time.monotonic()
            longest_interval = max(longest_interval, now - last_progress)
            if reference > last_observed:
                last_progress, last_observed = now, reference
            for role, endpoint in endpoints.items():
                max_lag[role] = max(max_lag[role], reference - rpc_height(*endpoint))
    elapsed = time.monotonic() - start
    final_height = rpc_height(source, None)
    report = {}
    for role, (pid, home) in processes.items():
        report[role] = {
            "cpu_seconds": round(cpu_seconds(pid) - ticks[role], 2),
            "process_peak_rss_kib": peak_rss_kib(pid),
            "store_kib": int(subprocess.check_output(["du", "-sk", Path(directory) / home]).split()[0]),
        }
        if role in max_lag:
            report[role]["max_observed_lag_blocks"] = max_lag[role]
    print(json.dumps(dict(seconds=round(elapsed, 2), verified_reads_per_bridge=rounds,
                          first_height=initial_height, last_height=final_height,
                          longest_sampled_commit_interval=round(longest_interval, 2), processes=report)))
    if final_height <= initial_height:
        raise RuntimeError("validator did not advance under paired read load")


if __name__ == "__main__":
    main()
