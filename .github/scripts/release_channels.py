#!/usr/bin/env python3
"""Moves celestia-rs release channels forward as releases soak.

A channel (corto, mocha, mainnet, ...) names the one release that nodes on
that network should run. .github/release-channels.json is the policy: each
channel's source (`releases`, or the channel before it) and how many hours a
release must have been in that source before the channel takes it. The
state lives on the `release-channels` branch:

  channels.json        every channel's tag and history, frozen channels,
                       yanked tags
  channels/<name>      the channel's tag alone, for shell clients
                       (scripts/install.sh --channel, the updaters)

Rules for one `promote` pass, per channel in policy order:

- A frozen channel does not move.
- Candidates are releases (or entries in the source channel's history) at
  least `soak_hours` old, not yanked, and not newer than what the source
  channel runs now (so a rollback of the source holds back the channels
  after it).
- The channel takes the newest candidate, by its commit's position on
  main, and only if it is newer than its current tag: promotion never
  downgrades. Automatic releases builds run in parallel and can publish out
  of order, so publication time only says when a release is available. A
  release's position is its `position` field (the workflow adds it from
  `git rev-list --count <tag>`), else the `main.<n>` in its tag; a release
  with neither is never taken.

main is linear, so a release that entered the source N hours ago has been
running there, inside every later release, for N hours. That keeps a busy
first channel from starving the channels after it.

Usage (each command writes the state back to --dir):

  release_channels.py --dir D --policy P promote --releases R [--now ISO]
  release_channels.py --dir D --policy P freeze|unfreeze CHANNEL
  release_channels.py --dir D --policy P yank|unyank TAG
  release_channels.py --dir D --policy P pin CHANNEL TAG   # also freezes

R is the JSON array GitHub's list-releases API returns, optionally with a
`position` per release. Changes print one per line.
"""

import argparse
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import re
import sys

TAG = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$")
# An automatic release's tag ends in its commit's position on main.
MAIN_POSITION = re.compile(r"[.-]main\.([0-9]+)$")
RELEASES = "releases"
HISTORY_LIMIT = 1000


def parse_time(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def format_time(value):
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def check_policy(policy):
    known = {RELEASES}
    for c in policy["channels"]:
        name, source = c["name"], c["source"]
        if name in known or not re.fullmatch(r"[a-z0-9-]+", name):
            raise ValueError(f"invalid or duplicate channel name: {name}")
        if source not in known:
            raise ValueError(f"{name}: source {source} must be '{RELEASES}' or an earlier channel")
        if not isinstance(c["soak_hours"], (int, float)) or c["soak_hours"] < 0:
            raise ValueError(f"{name}: soak_hours must be a number >= 0")
        known.add(name)


def empty_state(policy):
    return {"channels": {c["name"]: {"tag": None, "history": []} for c in policy["channels"]},
            "frozen": [], "yanked": []}


def check_channel(state, name):
    if name not in state["channels"]:
        raise ValueError(f"unknown channel: {name}")


def check_tag(tag):
    if not TAG.match(tag or ""):
        raise ValueError(f"invalid release tag: {tag}")


def set_tag(state, name, tag, now, by):
    channel = state["channels"][name]
    channel["tag"] = tag
    channel["history"].append({"tag": tag, "at": now, "by": by})
    del channel["history"][:-HISTORY_LIMIT]


def position(r):
    if isinstance(r.get("position"), int):
        return r["position"]
    m = MAIN_POSITION.search(r.get("tag_name") or "")
    return int(m.group(1)) if m else None


def published(releases, now):
    """Tag -> (main position, publication time), for published releases up
    to `now` whose position is known."""
    out = {}
    for r in releases:
        tag, at, pos = r.get("tag_name"), r.get("published_at"), position(r)
        if r.get("draft") or not at or not TAG.match(tag or "") or pos is None:
            continue
        if parse_time(at) > now:
            continue
        out[tag] = (pos, parse_time(at))
    return out


def promote(state, policy, releases, now):
    check_policy(policy)
    now_t = parse_time(now)
    order = published(releases, now_t)

    def key(tag):
        return (order[tag][0], tag)

    for c in policy["channels"]:
        state["channels"].setdefault(c["name"], {"tag": None, "history": []})
    changes = []
    for c in policy["channels"]:
        name, source = c["name"], c["source"]
        if name in state["frozen"]:
            continue
        cutoff = now_t - timedelta(hours=c["soak_hours"])
        if source == RELEASES:
            entries = [(tag, at) for tag, (_, at) in order.items()]
            ceiling = None
        else:
            src = state["channels"][source]
            entries = [(e["tag"], parse_time(e["at"])) for e in src["history"]]
            ceiling = src["tag"]
            if ceiling not in order:
                continue
        candidates = [tag for tag, at in entries
                      if at <= cutoff and tag in order and tag not in state["yanked"]
                      and (ceiling is None or key(tag) <= key(ceiling))]
        if not candidates:
            continue
        best = max(candidates, key=key)
        current = state["channels"][name]["tag"]
        if current == best or (current in order and key(best) <= key(current)):
            continue
        set_tag(state, name, best, now, "promote")
        changes.append(f"{name}: {current or '(none)'} -> {best}")
    return changes


def freeze(state, name):
    check_channel(state, name)
    if name not in state["frozen"]:
        state["frozen"].append(name)
    return [f"{name}: frozen"]


def unfreeze(state, name):
    check_channel(state, name)
    if name in state["frozen"]:
        state["frozen"].remove(name)
    return [f"{name}: unfrozen"]


def yank(state, tag):
    check_tag(tag)
    if tag not in state["yanked"]:
        state["yanked"].append(tag)
    return [f"{tag}: yanked"]


def unyank(state, tag):
    check_tag(tag)
    if tag in state["yanked"]:
        state["yanked"].remove(tag)
    return [f"{tag}: unyanked"]


def pin(state, name, tag, now):
    check_channel(state, name)
    check_tag(tag)
    current = state["channels"][name]["tag"]
    set_tag(state, name, tag, now, "pin")
    freeze(state, name)
    return [f"{name}: {current or '(none)'} -> {tag} (pinned, frozen)"]


def load(directory, policy):
    path = directory / "channels.json"
    if path.exists():
        state = json.loads(path.read_text())
        for c in policy["channels"]:
            state["channels"].setdefault(c["name"], {"tag": None, "history": []})
        return state
    return empty_state(policy)


def save(directory, state):
    (directory / "channels").mkdir(parents=True, exist_ok=True)
    (directory / "channels.json").write_text(json.dumps(state, indent=2, sort_keys=True) + "\n")
    for name, channel in state["channels"].items():
        pointer = directory / "channels" / name
        if channel["tag"]:
            pointer.write_text(channel["tag"] + "\n")
        elif pointer.exists():
            pointer.unlink()


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--dir", type=Path, required=True)
    p.add_argument("--policy", type=Path, required=True)
    sub = p.add_subparsers(dest="cmd", required=True)
    sp = sub.add_parser("promote")
    sp.add_argument("--releases", type=Path, required=True)
    sp.add_argument("--now", help="ISO 8601 UTC time (default: now)")
    for cmd in ("freeze", "unfreeze"):
        sub.add_parser(cmd).add_argument("channel")
    for cmd in ("yank", "unyank"):
        sub.add_parser(cmd).add_argument("tag")
    sp = sub.add_parser("pin")
    sp.add_argument("channel")
    sp.add_argument("tag")
    sp.add_argument("--now", help="ISO 8601 UTC time (default: now)")
    a = p.parse_args(argv)

    now = getattr(a, "now", None) or format_time(datetime.now(timezone.utc))
    try:
        policy = json.loads(a.policy.read_text())
        check_policy(policy)
        state = load(a.dir, policy)
        if a.cmd == "promote":
            changes = promote(state, policy, json.loads(a.releases.read_text()), now)
        elif a.cmd in ("freeze", "unfreeze"):
            changes = globals()[a.cmd](state, a.channel)
        elif a.cmd in ("yank", "unyank"):
            changes = globals()[a.cmd](state, a.tag)
        else:
            changes = pin(state, a.channel, a.tag, now)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    a.dir.mkdir(parents=True, exist_ok=True)
    save(a.dir, state)
    for change in changes:
        print(change)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
