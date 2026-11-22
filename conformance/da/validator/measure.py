"""Compare local validator voting during idle and concurrent RPC/SHREX reads."""
import concurrent.futures
import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


def status():
    with urllib.request.urlopen("http://127.0.0.1:26657/status", timeout=3) as response:
        status = json.load(response)["result"]
    return status


def height():
    return int(status()["sync_info"]["latest_block_height"])


def cpu(pid):
    fields = (Path("/proc") / pid / "stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def measure(duration, pid, commands, checker, rpc):
    initial = status()
    validator = initial["validator_info"]["address"]
    first = previous = int(initial["sync_info"]["latest_block_height"])
    start = last = time.monotonic()
    started_cpu = cpu(pid)
    longest_gap = 0.0
    counts = {name: 0 for name in commands}

    def load(name, args):
        while time.monotonic() - start < duration:
            try:
                result = subprocess.run(args, timeout=45, stdout=subprocess.DEVNULL)
            except subprocess.TimeoutExpired:
                raise RuntimeError(f"{name} exceeded 45 s after {counts[name]} verified reads, "
                                   f"last observed validator height {previous}") from None
            if result.returncode:
                raise RuntimeError(f"{name} failed with status {result.returncode} after "
                                   f"{counts[name]} verified reads at validator height {previous}")
            counts[name] += 1

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        workers = [pool.submit(load, name, args) for name, args in commands.items()]
        while time.monotonic() - start < duration:
            for worker in workers:
                if worker.done():
                    worker.result()
            head = height()
            now = time.monotonic()
            longest_gap = max(longest_gap, now - last)
            if head < previous:
                raise RuntimeError("consensus height decreased")
            if head > previous:
                previous, last = head, now
            time.sleep(0.25)
        for worker in workers:
            worker.result()
    end = height()
    elapsed = time.monotonic() - start
    processor = cpu(pid) - started_cpu
    if end - first < 2 or any(count == 0 for count in counts.values()):
        raise RuntimeError("insufficient commits or completed proof reads")
    subprocess.run([checker, rpc, str(first), str(end), validator], check=True, timeout=130)
    return dict(first=first, last=end, elapsed_seconds=round(elapsed, 2),
                max_observed_commit_gap_seconds=round(longest_gap, 2),
                validator_cpu_seconds=round(processor, 2), verified_reads=counts)


def main():
    pid, work, peer, paid_height, ns, data = sys.argv[1:]
    seconds = int(os.environ.get("DA_VALIDATOR_SECONDS", "30"))
    if seconds < 10:
        raise ValueError("each interval must last at least ten seconds")
    rpc = "http://127.0.0.1:32658"
    checker = Path(work) / "commits"
    commands = {
        "rpc": [Path(work) / "rpc-read", rpc, paid_height, ns, data],
        "shrex": [Path(work) / "peer-read", "celestia-rs-local", peer, paid_height, ns, data],
    }
    for label, workload in [("idle", {}), ("serving", commands)]:
        print(json.dumps({label: measure(seconds, pid, workload, checker, rpc)}), flush=True)


if __name__ == "__main__":
    main()
