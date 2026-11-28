#!/usr/bin/env python3
"""Join inclusive execution ranges from trusted, single-run follow logs for one chain."""

import argparse
import re
from pathlib import Path

STOP = re.compile(r"\bINFO celestia_rs::follow: stop height reached height=(\d+) verified_header_height=(\d+)\s*$")
FINISH = re.compile(r"\bINFO celestia_rs::follow: follow finished first=(\d+) last=(\d+) blocks=(\d+)(?:\s|$)")

RESUME = re.compile(r"\bINFO celestia_rs::follow: resuming height=(\d+)\s")
FOLLOWED = re.compile(r"\bINFO celestia_rs::follow: followed height=(\d+)\s")


def verified_range(path):
    """A finite run authenticates the effects of its last block with header H+1."""
    stop = finished = resume = followed = None
    with path.open() as log:
        for line in log:
            if match := RESUME.search(line):
                if any(value is not None for value in (resume, followed, stop, finished)):
                    raise ValueError("expected one run per file")
                resume = int(match[1])
            if match := FOLLOWED.search(line):
                height = int(match[1])
                if finished is not None or stop is not None or (followed is not None and followed >= height):
                    raise ValueError("out-of-order execution progress")
                followed = height
            if match := STOP.search(line):
                if stop is not None or finished is not None:
                    raise ValueError("expected one run per file")
                stop = tuple(map(int, match.groups()))
            if match := FINISH.search(line):
                if finished is not None:
                    raise ValueError("expected one run per file")
                finished = tuple(map(int, match.groups()))
    if finished is None and stop is None and resume is not None and followed is not None:
        if resume + 1 < followed:
            return resume + 1, followed - 1
        raise ValueError("progress does not authenticate an executed block")
    if finished is None or stop is None:
        raise ValueError("missing authenticated stop or final counters")
    first, last, blocks = finished
    if resume is not None and resume + 1 != first:
        raise ValueError("resume and final counters disagree")
    if stop != (last, last + 1) or not 1 <= first <= last or blocks != last - first + 1:
        raise ValueError("inconsistent stop height or execution counters")
    return first, last


def missing(ranges, first, last):
    """Return gaps within the requested inclusive range."""
    cursor = first
    for start, end in sorted(ranges):
        if start > last:
            break
        if start > cursor:
            yield cursor, start - 1
        cursor = max(cursor, end + 1)
    if cursor <= last:
        yield cursor, last


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--first", type=int, default=1)
    parser.add_argument("--last", type=int, required=True)
    parser.add_argument("logs", type=Path, nargs="+")
    args = parser.parse_args()
    if not 1 <= args.first <= args.last:
        parser.error("expected 1 <= first <= last")

    ranges = []
    invalid = False
    for path in args.logs:
        try:
            start, end = verified_range(path)
        except (OSError, ValueError) as err:
            print(f"unverified {path}: {err}")
            invalid = True
            continue
        print(f"verified {start}..{end} {path}")
        ranges.append((start, end))
    gaps = list(missing(ranges, args.first, args.last))
    for start, end in gaps:
        print(f"missing {start}..{end}")
    return 2 if invalid else int(bool(gaps))


if __name__ == "__main__":
    raise SystemExit(main())
