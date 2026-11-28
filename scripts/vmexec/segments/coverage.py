#!/usr/bin/env python3
"""Join inclusive execution ranges from trusted, single-run follow logs for one chain."""

import argparse
import re
from pathlib import Path

STOP = re.compile(r"\bINFO celestia_rs::follow: stop height reached height=(\d+) verified_header_height=(\d+)\s*$")
FINISH = re.compile(r"\bINFO celestia_rs::follow: follow finished first=(\d+) last=(\d+) blocks=(\d+)(?:\s|$)")


def verified_range(path):
    """A finite run authenticates the effects of its last block with header H+1."""
    stop = finished = None
    with path.open() as log:
        for line in log:
            if match := STOP.search(line):
                if stop is not None or finished is not None:
                    raise ValueError("expected one run per file")
                stop = tuple(map(int, match.groups()))
            if match := FINISH.search(line):
                if finished is not None:
                    raise ValueError("expected one run per file")
                finished = tuple(map(int, match.groups()))
    if finished is None or stop is None:
        raise ValueError("missing authenticated stop or final counters")
    first, last, blocks = finished
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
