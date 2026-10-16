#!/usr/bin/env python3
"""Soak + chaos orchestrator for `../soak.sh` (see there for the scenario).

Runs on a network prepared by soak.sh (homes n0..n6 with `kind` files):
starts the link proxy and the nodes, drives the workload and the chaos
schedule, checks the chain continuously and writes the report.
"""

import argparse
import base64
import hashlib
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cosmostx as ctx  # noqa: E402

EMPTY_HASH = hashlib.sha256(b"").hexdigest().upper()
ANSI = re.compile(r"\x1b\[[0-9;]*m")

# Log lines that fail the run, per node kind.
GO_FATAL = [
    "state machine rejected a proposed block",
    "received invalid compact block",
    "invalid compact block",
    "wrong Block.Header",
    "CONSENSUS FAILURE",
    "panic:",
    "found conflicting vote",
    "conflicting votes",
    "double sign",
    "Error in validation",
]
RS_FATAL = [
    "panicked",
    "CONSENSUS FAILURE",
    "found conflicting vote",
    "conflicting votes",
    "double sign",
    "Error in validation",
    "wrong Block.Header",
]


def now():
    return time.time()


def get_json(url, timeout=10):
    return json.load(urllib.request.urlopen(url, timeout=timeout))


class Node:
    def __init__(self, soak, i):
        self.soak = soak
        self.i = i
        self.home = f"{soak.dir}/n{i}"
        self.kind = open(f"{self.home}/kind").read().strip()
        self.p2p = 26656 + soak.off + 10 * i
        self.rpc = 26657 + soak.off + 10 * i
        self.url = f"http://127.0.0.1:{self.rpc}"
        self.prom = 26660 + soak.off + 10 * i
        self.proc = None
        self.expected_up = False
        self.restarts = 0
        self.statesyncs = 0
        self.checked = 0
        self.max_rss = 0
        self.lifetimes = []  # [(start_t, [(t, rss_mb)])]
        self.log_off = 0
        self.sync_pending = None  # state sync in progress: {"kind": ..., "since": t}
        self.wal_max = {}  # largest WAL group sample: head / total / files
        self.lock = threading.Lock()
        k = json.load(open(f"{self.home}/config/priv_validator_key.json"))
        self.val_addr = k["address"].upper()

    @property
    def is_validator_key(self):
        return self.i in self.soak.validator_nodes

    def peers(self):
        return ",".join(
            f"{self.soak.node_ids[j]}@127.0.0.1:{self.soak.proxy_port(self.i, j)}"
            for j in range(self.soak.n)
            if j != self.i
        )

    def start(self, extra=()):
        with self.lock:
            log = open(f"{self.home}/node.log", "ab")
            env = dict(os.environ)
            if self.kind == "go":
                cmd = ["celestia-appd", "start", "--home", self.home]
                if os.environ.get("SOAK_GOMEMLIMIT"):
                    env["GOMEMLIMIT"] = os.environ["SOAK_GOMEMLIMIT"]
            else:
                env.setdefault("RUST_LOG", "info")
                # Per-node overrides (memory profile: a jemalloc build, MALLOC_CONF).
                for kv in filter(None, os.environ.get(f"SOAK_ENV_N{self.i}", "").split(";")):
                    k, _, v = kv.partition("=")
                    env[k] = v
                cmd = [
                    os.environ.get(f"SOAK_RS_N{self.i}", self.soak.rs), "start", "--home", self.home,
                    "--p2p-laddr", f"127.0.0.1:{self.p2p}",
                    "--persistent-peers", self.peers(),
                    "--moniker", f"node{self.i}", "--allow-duplicate-ip", *extra,
                ]
            self.proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env, start_new_session=True)
            self.write_pid()
            self.expected_up = True
            self.lifetimes.append((now(), []))
        # Ports above 32767 can be taken as ephemeral source ports.
        for _ in range(10):
            time.sleep(3)
            if self.proc.poll() is None:
                return
            tail = open(f"{self.home}/node.log", errors="replace").read()[-4000:]
            if "address already in use" not in tail.lower():
                return
            self.soak.event("port-race-restart", node=self.i)
            with self.lock:
                self.proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env, start_new_session=True)
                self.write_pid()

    def write_pid(self):
        """`soak.pid`: lets a resumed orchestrator stop a node left running
        by a previous one (by PID, after checking its command line)."""
        open(f"{self.home}/soak.pid", "w").write(str(self.proc.pid))

    def kill_stale(self):
        try:
            pid = int(open(f"{self.home}/soak.pid").read())
            cmdline = open(f"/proc/{pid}/cmdline", "rb").read().decode(errors="replace")
        except (OSError, ValueError):
            return False
        if self.home not in cmdline.split("\0"):
            return False
        os.kill(pid, signal.SIGKILL)
        for _ in range(100):
            if not os.path.exists(f"/proc/{pid}"):
                break
            time.sleep(0.1)
        return True

    def wal(self):
        """The consensus WAL group: head size, total size, file count."""
        d = f"{self.home}/data/cs.wal"
        head = total = files = 0
        try:
            for e in os.listdir(d):
                if e == "wal" or re.fullmatch(r"wal\.\d{3,}", e):
                    sz = os.path.getsize(f"{d}/{e}")
                    total += sz
                    files += 1
                    if e == "wal":
                        head = sz
        except OSError:
            pass
        return {"head": head, "total": total, "files": files}

    def kill(self, sig=signal.SIGKILL):
        with self.lock:
            self.expected_up = False
            if self.proc and self.proc.poll() is None:
                try:
                    os.kill(self.proc.pid, sig)
                except ProcessLookupError:
                    pass
                try:
                    self.proc.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    os.kill(self.proc.pid, signal.SIGKILL)
                    self.proc.wait()

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def status(self):
        try:
            si = get_json(f"{self.url}/status", timeout=3)["result"]["sync_info"]
            return int(si["latest_block_height"]), int(si.get("earliest_block_height") or 1), si["catching_up"]
        except Exception:
            return None

    def height(self):
        s = self.status()
        return s[0] if s else None

    def rss_mb(self):
        if not self.alive():
            return 0.0
        try:
            for line in open(f"/proc/{self.proc.pid}/status"):
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024.0
        except Exception:
            pass
        return 0.0


class Soak:
    def __init__(self, a):
        self.a = a
        self.dir = a.dir
        self.off = a.off
        self.rs = a.rs
        self.chain_id = a.chain_id
        self.rng = random.Random(a.seed)
        self.n = len([d for d in os.listdir(self.dir) if re.fullmatch(r"n\d+", d)])
        self.node_ids = [open(f"{self.dir}/n{i}/node_id").read().strip() for i in range(self.n)]
        self.validator_nodes = [int(x) for x in a.validators.split(",")]
        self.ref_i = a.ref_node  # Go node with the tx index (queries, hash reference)
        self.nodes = [Node(self, i) for i in range(self.n)]
        self.by_addr = {nd.val_addr: nd.i for nd in self.nodes}
        self.t0 = now()
        self.deadline = self.t0 + a.secs
        self.stop = threading.Event()
        self.chaos_stop = threading.Event()
        self.failures = []
        self.events = open(f"{self.dir}/events.log", "a")
        self.ev_lock = threading.Lock()
        self.control = {"cut": [], "delay_ms": {}}
        self.halt_until = 0.0
        self.max_height = 0
        self.last_progress = now()
        self.stalls = []
        self.stall_open = None
        self.ref_cache = {}  # height -> dict
        self.ref_checked = 0
        self.metrics = {
            "bank": {"ok": 0, "rejected": {}, "errors": 0, "resyncs": 0},
            "pfb": {"submitted": 0, "rejected": {}, "included": 0, "failed_exec": 0, "lost": 0, "max_round_bytes": 0},
            "staking": {"submitted": 0, "rejected": {}, "included": 0, "failed_exec": 0, "lost": 0},
            "gov": {"proposals": 0, "passed": 0, "rejected": 0, "other": 0, "votes": 0},
            "valset": [],
            "chaos": [],
        }
        self.pending_txs = []  # (hash, category, t)
        self.pending_lock = threading.Lock()
        self.proposed = {}
        self.blocks = {"txs": 0, "max_bytes": 0, "over_8mb": 0, "count": 0}
        self.log_hits = {}
        self.unbonded = set()
        self.valset_node = a.valset_node
        # Long profile (restart-tolerant): new workloads and their state.
        for k in ("authz", "feegrant", "hyperlane", "utf8"):
            self.metrics[k] = {"submitted": 0, "rejected": {}, "included": 0, "failed_exec": 0, "lost": 0}
        self.metrics["utf8"]["checktx_parity"] = {"same": 0, "diff": []}
        self.metrics["precommit_stop"] = {"cut_short": 0, "signed_before_stop": 0, "unknown": 0}
        self.metrics["sign_refusals"] = {}
        self.metrics["wal"] = {}
        self.metrics["disk"] = {"max_total_mb": 0, "pressure_syncs": 0}
        self.metrics["rss"] = {"max_total_mb": 0, "pressure_restarts": 0}
        self.long = {"hyperlane_step": 0, "hyperlane_nonce": 0}
        self.segment = 1
        self.segments = []
        self.index_gaps = []  # [t0, t1] windows the tx-index node missed blocks in
        self.key_locks = {k: threading.Lock() for k in ("val0", "user0", "az1", "az2", "hyp1")}
        self.state_lock = threading.Lock()
        self.val_lock = threading.Lock()  # valset cycle vs validator-affecting chaos
        self.disk_pressure = self.rss_pressure = False
        self.hashes_f = None
        if a.resume:
            self.load_state()
        self.seg_start = now()
        self.seg_t0 = self.elapsed()

    # ------------------------------------------------------------ utils
    def elapsed(self):
        return now() - self.t0

    def event(self, _ev, **kw):
        line = json.dumps({"t": round(self.elapsed(), 1), "h": self.max_height, "ev": _ev, **kw})
        with self.ev_lock:
            self.events.write(line + "\n")
            self.events.flush()
        print(line, flush=True)

    def fail(self, msg):
        self.failures.append(f"[t={self.elapsed():.0f}s h={self.max_height}] {msg}")
        self.event("FAIL", msg=msg)

    # ------------------------------------------------------------ persistence
    # A long soak runs in segments (`--resume` after the machine restarted
    # and killed every process): `state.json` carries the cumulative
    # counters, `hashes.jsonl` every reference height's block / app hash.
    def save_state(self):
        st = {
            "cum_elapsed": round(self.elapsed(), 1), "segment": self.segment,
            "segments": self.segments + [{"segment": self.segment, "wall_start": round(self.seg_start),
                                          "wall_end": round(now()), "t_start": round(self.seg_t0)}],
            "max_height": self.max_height, "metrics": self.metrics, "failures": self.failures,
            "stalls": [{k: v for k, v in x.items() if not k.startswith("_")} for x in self.stalls],
            "log_hits": self.log_hits, "unbonded": sorted(self.unbonded), "long": self.long,
            "index_gaps": self.index_gaps,
            "pending": list(self.pending_txs),
            "nodes": {str(nd.i): {"checked": nd.checked, "restarts": nd.restarts, "statesyncs": nd.statesyncs,
                                  "max_rss": nd.max_rss, "log_off": nd.log_off, "sync_pending": nd.sync_pending,
                                  "wal_max": nd.wal_max} for nd in self.nodes},
        }
        with self.state_lock:
            for _ in range(5):
                try:
                    data = json.dumps(st, indent=1)
                    break
                except RuntimeError:  # a dict changed size while dumping
                    time.sleep(0.05)
            else:
                return
            tmp = f"{self.dir}/state.json.tmp"
            with open(tmp, "w") as f:
                f.write(data)
                f.flush()
                os.fsync(f.fileno())
            os.replace(tmp, f"{self.dir}/state.json")

    def load_state(self):
        st = json.load(open(f"{self.dir}/state.json"))
        cum = st["cum_elapsed"]
        self.t0 = now() - cum
        self.deadline = self.t0 + self.a.secs
        self.segment = st["segment"] + 1
        self.segments = st["segments"]
        self.max_height = st["max_height"]
        for k, v in st["metrics"].items():
            if isinstance(v, dict) and isinstance(self.metrics.get(k), dict):
                self.metrics[k].update(v)
            else:
                self.metrics[k] = v
        self.failures = st["failures"]
        self.stalls = st["stalls"]
        self.log_hits = st["log_hits"]
        self.unbonded = set(st["unbonded"])
        self.long.update(st["long"])
        self.index_gaps = st.get("index_gaps", [])
        self.pending_txs = [tuple(x) for x in st.get("pending", [])]
        for i, ns in st["nodes"].items():
            nd = self.nodes[int(i)]
            nd.checked, nd.restarts, nd.statesyncs = ns["checked"], ns["restarts"], ns["statesyncs"]
            nd.max_rss, nd.log_off, nd.sync_pending = ns["max_rss"], ns["log_off"], ns["sync_pending"]
            nd.wal_max = ns.get("wal_max", {})
        # Reference hashes and block counters from hashes.jsonl (written
        # before `ref_checked` moves, so nothing is counted twice).
        try:
            lines = open(f"{self.dir}/hashes.jsonl").read().splitlines()
        except FileNotFoundError:
            lines = []
        kept = []
        for line in lines:
            try:
                r = json.loads(line)
            except ValueError:
                continue  # torn last line
            ht = r["h"]
            if ht != self.ref_checked + 1:
                continue
            self.ref_cache[ht] = {"hash": r["hash"], "app_hash": r["app"], "proposer": r["p"]}
            self._count_block(r["p"], r["ntx"], r["size"])
            self.ref_checked = ht
            kept.append(line)
        self.max_height = max(self.max_height, self.ref_checked)
        # Drop a torn last line.
        if len(kept) != len(lines):
            with open(f"{self.dir}/hashes.jsonl", "w") as f:
                f.write("".join(x + "\n" for x in kept))

    def _count_block(self, proposer, ntx, size):
        self.blocks["txs"] += ntx
        self.blocks["count"] += 1
        self.blocks["max_bytes"] = max(self.blocks["max_bytes"], size)
        self.blocks["over_8mb"] += size > 8 * 2**20
        p = self.by_addr.get(proposer, -1)
        self.proposed[p] = self.proposed.get(p, 0) + 1

    def proxy_port(self, i, j):
        return 26656 + self.off + 500 + 10 * i + j

    def write_control(self):
        tmp = f"{self.dir}/proxy-control.json.tmp"
        json.dump(self.control, open(tmp, "w"))
        os.replace(tmp, f"{self.dir}/proxy-control.json")

    def up_nodes(self, kind=None):
        return [nd for nd in self.nodes if nd.expected_up and nd.alive() and (kind is None or nd.kind == kind)]

    def ref_node(self):
        """The highest Go node (else any node) that still holds the next
        height to check (a state-synced node starts at its snapshot)."""
        best, bh = None, -1
        need = self.ref_checked + 1
        for kind in ("go", None):
            for nd in self.up_nodes(kind):
                st = nd.status()
                if st is None or st[1] > need:
                    continue
                h = st[0]
                if h > bh or (h == bh and nd.i == self.ref_i):
                    best, bh = nd, h
            if best is not None:
                break
        return best, bh

    def tx_node(self):
        ups = self.up_nodes()
        return self.rng.choice(ups) if ups else self.nodes[self.ref_i]

    def cli(self, args, home=None, timeout=120):
        home = home or f"{self.dir}/n0"
        try:
            out = subprocess.run(
                ["celestia-appd", *args, "--keyring-backend", "test", "--home", home],
                capture_output=True, text=True, timeout=timeout,
            )
            return out.stdout + out.stderr
        except subprocess.TimeoutExpired:
            return "timeout"

    def addr(self, key, home=None, val=False):
        args = ["keys", "show", key, "-a"] + (["--bech", "val"] if val else [])
        return self.cli(args, home).strip().splitlines()[-1]

    def tx(self, category, key, args, home=None, gas=400000, fees=20000, track=True):
        nd = self.tx_node()
        out = self.cli(
            ["tx", *args, "--from", key, "--chain-id", self.chain_id, "--node", f"tcp://127.0.0.1:{nd.rpc}",
             "--gas", str(gas), "--fees", f"{fees}utia", "-y", "-o", "json"],
            home,
        )
        m = self.metrics[category]
        try:
            d = json.loads(out[out.index("{"):])
            code = int(d.get("code", 0))
            h = d.get("txhash", "")
        except Exception:
            code, h = -1, ""
        if code == 0 and h:
            m["submitted"] = m.get("submitted", 0) + 1
            if track:
                with self.pending_lock:
                    self.pending_txs.append((h, category, now(), nd.i))
        else:
            k = str(code) if code != -1 else ("err:" + out.strip()[:80])
            m["rejected"][k] = m["rejected"].get(k, 0) + 1
        return code, h

    def wait_included(self, h, timeout=120):
        """Exec code and result of tx `h` from the tx-index node, or (None,
        None) if it was not found within `timeout`."""
        t = now()
        while now() - t < timeout and not self.stop.is_set():
            try:
                r = get_json(f"{self.nodes[self.ref_i].url}/tx?hash=0x{h}", timeout=5).get("result")
                if r:
                    return int(r["tx_result"].get("code", 0)), r
            except Exception:
                pass
            self.stop.wait(2)
        return None, None

    def tx_wait(self, category, key, args, op=None, gas=400000, fees=20000):
        """A CLI tx, then its inclusion; counts per category and per op.
        Returns the exec code (None: rejected by CheckTx or not found)."""
        code, h = self.tx(category, key, args, gas=gas, fees=fees, track=False)
        m = self.metrics[category]
        ops = m.setdefault("ops", {})
        o = ops.setdefault(op or args[1], {"ok": 0, "exec_fail": {}, "checktx_fail": 0, "lost": 0})
        if code != 0 or not h:
            o["checktx_fail"] += 1
            return None
        ec, _ = self.wait_included(h)
        if ec is None:
            if not self.stop.is_set():
                m["lost"] += 1
                o["lost"] += 1
            return None
        m["included"] += 1
        if ec == 0:
            o["ok"] += 1
        else:
            m["failed_exec"] += 1
            o["exec_fail"][str(ec)] = o["exec_fail"].get(str(ec), 0) + 1
        return ec

    # ------------------------------------------------------------ setup
    def setup(self):
        routes = []
        for i in range(self.n):
            for j in range(self.n):
                if i != j:
                    routes.append({"listen": self.proxy_port(i, j), "target": self.nodes[j].p2p, "src": i, "dst": j})
        json.dump(routes, open(f"{self.dir}/proxy-routes.json", "w"))
        self.write_control()
        try:  # a proxy left by a previous orchestrator
            pid = int(open(f"{self.dir}/proxy.pid").read())
            if f"{self.dir}/proxy-routes.json" in open(f"/proc/{pid}/cmdline", "rb").read().decode(errors="replace"):
                os.kill(pid, signal.SIGKILL)
                time.sleep(0.5)
        except (OSError, ValueError):
            pass
        self.proxy = subprocess.Popen(
            [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "netproxy.py"),
             f"{self.dir}/proxy-routes.json", f"{self.dir}/proxy-control.json"],
            stdout=open(f"{self.dir}/proxy.log", "a"), stderr=subprocess.STDOUT, start_new_session=True,
        )
        open(f"{self.dir}/proxy.pid", "w").write(str(self.proxy.pid))
        time.sleep(1)
        for nd in self.nodes:
            cfg = f"{nd.home}/config/config.toml"
            s = open(cfg).read()
            s = re.sub(r'(?m)^persistent_peers = .*$', f'persistent_peers = "{nd.peers()}"', s)
            if nd.i == self.ref_i:
                s = re.sub(r'(?m)^indexer = .*$', 'indexer = "kv"', s, count=1)
            if self.a.mem_sample:
                s = re.sub(r'(?m)^prometheus = .*$', 'prometheus = true', s, count=1)
                s = re.sub(r'(?m)^prometheus_listen_addr = .*$', f'prometheus_listen_addr = "127.0.0.1:{nd.prom}"', s, count=1)
            open(cfg, "w").write(s)

    # ------------------------------------------------------------ monitor
    def monitor_loop(self):
        last_rss = last_disk = last_save = 0.0
        rss_f = open(f"{self.dir}/rss.csv", "a")
        disk_f = open(f"{self.dir}/disk.csv", "a")
        rss_f.write("t,height," + ",".join(f"n{i}_{nd.kind}" for i, nd in enumerate(self.nodes)) + "\n")
        disk_f.write("t,height," + ",".join(f"n{i}_{nd.kind}" for i, nd in enumerate(self.nodes)) + "\n")
        while not self.stop.is_set():
            try:
                hs = []
                for nd in self.nodes:
                    with nd.lock:  # kill() flips expected_up under the lock
                        died = nd.expected_up and not nd.alive()
                    if died:
                        self.fail(f"node {nd.i} ({nd.kind}) exited unexpectedly (code {nd.proc.returncode})")
                        nd.expected_up = False
                    if nd.expected_up:
                        h = nd.height()
                        if h is not None:
                            hs.append(h)
                mh = max(hs) if hs else self.max_height
                t = now()
                if mh > self.max_height:
                    self.max_height = mh
                    self.last_progress = t
                    if self.stall_open:
                        self.stall_open["end_t"] = round(self.elapsed(), 1)
                        self.stall_open["secs"] = round(t - self.stall_open["_since"], 1)
                        self.stall_open.pop("_since")
                        self.event("stall-end", **self.stall_open)
                        self.stall_open = None
                else:
                    since = max(self.last_progress, self.halt_until)
                    if t - since > 60 and not self.stall_open:
                        self.stall_open = {"start_t": round(since - self.t0, 1), "height": mh, "_since": since,
                                           "chaos": list(self.control["cut"]), "unbonded": sorted(self.unbonded)}
                        self.stalls.append(self.stall_open)
                        self.fail(f"chain stalled > 60 s at height {mh}")
                if t - last_rss >= 15:
                    last_rss = t
                    row = []
                    total_rss = 0.0
                    for nd in self.nodes:
                        r = nd.rss_mb()
                        total_rss += r
                        nd.max_rss = max(nd.max_rss, r)
                        if r and nd.lifetimes:
                            nd.lifetimes[-1][1].append((t, r))
                        row.append(f"{r:.1f}")
                    rss_f.write(f"{t - self.t0:.0f},{self.max_height}," + ",".join(row) + "\n")
                    rss_f.flush()
                    self.metrics["rss"]["max_total_mb"] = max(self.metrics["rss"]["max_total_mb"], round(total_rss))
                    budget = float(os.environ.get("SOAK_RSS_BUDGET_MB", "0"))
                    if budget and total_rss > budget:
                        self.rss_pressure = True
                if t - last_save >= 30:
                    last_save = t
                    self.check_wal()
                    self.save_state()
                if t - last_disk >= 60:
                    last_disk = t
                    free = shutil.disk_usage(self.dir).free
                    if free < float(os.environ.get("SOAK_MIN_FREE_GB", "3")) * 2**30 and now() < self.deadline:
                        self.fail(f"only {free / 2**30:.1f} GiB disk left: ending the soak early")
                        self.deadline = now()
                        self.a.min_heights = 0
                    row = [f"{du(f'{nd.home}/data') / 2**20:.1f}" for nd in self.nodes]
                    total = du(self.dir) / 2**20
                    disk_f.write(f"{t - self.t0:.0f},{self.max_height}," + ",".join(row) + f",{total:.1f}\n")
                    disk_f.flush()
                    self.metrics["disk"]["max_total_mb"] = max(self.metrics["disk"]["max_total_mb"], round(total))
                    soft = float(os.environ.get("SOAK_DISK_SOFT_GB", "0"))
                    min_free = float(os.environ.get("SOAK_MIN_FREE_GB", "3"))
                    if soft and (total > soft * 1024 or free < (min_free + 0.7) * 2**30):
                        self.disk_pressure = True
                    self.rotate_logs()
            except Exception:
                self.event("monitor-error", tb=traceback.format_exc()[-500:])
            self.stop.wait(5)

    def check_wal(self):
        """WAL group sizes against the group limits (Go's autofile.Group:
        the head rotates at the head limit, the oldest files go once the
        total passes the total limit; the head itself is never removed)."""
        for nd in self.nodes:
            w = nd.wal()
            for k, v in w.items():
                nd.wal_max[k] = max(nd.wal_max.get(k, 0), v)
            env = dict(kv.partition("=")[::2] for kv in filter(None, os.environ.get(f"SOAK_ENV_N{nd.i}", "").split(";")))
            head_lim = int(env.get("CELESTIA_RS_TEST_WAL_HEAD_SIZE_LIMIT", 10 * 1024 * 1024))
            total_lim = int(env.get("CELESTIA_RS_TEST_WAL_TOTAL_SIZE_LIMIT", 1024 * 1024 * 1024))
            over = w["total"] - w["head"] > total_lim + head_lim
            key = f"n{nd.i}"
            m = self.metrics["wal"].setdefault(key, {"over_samples": 0, "max_files": 0, "max_total": 0, "max_head": 0})
            m["max_files"] = max(m["max_files"], w["files"])
            m["max_total"] = max(m["max_total"], w["total"])
            m["max_head"] = max(m["max_head"], w["head"])
            if over:
                m["over_samples"] += 1
                if m["over_samples"] >= 3:
                    self.fail(f"node {nd.i} WAL group over its limits: {w} (head {head_lim}, total {total_lim})")
            else:
                m["over_samples"] = 0

    def rotate_logs(self):
        cap = int(float(os.environ.get("SOAK_LOG_CAP_MB", "200")) * 2**20)
        for nd in self.nodes:
            p = f"{nd.home}/node.log"
            try:
                if os.path.getsize(p) > cap:
                    # The node appends (O_APPEND): keep the last 20 MB in node.log.1 and truncate.
                    with open(p, "rb") as f:
                        f.seek(-20 * 2**20, 2)
                        tail = f.read()
                    open(p + ".1", "wb").write(tail)
                    os.truncate(p, 0)
                    nd.log_off = 0
                    self.event("log-rotated", node=nd.i)
            except OSError:
                pass

    # ------------------------------------------------------------ memory
    def mem_sample(self, nd):
        """One memory sample of a node: /proc (RSS, threads, fds), disk and
        its Prometheus memory gauges (Rust `memstats_*`, Go `go_memstats_*`)."""
        row = {"rss_mb": round(nd.rss_mb(), 1), "disk_mb": round(du(f"{nd.home}/data") / 2**20, 1)}
        if not nd.alive():
            return row
        pid = nd.proc.pid
        try:
            for line in open(f"/proc/{pid}/status"):
                if line.startswith(("Threads:", "VmHWM:", "RssAnon:", "RssFile:")):
                    k, v = line.split(":", 1)
                    row[k.lower()] = int(v.split()[0]) / (1 if k == "Threads" else 1024.0)
            row["fds"] = len(os.listdir(f"/proc/{pid}/fd"))
        except Exception:
            pass
        try:
            text = urllib.request.urlopen(f"http://127.0.0.1:{nd.prom}/metrics", timeout=5).read().decode()
        except Exception:
            return row
        kv = {}
        for line in text.splitlines():
            if line.startswith("#") or not line.strip():
                continue
            m = re.match(r'^(\S+?)(\{[^}]*\})?\s+(\S+)$', line)
            if not m:
                continue
            name, labels, v = m.groups()
            try:
                v = float(v)
            except ValueError:
                continue
            if "memstats_kvdb_" in name:
                db = re.search(r'db="([^"]+)"', labels or "")
                key = name.split("memstats_", 1)[1] + "." + (db.group(1) if db else "?")
                kv[key] = kv.get(key, 0.0) + v
            elif "memstats_" in name:
                kv[name.split("memstats_", 1)[1]] = v
            elif name.endswith(("mempool_size", "mempool_size_bytes", "p2p_peers")) or name in (
                    "go_memstats_heap_inuse_bytes", "go_memstats_heap_alloc_bytes", "go_memstats_heap_sys_bytes",
                    "go_memstats_heap_idle_bytes", "go_memstats_heap_released_bytes", "go_memstats_sys_bytes",
                    "go_goroutines"):
                short = name.split("_", 1)[1] if name.startswith(("cometbft_", "celestia_")) else name
                kv[short] = kv.get(short, 0.0) + v
        row.update({k: round(v, 1) for k, v in kv.items()})
        return row

    def mem_loop(self):
        f = open(f"{self.dir}/mem.jsonl", "a")
        while not self.stop.is_set():
            t = now()
            for nd in self.nodes:
                try:
                    row = self.mem_sample(nd)
                except Exception:
                    row = {"error": traceback.format_exc()[-200:]}
                f.write(json.dumps({"t": round(t - self.t0), "h": self.max_height, "node": nd.i, "kind": nd.kind,
                                    "life": len(nd.lifetimes), **row}) + "\n")
            f.flush()
            self.stop.wait(max(1.0, self.a.mem_sample - (now() - t)))

    # ------------------------------------------------------------ consistency
    def fetch_ref(self, ref, upto):
        h = self.ref_checked + 1
        while h <= upto and not self.stop.is_set():
            hi = min(upto, h + 19)
            metas = get_json(f"{ref.url}/blockchain?minHeight={h}&maxHeight={hi}")["result"]["block_metas"]
            for m in metas:
                hdr = m["header"]
                ht = int(hdr["height"])
                self.ref_cache[ht] = {
                    "hash": m["block_id"]["hash"].upper(),
                    "app_hash": hdr["app_hash"].upper(),
                    "proposer": hdr["proposer_address"].upper(),
                }
                if hdr["evidence_hash"].upper() not in ("", EMPTY_HASH):
                    self.fail(f"block {ht} carries evidence (evidence_hash {hdr['evidence_hash']})")
                ntx = int(m.get("num_txs", 0))
                size = int(m.get("block_size", 0))
                if self.hashes_f is None:
                    self.hashes_f = open(f"{self.dir}/hashes.jsonl", "a")
                r = self.ref_cache[ht]
                self.hashes_f.write(json.dumps({"h": ht, "hash": r["hash"], "app": r["app_hash"], "p": r["proposer"],
                                                "ntx": ntx, "size": size, "by": ref.i}) + "\n")
                self._count_block(r["proposer"], ntx, size)
            if self.hashes_f is not None:
                self.hashes_f.flush()
            h = hi + 1
            self.ref_checked = hi

    def check_node(self, nd, limit=400):
        st = nd.status()
        if not st:
            return
        top, base, _ = st
        upto = min(top, self.ref_checked)
        h = max(nd.checked + 1, base)
        n = 0
        while h <= upto and n < limit:
            try:
                c = get_json(f"{nd.url}/commit?height={h}")["result"]["signed_header"]
            except Exception:
                return
            got_hash = c["commit"]["block_id"]["hash"].upper()
            got_app = c["header"]["app_hash"].upper()
            r = self.ref_cache[h]
            if got_hash != r["hash"] or got_app != r["app_hash"]:
                self.fail(f"node {nd.i} ({nd.kind}) height {h}: block {got_hash[:12]} app {got_app[:12]} != ref {r['hash'][:12]} {r['app_hash'][:12]}")
            nd.checked = h
            h += 1
            n += 1

    def consistency_loop(self):
        while not self.stop.is_set():
            try:
                ref, rh = self.ref_node()
                if ref is not None and rh > 1:
                    self.fetch_ref(ref, rh - 1)
                    for nd in self.up_nodes():
                        if nd is not ref:
                            self.check_node(nd)
                    ref.checked = max(ref.checked, self.ref_checked)
                self.scan_logs()
                self.check_pending()
            except Exception:
                self.event("consistency-error", tb=traceback.format_exc()[-600:])
            self.stop.wait(15)

    def scan_logs(self):
        for nd in self.nodes:
            p = f"{nd.home}/node.log"
            try:
                if os.path.getsize(p) < nd.log_off:
                    nd.log_off = 0
                with open(p, "rb") as f:
                    f.seek(nd.log_off)
                    data = f.read()
                    # only whole lines
                    cut = data.rfind(b"\n")
                    if cut < 0:
                        continue
                    data = data[: cut + 1]
                    nd.log_off += len(data)
            except FileNotFoundError:
                continue
            text = ANSI.sub("", data.decode(errors="replace"))
            pats = GO_FATAL if nd.kind == "go" else RS_FATAL
            for line in text.splitlines():
                low = line.lower()
                ref = sign_refusal(line)
                if ref:
                    k = f"n{nd.i} {ref}"
                    self.metrics["sign_refusals"][k] = self.metrics["sign_refusals"].get(k, 0) + 1
                    if "conflicting data" in ref and self.a.strict_sign:
                        self.fail(f"node {nd.i} ({nd.kind}) signing refusal: {line.strip()[:300]}")
                for pat in pats:
                    if pat.lower() in low:
                        self.fail(f"node {nd.i} ({nd.kind}) log: {line.strip()[:300]}")
                if nd.kind == "rs" and (" ERROR " in line or " WARN " in line):
                    lvl = "ERROR" if " ERROR " in line else "WARN"
                    msg = line.split(lvl, 1)[1].strip()
                    msg = re.sub(r"\b[0-9A-Fa-f]{16,}\b", "<hex>", msg)
                    msg = re.sub(r"=\d+", "=N", msg)[:160]
                    key = f"n{nd.i} {lvl} {msg}"
                    self.log_hits[key] = self.log_hits.get(key, 0) + 1
                if nd.kind == "go" and ("Stopping peer for error" in line or "stopping peer for error" in line):
                    m = re.search(r'err="?([^"]{0,120})', line)
                    key = f"n{nd.i} go peer error: {m.group(1) if m else line[-120:]}"
                    key = re.sub(r"\b[0-9a-f]{40}\b", "<id>", key)
                    self.log_hits[key] = self.log_hits.get(key, 0) + 1

    def check_pending(self):
        with self.pending_lock:
            todo, self.pending_txs = self.pending_txs, []
        keep = []
        idx = self.nodes[self.ref_i]
        for h, cat, t, ni in todo:
            try:
                r = get_json(f"{idx.url}/tx?hash=0x{h}", timeout=5)
            except Exception:
                r = {}
            res = r.get("result")
            if res:
                m = self.metrics[cat]
                m["included"] += 1
                if int(res["tx_result"].get("code", 0)) != 0:
                    m["failed_exec"] += 1
            elif now() - t > 300 and any(g[0] - 300 <= t <= g[1] for g in self.index_gaps):
                # Submitted while the tx-index node was down / state syncing:
                # its inclusion cannot be looked up.
                self.metrics[cat]["unverifiable"] = self.metrics[cat].get("unverifiable", 0) + 1
            elif now() - t > 300:
                self.metrics[cat]["lost"] += 1
                lk = f"lost@n{ni}"  # node that accepted the tx (CheckTx 0)
                self.metrics[cat][lk] = self.metrics[cat].get(lk, 0) + 1
                self.event("tx-lost", category=cat, node=ni, hash=h, age_s=round(now() - t))
            else:
                keep.append((h, cat, t, ni))
        with self.pending_lock:
            self.pending_txs.extend(keep)

    # ------------------------------------------------------------ workload
    def bank_loop(self):
        keys = [ctx.Key.from_seed(f"soak-load-{k}") for k in range(self.a.load_accounts)]
        accs = {}
        m = self.metrics["bank"]
        while not self.stop.is_set() and len(accs) < len(keys):
            for k in keys:
                if k.address in accs:
                    continue
                try:
                    an, seq = ctx.query_account(self.tx_node().url, k.address)
                    accs[k.address] = [an, seq, now()]
                except Exception:
                    pass
            self.stop.wait(2)
        period = 1.0 / self.a.tx_rate
        nxt = now()
        i = 0
        last_resync = now()
        while not self.stop.is_set():
            k = keys[i % len(keys)]
            to = keys[(i * 7 + 3) % len(keys)].address
            i += 1
            acc = accs[k.address]
            ups = self.up_nodes()
            if not ups:
                self.stop.wait(1)
                continue
            # Each account sticks to one node (round-robin over accounts) so
            # consecutive sequences meet the same check state; all nodes,
            # Go and Rust, receive load.
            nd = ups[((i - 1) % len(keys)) % len(ups)]
            tx = ctx.sign_tx(k, self.chain_id, acc[0], acc[1], [ctx.msg_send(k.address, to, 1 + i % 997)], 120000, 3000)
            try:
                r = ctx.broadcast(nd.url, tx, timeout=5)
                code = int(r.get("code", 0))
                if code == 0:
                    acc[1] += 1
                    m["ok"] += 1
                else:
                    rk = f"{code}@n{nd.i}"  # code and the node that rejected
                    m["rejected"][rk] = m["rejected"].get(rk, 0) + 1
                    e = re.search(r"expected (\d+), got (\d+)", r.get("log", ""))
                    if code == 32 and e:
                        acc[1] = int(e.group(1))
            except Exception:
                m["errors"] += 1
            if now() - last_resync > 60:
                last_resync = now()
                # Txs lost with a killed node leave sequence gaps that CheckTx
                # rejects as "expected n": nothing else to do. Txs stuck in
                # mempools above a gap are re-synced from the chain.
                ref, _ = self.ref_node()
                for kk in keys:
                    try:
                        an, seq = ctx.query_account(ref.url, kk.address)
                        if accs[kk.address][1] - seq > 40:
                            accs[kk.address][1] = seq
                            m["resyncs"] += 1
                    except Exception:
                        pass
            nxt += period
            d = nxt - now()
            if d > 0:
                self.stop.wait(d)
            else:
                nxt = now()

    def blobs_file(self, path, sizes):
        blobs, shares = [], 0
        for n, size in enumerate(sizes):
            ns = f"{self.rng.randrange(1, 0xFFFF):04x}".ljust(20, "0")
            data = os.urandom(size)
            blobs.append({"namespaceID": "0x" + ns, "blob": "0x" + data.hex()})
            shares += 1 if size <= 478 else 1 + -(-(size - 478) // 482)
        json.dump({"Blobs": blobs}, open(path, "w"))
        return int((120000 + 70 * 10 * len(sizes) + shares * 512 * 9) * 1.2)

    def pfb(self, acct, sizes):
        f = f"{self.dir}/pfb-{acct}.json"
        gas = self.blobs_file(f, sizes)
        nd = self.tx_node()
        out = self.cli(
            ["tx", "blob", "pay-for-blob", "--input-file", f, "--from", acct, "--chain-id", self.chain_id,
             "--node", f"tcp://127.0.0.1:{nd.rpc}", "--gas", str(gas), "--fees", f"{gas // 100 + 1}utia", "-y", "-o", "json"],
            timeout=300,
        )
        os.unlink(f)
        m = self.metrics["pfb"]
        try:
            d = json.loads(out[out.index("{"):])
            code, h = int(d.get("code", 0)), d.get("txhash", "")
        except Exception:
            code, h = -1, ""
        if code == 0 and h:
            m["submitted"] += 1
            with self.pending_lock:
                self.pending_txs.append((h, "pfb", now(), nd.i))
        else:
            k = str(code) if code != -1 else "err:" + out.strip()[-100:]
            m["rejected"][k] = m["rejected"].get(k, 0) + 1

    def pfb_loop(self):
        accts = [f"blob{i}" for i in range(1, 9)]
        last_big = now()
        while not self.stop.is_set():
            big = now() - last_big > self.a.big_every
            if big:
                last_big = now()
                # Near the max: 256x256 squares hold 65,536 shares (~31 MB);
                # alternate ~15 MB rounds of 8 x 1.9 MB and ~22 MB rounds of
                # 3 x 7.3 MB blobs (max tx size 8 MiB).
                plans = os.environ.get("SOAK_BIG_PLANS")
                if plans:
                    # "<txs>x<bytes>:<weight>,...", e.g. 3x7300000:1,3x1900000:3
                    opts = [(int(a.split("x")[0]), int(a.split("x")[1]), float(w))
                            for a, w in (x.split(":") for x in plans.split(","))]
                    n_tx, size, _ = self.rng.choices(opts, weights=[o[2] for o in opts])[0]
                    plan = [[size]] * n_tx
                else:
                    p_max = float(os.environ.get("SOAK_P_MAX_ROUND", "0.5"))
                    plan = [[7_300_000]] * 3 if self.rng.random() < p_max else [[1_900_000]] * 8
            else:
                sizes = [int(x) for x in os.environ.get("SOAK_PFB_SIZES", "1,478,5000,40000,200000,600000").split(",")]
                plan = [[self.rng.choice(sizes) for _ in range(self.rng.randint(1, 3))]
                        for _ in range(self.rng.randint(1, 4))]
            total = sum(sum(p) for p in plan)
            self.metrics["pfb"]["max_round_bytes"] = max(self.metrics["pfb"]["max_round_bytes"], total)
            ths = [threading.Thread(target=self.pfb, args=(accts[k], plan[k])) for k in range(len(plan))]
            for t in ths:
                t.start()
            for t in ths:
                t.join()
            if big:
                self.event("pfb-big-round", bytes=total, txs=len(plan))
            self.stop.wait(self.rng.uniform(8, 20))

    def valopers(self):
        if not hasattr(self, "_valopers"):
            self._valopers = [self.addr(f"val{i}", f"{self.dir}/n{i}", val=True) for i in self.validator_nodes]
        return self._valopers

    def staking_loop(self):
        vs = self.valopers()
        accts = ["stake1", "stake2", "stake3"]
        delegated = {a: [] for a in accts}
        while not self.stop.is_set():
            a = self.rng.choice(accts)
            op = self.rng.random()
            if op < 0.5 or not delegated[a]:
                v = self.rng.choice(vs)
                mx = int(os.environ.get("SOAK_DELEGATE_MAX_TIA", "100"))
                code, _ = self.tx("staking", a, ["staking", "delegate", v, f"{self.rng.randint(1, mx) * 1_000_000}utia"])
                if code == 0:
                    delegated[a].append(v)
            elif op < 0.75:
                src = self.rng.choice(delegated[a])
                dst = self.rng.choice([v for v in vs if v != src])
                self.tx("staking", a, ["staking", "redelegate", src, dst, "500000utia"], gas=600000)
            else:
                v = self.rng.choice(delegated[a])
                self.tx("staking", a, ["staking", "unbond", v, "300000utia"])
            if self.rng.random() < 0.15:
                self.tx("staking", a, ["distribution", "withdraw-all-rewards"], gas=800000)
            self.stop.wait(self.rng.uniform(10, 25))

    def gov_loop(self):
        gov_addr = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
        n = 0
        self.stop.wait(90)
        while not self.stop.is_set():
            n += 1
            yes = n % 3 != 0
            gas_per_byte = 8 + (n % 2)
            prop = {
                "messages": [{"@type": "/celestia.blob.v1.MsgUpdateBlobParams", "authority": gov_addr,
                              "params": {"gas_per_blob_byte": gas_per_byte, "gov_max_square_size": "256"}}],
                "metadata": "ipfs://soak", "deposit": "10000000utia", "title": f"soak {n}",
                "summary": f"soak proposal {n}", "expedited": False,
            }
            f = f"{self.dir}/proposal.json"
            json.dump(prop, open(f, "w"))
            code, _ = self._gov_submit(f)
            if code != 0:
                self.stop.wait(60)
                continue
            self.metrics["gov"]["proposals"] += 1
            self.stop.wait(12)
            pid = self._latest_proposal()
            if pid is None:
                continue
            for i in self.validator_nodes:
                if i in self.unbonded:
                    continue
                lk = self.key_locks["val0"] if i == 0 else threading.Lock()
                with lk:
                    out = self.cli(["tx", "gov", "vote", str(pid), "yes" if yes else "no", "--from", f"val{i}",
                                    "--chain-id", self.chain_id, "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}",
                                    "--fees", "20000utia", "--gas", "300000", "-y", "-o", "json"], f"{self.dir}/n{i}")
                    ok = '"code":0' in out.replace(" ", "")
                    self.metrics["gov"]["votes"] += ok
                    if ok and i == 0:
                        hm = re.search(r'"txhash":\s*"([0-9A-F]+)"', out)
                        if hm:
                            self.wait_included(hm.group(1), timeout=60)
            self.stop.wait(75)
            st = self._proposal_status(pid)
            want = "PROPOSAL_STATUS_PASSED" if yes else "PROPOSAL_STATUS_REJECTED"
            key = "passed" if st == "PROPOSAL_STATUS_PASSED" else "rejected" if st == "PROPOSAL_STATUS_REJECTED" else "other"
            self.metrics["gov"][key] += 1
            self.event("gov-proposal", id=pid, want=want, got=st)
            self.stop.wait(self.rng.uniform(60, 180))

    def _gov_submit(self, f):
        nd = self.tx_node()
        out = self.cli(["tx", "gov", "submit-proposal", f, "--from", "gov1", "--chain-id", self.chain_id,
                        "--node", f"tcp://127.0.0.1:{nd.rpc}", "--gas", "600000", "--fees", "20000utia", "-y", "-o", "json"])
        try:
            d = json.loads(out[out.index("{"):])
            return int(d.get("code", 0)), d.get("txhash", "")
        except Exception:
            return -1, ""

    def _latest_proposal(self):
        out = self.cli(["query", "gov", "proposals", "--node", f"tcp://127.0.0.1:{self.nodes[self.ref_i].rpc}", "-o", "json",
                        "--page-reverse", "--page-limit", "1"])
        try:
            d = json.loads(out[out.index("{"):])
            return int(d["proposals"][0]["id"])
        except Exception:
            return None

    def _proposal_status(self, pid):
        out = self.cli(["query", "gov", "proposal", str(pid), "--node", f"tcp://127.0.0.1:{self.nodes[self.ref_i].rpc}", "-o", "json"])
        try:
            d = json.loads(out[out.index("{"):])
            return (d.get("proposal") or d)["status"]
        except Exception:
            return "unknown"

    # ------------------------------------------------------------ chaos
    def in_sync(self, slack=3):
        hs = {nd.i: nd.height() for nd in self.nodes if nd.expected_up}
        if any(h is None for h in hs.values()):
            return False, hs
        return max(hs.values()) - min(hs.values()) <= slack, hs

    def wait_converged(self, label, timeout=420):
        t = now()
        while now() - t < timeout and not self.stop.is_set():
            ok, hs = self.in_sync()
            if ok:
                return round(now() - t, 1)
            time.sleep(3)
        ok, hs = self.in_sync()
        if not ok:
            self.fail(f"network did not converge {timeout}s after {label}: heights {hs}")
        return None

    def cut_node(self, i):
        self.control["cut"] = [[i, j] for j in range(self.n) if j != i]
        self.write_control()

    def heal(self):
        self.control = {"cut": [], "delay_ms": {}}
        self.write_control()

    def ev_kill_restart(self, nd):
        down = self.rng.uniform(5, 60)
        extra = [] if self.rng.random() < 0.6 else ["--no-block-sync"]
        h = nd.height()
        nd.kill(signal.SIGKILL)
        self.event("kill9", node=nd.i, at=h, down_s=round(down), restart_flags=extra)
        time.sleep(down)
        with open(f"{nd.home}/node.log", "ab") as f:
            f.write(b"--- soak restart ---\n")
        nd.start(extra)
        nd.restarts += 1
        return {"node": nd.i, "down_s": round(down), "flags": extra}

    def ev_isolate(self, nd):
        secs = self.rng.uniform(30, 90)
        self.cut_node(nd.i)
        self.event("isolate", node=nd.i, secs=round(secs))
        time.sleep(secs)
        self.heal()
        return {"node": nd.i, "secs": round(secs)}

    def ev_delay(self, nd):
        ms = self.rng.choice([100, 300, 700])
        secs = self.rng.uniform(60, 180)
        self.control["delay_ms"] = {f"{nd.i}-{j}": ms for j in range(self.n) if j != nd.i}
        self.write_control()
        self.event("delay", node=nd.i, ms=ms, secs=round(secs))
        time.sleep(secs)
        self.heal()
        return {"node": nd.i, "ms": ms, "secs": round(secs)}

    def ev_split(self):
        a = {0, 2, 5}
        secs = self.rng.uniform(20, 45)
        self.control["cut"] = [[i, j] for i in range(self.n) for j in range(self.n) if i < j and ((i in a) != (j in a))]
        self.write_control()
        self.halt_until = now() + secs + 60
        self.event("split", group_a=sorted(a), secs=round(secs))
        time.sleep(secs)
        self.heal()
        h0 = self.max_height
        t = now()
        while now() - t < 120 and self.max_height <= h0 + 2:
            time.sleep(2)
        resumed = self.max_height > h0 + 2
        if not resumed:
            self.fail("chain did not resume within 120 s after healing the split")
        return {"secs": round(secs), "resume_s": round(now() - t, 1)}

    def ev_statesync(self, nd):
        ref, rh = self.ref_node()
        if ref is None or rh < 230:
            return None
        nd.kill(signal.SIGKILL)
        data = f"{nd.home}/data"
        for e in os.listdir(data):
            if e == "priv_validator_state.json":
                continue
            p = f"{data}/{e}"
            shutil.rmtree(p) if os.path.isdir(p) else os.unlink(p)
        th = rh - 5
        thash = get_json(f"{ref.url}/commit?height={th}")["result"]["signed_header"]["commit"]["block_id"]["hash"]
        servers = ",".join(f"127.0.0.1:{x.rpc}" for x in self.up_nodes("go")[:2])
        extra = ["--statesync-rpc-servers", servers, "--statesync-trust-height", str(th),
                 "--statesync-trust-hash", thash, "--statesync-discovery-secs", "10"]
        self.event("statesync-restart", node=nd.i, trust_height=th)
        with open(f"{nd.home}/node.log", "ab") as f:
            f.write(b"--- soak state sync restart ---\n")
        nd.checked = 0
        nd.start(extra)
        nd.restarts += 1
        nd.statesyncs += 1
        t = now()
        while now() - t < 300:
            st = nd.status()
            if st and st[1] > 1 and not st[2] and st[0] >= self.max_height - 3:
                return {"node": nd.i, "trust_height": th, "base": st[1], "synced_s": round(now() - t)}
            time.sleep(3)
        self.fail(f"node {nd.i} did not finish state sync + catch up within 300 s")
        return {"node": nd.i, "trust_height": th, "failed": True}

    def chaos_loop(self):
        self.stop.wait(self.a.warmup)
        rs_vals = [nd for nd in self.nodes if nd.kind == "rs" and nd.i in self.validator_nodes]
        rs_full = [nd for nd in self.nodes if nd.kind == "rs" and nd.i not in self.validator_nodes]
        kinds = ["kill_val"] * 4 + ["kill_full"] * 2 + ["isolate"] * 3 + ["delay"] * 2 + ["split"] + ["statesync"] * 2
        while not self.chaos_stop.is_set() and now() < self.deadline - 300:
            k = self.rng.choice(kinds)
            rec = {"t": round(self.elapsed()), "h": self.max_height, "kind": k}
            try:
                if k == "kill_val":
                    rec.update(self.ev_kill_restart(self.rng.choice(rs_vals)))
                elif k == "kill_full":
                    rec.update(self.ev_kill_restart(self.rng.choice(rs_full)))
                elif k == "isolate":
                    rec.update(self.ev_isolate(self.rng.choice(self.nodes)))
                elif k == "delay":
                    rec.update(self.ev_delay(self.rng.choice(self.nodes)))
                elif k == "split":
                    rec.update(self.ev_split())
                elif k == "statesync":
                    r = self.ev_statesync(self.rng.choice(rs_vals + rs_full))
                    if r is None:
                        continue
                    rec.update(r)
            except Exception:
                self.event("chaos-error", tb=traceback.format_exc()[-600:])
                self.heal()
            rec["converge_s"] = self.wait_converged(k)
            self.metrics["chaos"].append(rec)
            self.event("chaos-done", **rec)
            self.chaos_stop.wait(self.rng.uniform(30, 90))

    # ------------------------------------------------------------ long profile: workloads
    def _work(self, name, fn):
        try:
            fn()
        except Exception:
            self.event("workload-error", w=name, tb=traceback.format_exc()[-600:])

    def _gen(self, path, args):
        """An unsigned tx (`--generate-only`) of n0's keyring for MsgExec."""
        out = self.cli(["tx", *args, "--chain-id", self.chain_id, "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}",
                        "--fees", "20000utia", "--gas", "300000", "--generate-only", "-o", "json"])
        json.dump(json.loads(out[out.index("{"):]), open(path, "w"))

    def authz_loop(self):
        """authz grants / execs / revokes and feegrant allowances / fee-granted
        txs / revokes between three accounts of n0's keyring."""
        self.stop.wait(60)
        az1, az2, az3 = (self.addr(k) for k in ("az1", "az2", "az3"))
        val = self.valopers()[0]
        f = f"{self.dir}/authz-msg.json"

        def rfc3339(secs):
            return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + secs))

        ops = {
            "grant-send": lambda: self.tx_wait("authz", "az1", ["authz", "grant", az2, "send", "--spend-limit", "50000utia",
                                                               "--expiration", str(int(time.time()) + self.rng.choice([45, 900]))], "grant-send"),
            "grant-generic": lambda: self.tx_wait("authz", "az1", ["authz", "grant", az2, "generic", "--msg-type",
                                                                  "/cosmos.staking.v1beta1.MsgDelegate"], "grant-generic"),
            "exec-send": lambda: (self._gen(f, ["bank", "send", az1, az3, f"{self.rng.randint(1, 20000)}utia", "--from", az1]),
                                  self.tx_wait("authz", "az2", ["authz", "exec", f], "exec-send")),
            "exec-delegate": lambda: (self._gen(f, ["staking", "delegate", val, "1000utia", "--from", az1]),
                                      self.tx_wait("authz", "az2", ["authz", "exec", f], "exec-delegate")),
            "revoke": lambda: self.tx_wait("authz", "az1", ["authz", "revoke", az2,
                                                           self.rng.choice(["/cosmos.bank.v1beta1.MsgSend",
                                                                            "/cosmos.staking.v1beta1.MsgDelegate"])], "revoke"),
            "fg-basic": lambda: self.tx_wait("feegrant", "az1", ["feegrant", "grant", az1, az2, "--spend-limit", "100000utia",
                                                                "--expiration", rfc3339(self.rng.choice([40, 900]))], "fg-basic"),
            "fg-periodic": lambda: self.tx_wait("feegrant", "az1", ["feegrant", "grant", az1, az2, "--spend-limit", "200000utia",
                                                                   "--period", "30", "--period-limit", "50000utia"], "fg-periodic"),
            "fg-allowed": lambda: self.tx_wait("feegrant", "az1", ["feegrant", "grant", az1, az2, "--allowed-messages",
                                                                  "/cosmos.bank.v1beta1.MsgSend", "--spend-limit", "100000utia"], "fg-allowed"),
            "fg-use": lambda: self.tx_wait("feegrant", "az2", ["bank", "send", az2, az3, "1utia", "--fee-granter", az1], "fg-use"),
            "fg-revoke": lambda: self.tx_wait("feegrant", "az1", ["feegrant", "revoke", az1, az2], "fg-revoke"),
            "fg-prune": lambda: self.tx_wait("feegrant", "az2", ["feegrant", "prune"], "fg-prune"),
        }
        weights = {"fg-use": 4, "exec-send": 3, "exec-delegate": 2}
        names = list(ops)
        while not self.stop.is_set():
            op = self.rng.choices(names, weights=[weights.get(n, 1) for n in names])[0]
            self._work("authz", ops[op])
            self.stop.wait(self.rng.uniform(10, 40))

    def utf8_loop(self):
        """Txs with invalid UTF-8 in string fields (`fuzz-diff craft`): CheckTx
        compared on a Go and a Rust node (`/check_tx`), then broadcast."""
        fd = self.a.fuzz_diff
        if not fd:
            return
        home = f"{self.dir}/n0"
        out = subprocess.run([fd, "craft", home, "list"], capture_output=True, text=True, timeout=60).stdout
        cases = []
        for line in out.splitlines():
            i, signer, what = line.split(" ", 2)
            cases.append((int(i), signer, what))
        addrs = {"val0": self.addr("val0"), "user0": self.addr("user0")}
        m = self.metrics["utf8"]
        self.stop.wait(120)
        while not self.stop.is_set():
            i, signer, what = self.rng.choice(cases)
            with self.key_locks[signer]:
                self._work("utf8", lambda: self._utf8_case(fd, home, i, signer, addrs[signer], m))
            self.stop.wait(self.rng.uniform(20, 60))

    def _utf8_case(self, fd, home, i, signer, address, m):
        nd = self.tx_node()
        an, seq = ctx.query_account(nd.url, address)
        raw = subprocess.run([fd, "craft", home, self.chain_id, str(i), str(an), str(seq)],
                             capture_output=True, text=True, timeout=60).stdout.strip()
        tx = bytes.fromhex(raw)
        self.checktx_parity(tx, f"utf8 case {i}")
        r = ctx.broadcast(nd.url, tx)
        code = int(r.get("code", 0))
        cs = m.setdefault("cases", {}).setdefault(str(i), {"ok": 0, "exec_fail": {}, "checktx_fail": {}})
        if code != 0:
            k = f"{code}@{nd.kind}"
            m["rejected"][k] = m["rejected"].get(k, 0) + 1
            cs["checktx_fail"][str(code)] = cs["checktx_fail"].get(str(code), 0) + 1
            return
        m["submitted"] += 1
        ec, _ = self.wait_included(r["hash"])
        if ec is None:
            m["lost"] += 1
            return
        m["included"] += 1
        if ec == 0:
            cs["ok"] += 1
        else:
            m["failed_exec"] += 1
            cs["exec_fail"][str(ec)] = cs["exec_fail"].get(str(ec), 0) + 1

    def checktx_parity(self, tx, label):
        """CheckTx of `tx` on a Go and a Rust node (no mempool insert)."""
        got = {}
        for kind in ("go", "rs"):
            ups = self.up_nodes(kind)
            if not ups:
                return
            nd = self.rng.choice(ups)
            try:
                r = ctx.rpc_call(nd.url, "check_tx", {"tx": base64.b64encode(tx).decode()})
            except Exception as e:
                got[kind] = f"error {str(e)[:80]}"
                continue
            got[kind] = (int(r.get("code", 0)), r.get("codespace", ""), int(r.get("gas_wanted", 0) or 0),
                         int(r.get("gas_used", 0) or 0))
        p = self.metrics["utf8"]["checktx_parity"]
        if len(got) == 2 and got["go"] == got["rs"]:
            p["same"] += 1
        else:
            p["diff"].append({"what": label, "go": got.get("go"), "rs": got.get("rs")})
            p["diff"] = p["diff"][-20:]
            self.event("checktx-diff", what=label, go=str(got.get("go")), rs=str(got.get("rs")))

    def hyperlane_loop(self):
        """Hyperlane + warp: the devnet workload's mailbox / hooks / collateral
        token set up once (ids are deterministic: router sequences start at 0),
        then outbound warp transfers, inbound messages processed through the
        noop ISM (new nonces; sometimes a replay, which fails), IGP claims."""
        self.stop.wait(90)

        def ident(name, typ, seq):
            return "0x" + (name.encode().ljust(20, b"\0") + typ.to_bytes(4, "big") + seq.to_bytes(8, "big")).hex()

        ism, merkle, igp = ident("router_ism", 0, 0), ident("router_post_dispatch", 3, 1), ident("router_post_dispatch", 4, 2)
        mailbox, token = ident("hyperlane", 0, 0), ident("router_app", 1, 0)
        remote = "0x000000000000000000000000ee00000000000000000000000000000000000001"
        steps = [
            ["hyperlane", "ism", "create-noop"],
            ["hyperlane", "hooks", "noop", "create"],
            ["hyperlane", "mailbox", "create", ism, "69420"],
            ["hyperlane", "hooks", "merkle", "create", mailbox],
            ["hyperlane", "hooks", "igp", "create", "utia"],
            ["hyperlane", "hooks", "igp", "set-destination-gas-config", igp, "1", "10000000000", "1", "100"],
            ["hyperlane", "mailbox", "set", mailbox, "--required-hook", merkle, "--default-hook", igp],
            ["warp", "create-collateral-token", mailbox, "utia"],
            ["warp", "enroll-remote-router", token, "1", remote, "50000"],
        ]
        while self.long["hyperlane_step"] < len(steps) and not self.stop.is_set():
            st = steps[self.long["hyperlane_step"]]
            ec = self.tx_wait("hyperlane", "hyp1", st, "setup:" + " ".join(st[:3]))
            if ec == 0:
                self.long["hyperlane_step"] += 1
                self.save_state()
            elif ec is not None:
                self.event("hyperlane-setup-failed", step=st, code=ec)
                return
            else:
                self.stop.wait(30)
        hyp_hex = bech32_decode(self.addr("hyp1")).hex()
        last = None
        while not self.stop.is_set():
            op = self.rng.choices(["out", "in", "replay", "claim"], weights=[4, 3, 1, 1])[0]
            if op == "out":
                self._work("hyperlane", lambda: self.tx_wait(
                    "hyperlane", "hyp1", ["warp", "transfer", token, "1", "0x" + "00" * 30 + "0abc",
                                          str(self.rng.randint(1000, 5000)), "--max-hyperlane-fee", "100000utia"], "warp-transfer"))
                self.long["hyperlane_out"] = self.long.get("hyperlane_out", 0) + 1
            elif op == "in" and self.long.get("hyperlane_out", 0) > self.long["hyperlane_nonce"]:
                n = self.long["hyperlane_nonce"]
                self.long["hyperlane_nonce"] += 1
                body = "00" * 12 + hyp_hex + f"{250:064x}"
                msg = "0x03" + f"{n:08x}{1:08x}" + remote[2:] + f"{69420:08x}" + token[2:] + body
                last = msg
                self._work("hyperlane", lambda: self.tx_wait("hyperlane", "hyp1", ["hyperlane", "mailbox", "process", mailbox, "0x", msg],
                                                             "mailbox-process"))
            elif op == "replay" and last:
                self._work("hyperlane", lambda: self.tx_wait("hyperlane", "hyp1", ["hyperlane", "mailbox", "process", mailbox, "0x", last],
                                                             "mailbox-process-replay"))
            elif op == "claim":
                self._work("hyperlane", lambda: self.tx_wait("hyperlane", "hyp1", ["hyperlane", "hooks", "igp", "claim", igp], "igp-claim"))
            self.stop.wait(self.rng.uniform(20, 60))

    # ------------------------------------------------------------ long profile: chaos
    def wipe_data(self, nd):
        data = f"{nd.home}/data"
        for e in os.listdir(data):
            if e == "priv_validator_state.json":
                continue
            p = f"{data}/{e}"
            shutil.rmtree(p) if os.path.isdir(p) else os.unlink(p)

    def go_statesync_config(self, nd, servers=None, th=None, thash=None):
        """Turns the [statesync] section of a Go node's config.toml on (with
        the given light-client servers and trust root) or off."""
        cfg = f"{nd.home}/config/config.toml"
        s = open(cfg).read()
        head, sep, rest = s.partition("[statesync]")
        body, nxt, tail = rest.partition("\n[")
        on = servers is not None

        def kv(b, k, v):
            return re.sub(rf'(?m)^{k} = .*$', f'{k} = {v}', b, count=1)
        body = kv(body, "enable", "true" if on else "false")
        if on:
            body = kv(body, "rpc_servers", f'"{servers}"')
            body = kv(body, "trust_height", str(th))
            body = kv(body, "trust_hash", f'"{thash}"')
            body = kv(body, "trust_period", '"168h0m0s"')
            body = kv(body, "discovery_time", '"10s"')
        open(cfg, "w").write(head + sep + body + nxt + tail)

    def ev_statesync2(self, nd, reason="chaos"):
        """Wipes `nd` (keeping its sign state) and state-syncs it: snapshots
        from the Rust nodes (only they take snapshots in the long profile,
        in the background), light-client RPC from one Go and one Rust node."""
        others = [x for x in self.up_nodes() if x is not nd and x.i not in self.syncing()]
        go = [x for x in others if x.kind == "go"]
        rs = [x for x in others if x.kind == "rs"]
        if not go or not rs:
            return None
        hs = [x.height() or 0 for x in others]
        rh = max(hs) if hs else 0
        if rh < 260:
            return None
        th = rh - 5
        thash = get_json(f"{go[0].url}/commit?height={th}")["result"]["signed_header"]["commit"]["block_id"]["hash"]
        servers = f"127.0.0.1:{self.rng.choice(go).rpc},127.0.0.1:{self.rng.choice(rs).rpc}"
        t_kill = now()
        nd.kill(signal.SIGKILL)
        nd.sync_pending = {"since": round(self.elapsed()), "reason": reason}
        self.save_state()
        self.wipe_data(nd)
        self.event("statesync-restart", node=nd.i, kind=nd.kind, trust_height=th, servers=servers, reason=reason)
        with open(f"{nd.home}/node.log", "ab") as f:
            f.write(b"--- soak state sync restart ---\n")
        nd.checked = 0
        if nd.kind == "go":
            self.go_statesync_config(nd, servers, th, thash)
            nd.start()
        else:
            nd.start(["--statesync-rpc-servers", servers, "--statesync-trust-height", str(th),
                      "--statesync-trust-hash", thash, "--statesync-discovery-secs", "10"])
        nd.restarts += 1
        nd.statesyncs += 1
        t = now()
        res = {"node": nd.i, "node_kind": nd.kind, "trust_height": th, "reason": reason}
        while now() - t < 600:
            st = nd.status()
            if st and st[1] > 1 and not st[2] and st[0] >= self.max_height - 3:
                res.update({"base": st[1], "synced_s": round(now() - t)})
                break
            if not nd.alive():
                break
            time.sleep(3)
        else:
            self.fail(f"node {nd.i} ({nd.kind}) did not finish state sync + catch up within 600 s")
            res["failed"] = True
        if nd.kind == "go":
            self.go_statesync_config(nd)
        nd.sync_pending = None
        if nd.i == self.ref_i:
            self.index_gaps.append([round(t_kill - 1), round(now() + 1)])
        self.save_state()
        return res

    def syncing(self):
        return {nd.i for nd in self.nodes if nd.sync_pending}

    def ev_kill2(self, nd, sig, down=None, extra=()):
        down = self.rng.uniform(5, 60) if down is None else down
        h = nd.height()
        t_kill = now()
        nd.kill(sig)
        self.event("kill", node=nd.i, kind=nd.kind, sig=int(sig), at=h, down_s=round(down), restart_flags=list(extra))
        self.chaos_stop.wait(down)
        with open(f"{nd.home}/node.log", "ab") as f:
            f.write(b"--- soak restart ---\n")
        nd.start(list(extra))
        nd.restarts += 1
        if nd.i == self.ref_i:
            self.index_gaps.append([round(t_kill - 1), round(now() + 5)])
        return {"node": nd.i, "node_kind": nd.kind, "sig": signal.Signals(sig).name, "at": h, "down_s": round(down),
                "flags": list(extra)}

    def ev_term_in_precommit_delay(self, nd):
        """SIGTERM a Rust validator while it waits out the delayed precommit
        (round step Prevote, 1.0-1.9 s after the round's start time; the
        delay ends at start + 2.1 s), then read its sign state: the fix in
        ea301ac (`enterPrecommit` returns on Quit) leaves the last signed step
        at prevote for that height / round."""
        target = None
        t_end = now() + 90
        while now() < t_end and target is None:
            try:
                rs = get_json(f"{nd.url}/consensus_state", timeout=1)["result"]["round_state"]
                h, r, st = (int(x) for x in rs["height/round/step"].split("/"))
                start = parse_rfc3339(rs["start_time"])
            except Exception:
                time.sleep(0.05)
                continue
            dt = time.time() - start
            if st == 4 and 1.0 <= dt <= 1.9:
                target = (h, r, round(dt, 2))
                break
            time.sleep(0.02)
        if target is None:
            return self.ev_kill2(nd, signal.SIGTERM)
        t_sig = now()
        nd.kill(signal.SIGTERM)
        stop_s = round(now() - t_sig, 2)
        try:
            pv = json.load(open(f"{nd.home}/data/priv_validator_state.json"))
            ph, pr, ps = int(pv["height"]), int(pv["round"]), int(pv["step"])
        except Exception:
            ph = pr = ps = -1
        m = self.metrics["precommit_stop"]
        if (ph, pr) == target[:2] and ps == 2:
            outcome = "cut_short"
        elif (ph, pr) == target[:2] and ps == 3:
            outcome = "signed_before_stop"
        else:
            outcome = "unknown"
        m[outcome] += 1
        rec = {"node": nd.i, "at": list(target), "privval_after": [ph, pr, ps], "outcome": outcome, "stop_s": stop_s}
        self.event("term-in-precommit-delay", **rec)
        down = self.rng.uniform(3, 20)
        self.chaos_stop.wait(down)
        with open(f"{nd.home}/node.log", "ab") as f:
            f.write(b"--- soak restart ---\n")
        nd.start()
        nd.restarts += 1
        # It must sign the next heights again (no refusal of its own precommit).
        time.sleep(30)
        try:
            rec["signed_last_8_after_restart"] = self.signed_recent(nd.val_addr, 8)
        except Exception:
            pass
        rec["down_s"] = round(down)
        return rec

    def ev_split2(self, a):
        secs = self.rng.uniform(20, 45)
        self.control["cut"] = [[i, j] for i in range(self.n) for j in range(self.n) if i < j and ((i in a) != (j in a))]
        self.write_control()
        self.halt_until = now() + secs + 60
        self.event("split", group_a=sorted(a), secs=round(secs))
        time.sleep(secs)
        self.heal()
        h0 = self.max_height
        t = now()
        while now() - t < 120 and self.max_height <= h0 + 2:
            time.sleep(2)
        if self.max_height <= h0 + 2:
            self.fail("chain did not resume within 120 s after healing the split")
        return {"group_a": sorted(a), "secs": round(secs), "resume_s": round(now() - t, 1)}

    def chaos_loop_long(self):
        self.stop.wait(self.a.warmup)
        vals = [nd for nd in self.nodes if nd.i in self.validator_nodes]
        rs_vals = [nd for nd in vals if nd.kind == "rs"]
        go_vals = [nd for nd in vals if nd.kind == "go"]
        rs_full = [nd for nd in self.nodes if nd.kind == "rs" and nd.i not in self.validator_nodes]
        wal_nodes = [nd for nd in rs_vals if "WAL_HEAD" in os.environ.get(f"SOAK_ENV_N{nd.i}", "")]
        kinds = {"kill_val_rs": 3, "term_val_rs": 1, "term_precommit": 3, "kill_val_go": 1, "term_val_go": 1,
                 "kill_wal_node": 2, "kill_full": 2, "long_down": 1, "isolate": 2, "delay": 2, "split": 1,
                 "statesync_rs": 2, "statesync_go": 1}
        # Events that can take a validator down (exclusive with the valset
        # cycle: with 4 validators, one unbonded and another down halts).
        val_kinds = {"kill_val_rs", "term_val_rs", "term_precommit", "kill_val_go", "term_val_go", "kill_wal_node", "split",
                     "statesync_go", "statesync_rs", "isolate", "long_down"}
        while not self.chaos_stop.is_set() and now() < self.deadline - 300:
            rec = {"t": round(self.elapsed()), "h": self.max_height}
            got_val = False
            try:
                if getattr(self, "disk_pressure", False):
                    self.disk_pressure = False
                    cands = [nd for nd in self.nodes if nd.i not in self.unbonded]
                    nd = max(cands, key=lambda x: du(f"{x.home}/data"))
                    need_val = nd.i in self.validator_nodes
                    if need_val and not self.val_lock.acquire(blocking=False):
                        continue
                    got_val = need_val
                    rec["kind"] = "disk_statesync"
                    rec["data_mb"] = round(du(f"{nd.home}/data") / 2**20)
                    r = self.ev_statesync2(nd, reason="disk")
                    if r is None:
                        self.chaos_stop.wait(10)
                        continue
                    self.metrics["disk"]["pressure_syncs"] += 1
                    rec.update(r)
                elif getattr(self, "rss_pressure", False):
                    self.rss_pressure = False
                    nd = max(self.up_nodes(), key=lambda x: x.rss_mb())
                    need_val = nd.i in self.validator_nodes
                    if need_val and not self.val_lock.acquire(blocking=False):
                        continue
                    got_val = need_val
                    rec["kind"] = "rss_restart"
                    rec["rss_mb"] = round(nd.rss_mb())
                    self.metrics["rss"]["pressure_restarts"] += 1
                    rec.update(self.ev_kill2(nd, signal.SIGTERM, down=3))
                else:
                    k = self.rng.choices(list(kinds), weights=list(kinds.values()))[0]
                    if k in val_kinds:
                        if not self.val_lock.acquire(blocking=False):
                            k = self.rng.choice(["kill_full", "isolate", "delay", "statesync_rs"])
                        else:
                            got_val = True
                    rec["kind"] = k
                    if k == "kill_val_rs":
                        rec.update(self.ev_kill2(self.rng.choice(rs_vals), signal.SIGKILL,
                                                 extra=[] if self.rng.random() < 0.7 else ["--no-block-sync"]))
                    elif k == "term_val_rs":
                        rec.update(self.ev_kill2(self.rng.choice(rs_vals), signal.SIGTERM))
                    elif k == "term_precommit":
                        rec.update(self.ev_term_in_precommit_delay(self.rng.choice(rs_vals)))
                    elif k in ("kill_val_go", "term_val_go"):
                        nd = self.rng.choice(go_vals)
                        sig = signal.SIGKILL if k == "kill_val_go" else signal.SIGTERM
                        rec.update(self.ev_kill2(nd, sig, down=self.rng.uniform(5, 40)))
                    elif k == "kill_wal_node":
                        rec.update(self.ev_kill2(self.rng.choice(wal_nodes or rs_vals),
                                                 self.rng.choice([signal.SIGKILL, signal.SIGTERM]), down=self.rng.uniform(2, 20)))
                    elif k == "kill_full":
                        rec.update(self.ev_kill2(self.rng.choice(rs_full), signal.SIGKILL))
                    elif k == "long_down":
                        # Down 5-10 min, then block sync over a few hundred heights.
                        nd = self.rng.choice(rs_full + (rs_vals if got_val else []))
                        rec.update(self.ev_kill2(nd, signal.SIGKILL, down=self.rng.uniform(300, 600)))
                    elif k == "isolate":
                        rec.update(self.ev_isolate(self.rng.choice(rs_full + (vals if got_val else []))))
                    elif k == "delay":
                        rec.update(self.ev_delay(self.rng.choice(self.nodes)))
                    elif k == "split":
                        rec.update(self.ev_split2(set(self.rng.choice([[0, 2], [0, 3], [1, 2, 4], [0, 1, 4]]))))
                    elif k == "statesync_rs":
                        r = self.ev_statesync2(self.rng.choice(rs_full * 3 + (rs_vals if got_val else [])))
                        if r is None:
                            self.chaos_stop.wait(10)
                            continue
                        rec.update(r)
                    elif k == "statesync_go":
                        # Not the tx-index node.
                        r = self.ev_statesync2(self.rng.choice([nd for nd in go_vals if nd.i != self.ref_i] or go_vals))
                        if r is None:
                            self.chaos_stop.wait(10)
                            continue
                        rec.update(r)
            except Exception:
                self.event("chaos-error", tb=traceback.format_exc()[-600:])
                self.heal()
            finally:
                if got_val:
                    self.val_lock.release()
            rec["converge_s"] = self.wait_converged(rec.get("kind", "?"))
            self.metrics["chaos"].append(rec)
            self.event("chaos-done", **rec)
            self.save_state()
            self.chaos_stop.wait(self.rng.uniform(30, 90))

    def valset_loop(self):
        vi = self.valset_node
        nd = self.nodes[vi]
        home = nd.home
        acc = self.addr(f"val{vi}", home)
        valoper = self.addr(f"val{vi}", home, val=True)
        resume_cycle = vi in self.unbonded  # the previous segment ended mid-cycle
        if not resume_cycle:
            self.stop.wait(self.a.warmup + 600)
        while not self.chaos_stop.is_set() and now() < self.deadline - 1200:
            self.val_lock.acquire()
            try:
                self._valset_cycle(vi, nd, home, acc, valoper, resume_cycle)
            finally:
                self.val_lock.release()
            resume_cycle = False
            self.stop.wait(self.rng.uniform(900, 1500))

    def _valset_cycle(self, vi, nd, home, acc, valoper, resume_cycle):
        if True:
            if resume_cycle:
                amt = self.long.get("valset_amt", 5_000_000_000)
                rec = {"t": round(self.elapsed()), "node": vi, "self_delegation": amt, "resumed_mid_cycle": True}
            else:
                out = self.cli(["query", "staking", "delegation", acc, valoper, "--node", f"tcp://127.0.0.1:{self.nodes[self.ref_i].rpc}", "-o", "json"])
                try:
                    d = json.loads(out[out.index("{"):])
                    amt = int((d.get("delegation_response") or d)["balance"]["amount"])
                except Exception:
                    self.event("valset-query-failed", out=out[-200:])
                    self.stop.wait(60)
                    return
                self.long["valset_amt"] = amt
                rec = {"t": round(self.elapsed()), "node": vi, "self_delegation": amt}
                out = self.cli(["tx", "staking", "unbond", valoper, f"{amt}utia", "--from", f"val{vi}", "--chain-id", self.chain_id,
                                "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}", "--gas", "400000", "--fees", "20000utia", "-y", "-o", "json"], home)
                self.event("valset-unbond", amount=amt, out=out.strip()[-160:])
                t = now()
                gone = None
                while now() - t < 180:
                    if not self.in_valset(nd.val_addr):
                        gone = round(now() - t)
                        break
                    time.sleep(3)
                if gone is None:
                    self.fail(f"validator {vi} still in the validator set 180 s after unbonding its self-delegation")
                self.unbonded.add(vi)
                self.save_state()
                rec["left_set_s"] = gone
                self.stop.wait(self.rng.uniform(240, 420))
            rebonded = 0
            if resume_cycle:
                out = self.cli(["query", "staking", "delegation", acc, valoper, "--node", f"tcp://127.0.0.1:{self.nodes[self.ref_i].rpc}", "-o", "json"])
                try:
                    d = json.loads(out[out.index("{"):])
                    rebonded = int((d.get("delegation_response") or d)["balance"]["amount"])
                except Exception:
                    pass
            if rebonded < amt // 2:
                out = self.cli(["tx", "staking", "delegate", valoper, f"{amt}utia", "--from", f"val{vi}", "--chain-id", self.chain_id,
                                "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}", "--gas", "400000", "--fees", "20000utia", "-y", "-o", "json"], home)
                self.event("valset-delegate", out=out.strip()[-160:])
                time.sleep(15)
            back = None
            t = now()
            # An accepted unjail can still be lost (its node killed / wiped
            # before gossiping it): resubmit once, through a Go node, and
            # record it; failing is reserved for a re-bond that never lands.
            for attempt, via in ((1, self.tx_node()), (2, self.nodes[self.ref_i])):
                out = self.cli(["tx", "slashing", "unjail", "--from", f"val{vi}", "--chain-id", self.chain_id,
                                "--node", f"tcp://127.0.0.1:{via.rpc}", "--gas", "400000", "--fees", "20000utia", "-y", "-o", "json"], home)
                self.event("valset-unjail", attempt=attempt, via=via.i, out=out.strip()[-160:])
                t1 = now()
                while now() - t1 < 120:
                    if self.in_valset(nd.val_addr):
                        back = round(now() - t)
                        break
                    time.sleep(3)
                if back is not None:
                    break
                rec["unjail_retried_after_loss_via_node"] = via.i
            if back is None:
                self.fail(f"validator {vi} did not rejoin the validator set after re-bonding + two unjail attempts")
            else:
                self.unbonded.discard(vi)
                # It must sign again.
                time.sleep(60)
                signed = self.signed_recent(nd.val_addr, 20)
                rec["signed_last_20_after_rejoin"] = signed
                if signed < 10:
                    self.fail(f"validator {vi} signed only {signed} of the last 20 blocks after rejoining")
            rec["rejoined_s"] = back
            self.metrics["valset"].append(rec)
            self.event("valset-cycle", **rec)

    def in_valset(self, addr):
        ref, _ = self.ref_node()
        vals = get_json(f"{ref.url}/validators?per_page=100")["result"]["validators"]
        return any(v["address"].upper() == addr for v in vals)

    def signed_recent(self, addr, n):
        ref, rh = self.ref_node()
        c = 0
        for h in range(rh - n + 1, rh + 1):
            sigs = get_json(f"{ref.url}/commit?height={h}")["result"]["signed_header"]["commit"]["signatures"]
            c += any((s.get("validator_address") or "").upper() == addr and s["block_id_flag"] == 2 for s in sigs)
        return c

    # ------------------------------------------------------------ run
    def run(self):
        for nd in self.nodes:
            if nd.kill_stale():
                self.event("killed-stale-node", node=nd.i)
        self.setup()
        self.event("start" if not self.a.resume else "resume", nodes=[nd.kind for nd in self.nodes], secs=self.a.secs,
                   segment=self.segment, cum_elapsed=round(self.elapsed()))
        resync = [nd for nd in self.nodes if nd.sync_pending]
        for nd in self.nodes:
            if nd in resync:
                continue
            threading.Thread(target=nd.start, daemon=True).start()
        time.sleep(15)
        t = now()
        limit = 600 if self.a.resume else 180
        while now() - t < limit:
            ok, hs = self.in_sync()
            if ok and min(hs.values()) >= 3 and (not self.a.resume or max(hs.values()) > self.max_height):
                break
            time.sleep(3)
        else:
            self.fail(f"network did not {'resume' if self.a.resume else 'start'}: {self.in_sync()[1]}")
        if self.a.resume:
            # The machine restart killed every node at once: a whole-network
            # crash / restart (every node replays its WAL).
            rec = {"t": round(self.elapsed()), "h": self.max_height, "kind": "crash_all", "segment": self.segment,
                   "restart_s": round(now() - t, 1)}
            for nd in self.nodes:
                nd.restarts += 1
            for nd in resync:  # killed mid state sync: start over
                self.event("statesync-redo", node=nd.i)
                rec.setdefault("statesync_redone", []).append(self.ev_statesync2(nd, reason="resume"))
            rec["converge_s"] = self.wait_converged("crash_all", timeout=600)
            self.metrics["chaos"].append(rec)
            self.event("chaos-done", **rec)
        self.last_progress = now()
        self.save_state()
        work = [self.monitor_loop, self.consistency_loop, self.bank_loop, self.pfb_loop, self.staking_loop,
                self.gov_loop]
        if self.a.long:
            work += [self.authz_loop, self.utf8_loop, self.hyperlane_loop]
        if self.a.mem_sample:
            work.append(self.mem_loop)
        chaos = [] if self.a.no_chaos else [self.chaos_loop_long if self.a.long else self.chaos_loop, self.valset_loop]
        threads = [threading.Thread(target=f, daemon=True, name=f.__name__) for f in work]
        chaos_threads = [threading.Thread(target=f, daemon=True, name=f.__name__) for f in chaos]
        for th in threads + chaos_threads:
            th.start()
        last_report = now()
        while not (now() >= self.deadline and self.max_height >= self.a.min_heights):
            time.sleep(10)
            if now() - last_report > 300:
                last_report = now()
                self.write_report(final=False)
            if now() > self.deadline + 3600:
                self.fail(f"only {self.max_height} heights after {self.a.secs + 3600}s")
                break
        self.event("draining")
        self.chaos_stop.set()
        for th in chaos_threads[:1]:
            th.join(timeout=900)
        self.heal()
        for nd in self.nodes:
            if not nd.alive():
                nd.start()
        self.stop.set()  # workload stops
        for th in threads:
            th.join(timeout=120)
        self.stop.clear()
        self.wait_converged("end of soak", timeout=600)
        # Final full pass.
        ref, rh = self.ref_node()
        self.fetch_ref(ref, rh - 1)
        for nd in self.nodes:
            if nd is not ref:
                self.check_node(nd, limit=10**9)
        self.scan_logs()
        self.stop.set()
        self.done = True
        self.save_state()
        self.write_report(final=True)
        for nd in self.nodes:
            nd.kill(signal.SIGTERM)
        self.proxy.kill()
        return 0 if not self.failures else 1

    def write_report(self, final):
        rep = {
            "final": final,
            "elapsed_s": round(self.elapsed()),
            "heights": self.max_height,
            "blocks": self.blocks,
            "proposed_by_node": {str(k): v for k, v in sorted(self.proposed.items())},
            "nodes": [],
            "stalls": [{k: v for k, v in s.items() if not k.startswith("_")} for s in self.stalls],
            "metrics": self.metrics,
            "failures": self.failures,
            "log_hits": dict(sorted(self.log_hits.items(), key=lambda x: -x[1])[:60]),
        }
        for nd in self.nodes:
            growth = []
            for t0, pts in nd.lifetimes:
                pts = [p for p in pts if p[0] - t0 > 600]  # skip warm-up
                if len(pts) >= 20:
                    growth.append({"start_t": round(t0 - self.t0), "mins": round((pts[-1][0] - pts[0][0]) / 60),
                                   "rss_first": round(pts[0][1]), "rss_last": round(pts[-1][1]),
                                   "slope_mb_per_h": round(slope(pts) * 3600, 1)})
            rep["nodes"].append({
                "node": nd.i, "kind": nd.kind, "validator": nd.i in self.validator_nodes,
                "restarts": nd.restarts, "statesyncs": nd.statesyncs, "max_rss_mb": round(nd.max_rss),
                "disk_mb": round(du(f"{nd.home}/data") / 2**20), "checked_upto": nd.checked,
                "rss_lifetimes": growth,
            })
        json.dump(rep, open(f"{self.dir}/report.json", "w"), indent=1)
        md = [f"# Soak report ({'final' if final else 'interim'})", "",
              f"- wall clock: {rep['elapsed_s'] / 3600:.2f} h, heights: {rep['heights']}, txs: {self.blocks['txs']}, "
              f"max block {self.blocks['max_bytes'] / 2**20:.1f} MiB, blocks > 8 MiB: {self.blocks['over_8mb']}",
              f"- bank: {self.metrics['bank']}", f"- pfb: {self.metrics['pfb']}", f"- staking: {self.metrics['staking']}",
              f"- gov: {self.metrics['gov']}", f"- chaos events: {len(self.metrics['chaos'])}, valset cycles: {len(self.metrics['valset'])}",
              f"- chaos by kind: {chaos_kinds(self.metrics['chaos'])}",
              f"- segments: {self.segment}", f"- authz: {self.metrics['authz']}", f"- feegrant: {self.metrics['feegrant']}",
              f"- hyperlane: {self.metrics['hyperlane']}", f"- utf8: {self.metrics['utf8']}",
              f"- precommit-delay stops: {self.metrics['precommit_stop']}", f"- signing refusals: {self.metrics['sign_refusals']}",
              f"- WAL: {self.metrics['wal']}", f"- disk: {self.metrics['disk']}", f"- rss: {self.metrics['rss']}",
              f"- stalls: {rep['stalls']}", f"- proposed by node: {rep['proposed_by_node']}", "",
              "| node | kind | restarts | state syncs | max RSS MB | disk MB | RSS growth (MB/h per lifetime) |",
              "|---|---|---|---|---|---|---|"]
        for n in rep["nodes"]:
            md.append(f"| {n['node']} | {n['kind']}{' val' if n['validator'] else ''} | {n['restarts']} | {n['statesyncs']} | "
                      f"{n['max_rss_mb']} | {n['disk_mb']} | {', '.join(str(g['slope_mb_per_h']) for g in n['rss_lifetimes'])} |")
        md += ["", "## Failures", ""] + [f"- {f}" for f in self.failures] + ["", "## Chaos", ""]
        md += [f"- {json.dumps(c)}" for c in self.metrics["chaos"]]
        md += ["", "## Valset cycles", ""] + [f"- {json.dumps(c)}" for c in self.metrics["valset"]]
        md += ["", "## Rust log warnings/errors (top)", ""] + [f"- {v} x {k}" for k, v in rep["log_hits"].items()]
        open(f"{self.dir}/report.md", "w").write("\n".join(md) + "\n")


def parse_rfc3339(ts):
    """Unix time of an RFC 3339 UTC timestamp with up to nanoseconds."""
    m = re.match(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d+))?Z", ts)
    import calendar
    base = calendar.timegm(time.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S"))
    return base + (float("0." + m.group(2)) if m.group(2) else 0.0)


def bech32_decode(addr):
    data = [ctx.CHARSET.index(c) for c in addr.rsplit("1", 1)[1][:-6]]
    acc, bits, out = 0, 0, bytearray()
    for v in data:
        acc = (acc << 5) | v
        bits += 5
        if bits >= 8:
            bits -= 8
            out.append((acc >> bits) & 0xFF)
    return bytes(out)


def sign_refusal(line):
    """A privval refusal in a node log, normalized: Go `failed signing vote`
    / `failed signing proposal`, Rust `failed signing ...`."""
    low = line.lower()
    if "failed signing" not in low and "error signing" not in low:
        return None
    what = "proposal" if "proposal" in low else "vote"
    m = re.search(r"(conflicting data|step regression|round regression|height regression|"
                  r"no sign bytes|unexpected step)", low)
    return f"{what}: {m.group(1) if m else 'other'}"


def chaos_kinds(chaos):
    out = {}
    for c in chaos:
        out[c.get("kind", "?")] = out.get(c.get("kind", "?"), 0) + 1
    return dict(sorted(out.items()))


def slope(pts):
    n = len(pts)
    mx = sum(p[0] for p in pts) / n
    my = sum(p[1] for p in pts) / n
    den = sum((p[0] - mx) ** 2 for p in pts)
    return sum((p[0] - mx) * (p[1] - my) for p in pts) / den if den else 0.0


def du(path):
    total = 0
    for dp, _, fs in os.walk(path):
        for f in fs:
            try:
                total += os.lstat(os.path.join(dp, f)).st_size
            except OSError:
                pass
    return total


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dir")
    p.add_argument("--off", type=int, default=19000)
    p.add_argument("--rs")
    p.add_argument("--root")
    p.add_argument("--chain-id", default="rs-soak-1")
    p.add_argument("--secs", type=int, default=7200)
    p.add_argument("--min-heights", type=int, default=1500)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--load-accounts", type=int, default=48)
    p.add_argument("--tx-rate", type=float, default=float(os.environ.get("SOAK_TX_RATE", "25")))
    p.add_argument("--big-every", type=float, default=float(os.environ.get("SOAK_BIG_EVERY", "300")))
    p.add_argument("--warmup", type=float, default=float(os.environ.get("SOAK_WARMUP", "180")))
    p.add_argument("--print-load-addresses", action="store_true")
    p.add_argument("--validators", default="0,1,2,3,4", help="node indices holding validator keys")
    p.add_argument("--ref-node", type=int, default=5, help="Go node with the tx index (queries, hash reference)")
    p.add_argument("--no-chaos", action="store_true", help="workload only: no kills, cuts or valset cycles")
    p.add_argument("--long", action="store_true",
                   help="long profile: authz / feegrant / invalid-UTF-8 / hyperlane workloads, Go + Rust chaos, WAL and disk checks")
    p.add_argument("--resume", action="store_true",
                   help="continue a soak in --dir from state.json (restart every node from its home)")
    p.add_argument("--valset-node", type=int, default=4, help="validator that unbonds / re-bonds in the valset cycle")
    p.add_argument("--fuzz-diff", default="", help="fuzz-diff binary (invalid-UTF-8 txs via `fuzz-diff craft`)")
    p.add_argument("--strict-sign", action="store_true", help="fail on any privval `conflicting data` refusal")
    p.add_argument("--mem-sample", type=float, default=0.0,
                   help="seconds between memory samples (mem.jsonl: RSS, heap, threads, fds, disk, caches); 0 = off")
    a = p.parse_args()
    if a.print_load_addresses:
        for k in range(a.load_accounts):
            print(ctx.Key.from_seed(f"soak-load-{k}").address)
        return 0
    s = Soak(a)

    def on_signal(*_):
        if s.a.long:
            s.event("segment-interrupted")
        else:
            s.fail("interrupted")
        s.stop.set()
        s.save_state()
        s.write_report(final=True)
        for nd in s.nodes:
            nd.kill(signal.SIGTERM)
        try:
            s.proxy.kill()
        except Exception:
            pass
        os._exit(1)

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    try:
        rc = s.run()
    except Exception:
        s.fail("orchestrator crashed: " + traceback.format_exc()[-800:])
        rc = 1
    finally:
        for nd in s.nodes:
            nd.kill(signal.SIGTERM)
        try:
            s.proxy.kill()
        except Exception:
            pass
    s.write_report(final=True)
    print(open(f"{s.dir}/report.md").read())
    return rc


if __name__ == "__main__":
    sys.exit(main())
