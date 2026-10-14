#!/usr/bin/env python3
"""Chaos TCP proxy for the soak devnet (`../soak.sh`).

Every directed node pair (i -> j) gets its own listener: node i's
persistent peer entry for node j points at it, and it forwards to node j's
real P2P port. Because each listener carries exactly one directed link,
the proxy can cut or slow down arbitrary links without iptables.

usage: netproxy.py <routes.json> <control.json>

routes.json: [{"listen": port, "target": port, "src": i, "dst": j}, ...]

control.json (re-read every 0.5 s; missing or invalid = all links up):
  {"cut": [[i, j], ...],          # links that drop their connections and
                                   # refuse new ones (both directions of a
                                   # TCP stream: the pair is symmetric)
   "delay_ms": {"i-j": ms, ...}}   # added one-way latency per chunk on
                                   # the link's streams (both directions)

A link is matched symmetrically: cutting [i, j] also cuts the j -> i
listener, so a single entry isolates the pair. Stats (bytes / connections
per link) are written to <control.json>.stats every 10 s.
"""

import asyncio
import json
import os
import sys
import time

ROUTES = json.load(open(sys.argv[1]))
CONTROL = sys.argv[2]

state = {"cut": set(), "delay": {}}
conns = {}  # (src, dst) -> set of (reader-task-pair close callbacks)
stats = {}


def pair(a, b):
    return (min(a, b), max(a, b))


def load_control():
    try:
        c = json.load(open(CONTROL))
    except Exception:
        c = {}
    cut = {pair(int(a), int(b)) for a, b in c.get("cut", [])}
    delay = {}
    for k, v in (c.get("delay_ms") or {}).items():
        a, b = k.split("-")
        delay[pair(int(a), int(b))] = float(v) / 1000.0
    return cut, delay


async def pump(reader, writer, key, direction):
    """Copies reader -> writer, adding the link's current delay per chunk
    while preserving order."""
    q = asyncio.Queue()

    async def sender():
        while True:
            item = await q.get()
            if item is None:
                break
            due, data = item
            now = time.monotonic()
            if due > now:
                await asyncio.sleep(due - now)
            writer.write(data)
            await writer.drain()
            s = stats.setdefault(f"{key[0]}-{key[1]}", {"bytes": 0, "conns": 0})
            s["bytes"] += len(data)

    t = asyncio.create_task(sender())
    try:
        while True:
            data = await reader.read(65536)
            if not data:
                break
            d = state["delay"].get(pair(*key), 0.0)
            await q.put((time.monotonic() + d, data))
    except Exception:
        pass
    finally:
        await q.put(None)
        try:
            await t
        except Exception:
            pass
        try:
            writer.close()
        except Exception:
            pass


def make_handler(route):
    key = (route["src"], route["dst"])

    async def handle(creader, cwriter):
        if pair(*key) in state["cut"]:
            cwriter.close()
            return
        try:
            ureader, uwriter = await asyncio.open_connection("127.0.0.1", route["target"])
        except Exception:
            cwriter.close()
            return
        s = stats.setdefault(f"{key[0]}-{key[1]}", {"bytes": 0, "conns": 0})
        s["conns"] += 1
        live = conns.setdefault(pair(*key), set())
        entry = (cwriter, uwriter)
        live.add(entry)
        try:
            await asyncio.gather(
                pump(creader, uwriter, key, "fwd"), pump(ureader, cwriter, key, "back")
            )
        finally:
            live.discard(entry)
            for w in entry:
                try:
                    w.close()
                except Exception:
                    pass

    return handle


async def control_loop():
    last_stats = 0.0
    while True:
        cut, delay = load_control()
        newly = cut - state["cut"]
        state["cut"], state["delay"] = cut, delay
        for p in newly:
            for entry in list(conns.get(p, ())):
                for w in entry:
                    try:
                        w.transport.abort()
                    except Exception:
                        pass
        if time.monotonic() - last_stats > 10:
            last_stats = time.monotonic()
            try:
                tmp = CONTROL + ".stats.tmp"
                json.dump(stats, open(tmp, "w"))
                os.replace(tmp, CONTROL + ".stats")
            except Exception:
                pass
        await asyncio.sleep(0.5)


async def main():
    servers = []
    for r in ROUTES:
        servers.append(await asyncio.start_server(make_handler(r), "127.0.0.1", r["listen"]))
    print(f"netproxy: {len(servers)} links up", flush=True)
    await control_loop()


asyncio.run(main())
