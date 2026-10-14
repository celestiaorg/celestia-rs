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
        self.proc = None
        self.expected_up = False
        self.restarts = 0
        self.statesyncs = 0
        self.checked = 0
        self.max_rss = 0
        self.lifetimes = []  # [(start_t, [(t, rss_mb)])]
        self.log_off = 0
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
            else:
                env.setdefault("RUST_LOG", "info")
                cmd = [
                    self.soak.rs, "start", "--home", self.home,
                    "--p2p-laddr", f"127.0.0.1:{self.p2p}",
                    "--persistent-peers", self.peers(),
                    "--moniker", f"node{self.i}", "--allow-duplicate-ip", *extra,
                ]
            self.proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env, start_new_session=True)
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
        self.validator_nodes = [0, 1, 2, 3, 4]
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

    def proxy_port(self, i, j):
        return 26656 + self.off + 500 + 10 * i + j

    def write_control(self):
        tmp = f"{self.dir}/proxy-control.json.tmp"
        json.dump(self.control, open(tmp, "w"))
        os.replace(tmp, f"{self.dir}/proxy-control.json")

    def up_nodes(self, kind=None):
        return [nd for nd in self.nodes if nd.expected_up and nd.alive() and (kind is None or nd.kind == kind)]

    def ref_node(self):
        best, bh = None, -1
        for nd in self.up_nodes("go"):
            h = nd.height()
            if h is not None and (h > bh or (h == bh and nd.i == 5)):
                best, bh = nd, h
        return best, bh

    def tx_node(self):
        ups = self.up_nodes()
        return self.rng.choice(ups) if ups else self.nodes[5]

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

    def tx(self, category, key, args, home=None, gas=400000, fees=20000):
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
            with self.pending_lock:
                self.pending_txs.append((h, category, now()))
        else:
            k = str(code) if code != -1 else ("err:" + out.strip()[:80])
            m["rejected"][k] = m["rejected"].get(k, 0) + 1
        return code, h

    # ------------------------------------------------------------ setup
    def setup(self):
        routes = []
        for i in range(self.n):
            for j in range(self.n):
                if i != j:
                    routes.append({"listen": self.proxy_port(i, j), "target": self.nodes[j].p2p, "src": i, "dst": j})
        json.dump(routes, open(f"{self.dir}/proxy-routes.json", "w"))
        self.write_control()
        self.proxy = subprocess.Popen(
            [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "netproxy.py"),
             f"{self.dir}/proxy-routes.json", f"{self.dir}/proxy-control.json"],
            stdout=open(f"{self.dir}/proxy.log", "a"), stderr=subprocess.STDOUT, start_new_session=True,
        )
        time.sleep(1)
        for nd in self.nodes:
            cfg = f"{nd.home}/config/config.toml"
            s = open(cfg).read()
            s = re.sub(r'(?m)^persistent_peers = .*$', f'persistent_peers = "{nd.peers()}"', s)
            if nd.i == 5:
                s = re.sub(r'(?m)^indexer = .*$', 'indexer = "kv"', s, count=1)
            open(cfg, "w").write(s)

    # ------------------------------------------------------------ monitor
    def monitor_loop(self):
        last_rss = last_disk = 0.0
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
                    for nd in self.nodes:
                        r = nd.rss_mb()
                        nd.max_rss = max(nd.max_rss, r)
                        if r and nd.lifetimes:
                            nd.lifetimes[-1][1].append((t, r))
                        row.append(f"{r:.1f}")
                    rss_f.write(f"{t - self.t0:.0f},{self.max_height}," + ",".join(row) + "\n")
                    rss_f.flush()
                if t - last_disk >= 60:
                    last_disk = t
                    row = [f"{du(f'{nd.home}/data') / 2**20:.1f}" for nd in self.nodes]
                    disk_f.write(f"{t - self.t0:.0f},{self.max_height}," + ",".join(row) + "\n")
                    disk_f.flush()
            except Exception:
                self.event("monitor-error", tb=traceback.format_exc()[-500:])
            self.stop.wait(5)

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
                self.blocks["txs"] += ntx
                self.blocks["count"] += 1
                self.blocks["max_bytes"] = max(self.blocks["max_bytes"], size)
                self.blocks["over_8mb"] += size > 8 * 2**20
                p = self.by_addr.get(hdr["proposer_address"].upper(), -1)
                self.proposed[p] = self.proposed.get(p, 0) + 1
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
        idx = self.nodes[5]
        for h, cat, t in todo:
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
            elif now() - t > 300:
                self.metrics[cat]["lost"] += 1
            else:
                keep.append((h, cat, t))
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
                    m["rejected"][str(code)] = m["rejected"].get(str(code), 0) + 1
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
                self.pending_txs.append((h, "pfb", now()))
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
                p_max = float(os.environ.get("SOAK_P_MAX_ROUND", "0.5"))
                plan = [[7_300_000]] * 3 if self.rng.random() < p_max else [[1_900_000]] * 8
            else:
                plan = [[self.rng.choice([1, 478, 5_000, 40_000, 200_000, 600_000]) for _ in range(self.rng.randint(1, 3))]
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
                code, _ = self.tx("staking", a, ["staking", "delegate", v, f"{self.rng.randint(1, 100) * 1_000_000}utia"])
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
                out = self.cli(["tx", "gov", "vote", str(pid), "yes" if yes else "no", "--from", f"val{i}",
                                "--chain-id", self.chain_id, "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}",
                                "--fees", "20000utia", "--gas", "300000", "-y", "-o", "json"], f"{self.dir}/n{i}")
                self.metrics["gov"]["votes"] += '"code":0' in out.replace(" ", "")
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
        out = self.cli(["query", "gov", "proposals", "--node", f"tcp://127.0.0.1:{self.nodes[5].rpc}", "-o", "json",
                        "--page-reverse", "--page-limit", "1"])
        try:
            d = json.loads(out[out.index("{"):])
            return int(d["proposals"][0]["id"])
        except Exception:
            return None

    def _proposal_status(self, pid):
        out = self.cli(["query", "gov", "proposal", str(pid), "--node", f"tcp://127.0.0.1:{self.nodes[5].rpc}", "-o", "json"])
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

    def valset_loop(self):
        nd = self.nodes[4]
        home = nd.home
        acc = self.addr("val4", home)
        valoper = self.addr("val4", home, val=True)
        self.stop.wait(self.a.warmup + 600)
        while not self.chaos_stop.is_set() and now() < self.deadline - 1200:
            out = self.cli(["query", "staking", "delegation", acc, valoper, "--node", f"tcp://127.0.0.1:{self.nodes[5].rpc}", "-o", "json"])
            try:
                d = json.loads(out[out.index("{"):])
                amt = int((d.get("delegation_response") or d)["balance"]["amount"])
            except Exception:
                self.event("valset-query-failed", out=out[-200:])
                self.stop.wait(60)
                continue
            rec = {"t": round(self.elapsed()), "node": 4, "self_delegation": amt}
            out = self.cli(["tx", "staking", "unbond", valoper, f"{amt}utia", "--from", "val4", "--chain-id", self.chain_id,
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
                self.fail("validator 4 still in the validator set 180 s after unbonding its self-delegation")
            self.unbonded.add(4)
            rec["left_set_s"] = gone
            self.stop.wait(self.rng.uniform(240, 420))
            out = self.cli(["tx", "staking", "delegate", valoper, f"{amt}utia", "--from", "val4", "--chain-id", self.chain_id,
                            "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}", "--gas", "400000", "--fees", "20000utia", "-y", "-o", "json"], home)
            self.event("valset-delegate", out=out.strip()[-160:])
            time.sleep(15)
            out = self.cli(["tx", "slashing", "unjail", "--from", "val4", "--chain-id", self.chain_id,
                            "--node", f"tcp://127.0.0.1:{self.tx_node().rpc}", "--gas", "400000", "--fees", "20000utia", "-y", "-o", "json"], home)
            self.event("valset-unjail", out=out.strip()[-160:])
            t = now()
            back = None
            while now() - t < 180:
                if self.in_valset(nd.val_addr):
                    back = round(now() - t)
                    break
                time.sleep(3)
            if back is None:
                self.fail("validator 4 did not rejoin the validator set within 180 s of re-bonding + unjail")
            else:
                self.unbonded.discard(4)
                # It must sign again.
                time.sleep(60)
                signed = self.signed_recent(nd.val_addr, 20)
                rec["signed_last_20_after_rejoin"] = signed
                if signed < 10:
                    self.fail(f"validator 4 signed only {signed} of the last 20 blocks after rejoining")
            rec["rejoined_s"] = back
            self.metrics["valset"].append(rec)
            self.event("valset-cycle", **rec)
            self.stop.wait(self.rng.uniform(900, 1500))

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
        self.setup()
        self.event("start", nodes=[nd.kind for nd in self.nodes], secs=self.a.secs)
        for nd in self.nodes:
            threading.Thread(target=nd.start, daemon=True).start()
        time.sleep(15)
        t = now()
        while now() - t < 180:
            ok, hs = self.in_sync()
            if ok and min(hs.values()) >= 3:
                break
            time.sleep(3)
        else:
            self.fail(f"network did not start: {self.in_sync()[1]}")
        self.last_progress = now()
        threads = [threading.Thread(target=f, daemon=True, name=f.__name__) for f in (
            self.monitor_loop, self.consistency_loop, self.bank_loop, self.pfb_loop, self.staking_loop,
            self.gov_loop, self.chaos_loop, self.valset_loop)]
        for th in threads:
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
        threads[6].join(timeout=900)
        self.heal()
        for nd in self.nodes:
            if not nd.alive():
                nd.start()
        self.stop.set()  # workload stops
        for th in threads[:6]:
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
    a = p.parse_args()
    if a.print_load_addresses:
        for k in range(a.load_accounts):
            print(ctx.Key.from_seed(f"soak-load-{k}").address)
        return 0
    s = Soak(a)

    def on_signal(*_):
        s.fail("interrupted")
        s.stop.set()
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
