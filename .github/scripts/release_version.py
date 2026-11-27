#!/usr/bin/env python3
"""Conventional-commit PR titles, the daily release's version, and its notes.

PRs are squash-merged, so a PR's title is its commit's subject. Titles
must be conventional commits: `<type>[(scope)][!]: <description>`, with
type one of TYPES. A `!` or a `BREAKING CHANGE:` footer marks a breaking
change.

The next version, from the last release and the commits since it:

- before 1.0: a breaking change bumps the minor version; anything else,
  the patch. Automatic releases never reach 1.0.0; that is a human
  release (`prepare release`).
- from 1.0: breaking: major; `feat`: minor; anything else: patch.

With no release yet on the line, or a crate version (in Cargo.toml) ahead
of the last release, the crate version's <major>.<minor>.<patch> wins.

Usage:

  release_version.py check-title TITLE
  release_version.py next --crate-version V --last VERSION|'' --log FILE
  release_version.py notes --log FILE

FILE is `git log --format=%s%x1f%b%x1e <last tag>..<commit>`.
"""

import argparse
from dataclasses import dataclass
import re
import sys

TYPES = ("feat", "fix", "perf", "refactor", "docs", "test", "ci", "build", "chore", "style", "revert")
TITLE = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^()\s][^()]*)\))?(?P<bang>!)?: (?P<desc>\S.*)$")
BREAKING_FOOTER = re.compile(r"^BREAKING[ -]CHANGE: ", re.MULTILINE)


@dataclass
class Commit:
    subject: str
    body: str = ""

    def parsed(self):
        m = TITLE.match(self.subject)
        return m if m and m["type"] in TYPES else None

    def breaking(self):
        m = self.parsed()
        return bool(m and m["bang"]) or bool(BREAKING_FOOTER.search(self.body))

    def type(self):
        m = self.parsed()
        return m["type"] if m else None


def check_title(title):
    """None if TITLE is a conventional commit, else why not."""
    m = TITLE.match(title)
    if not m:
        return ("not a conventional commit: want '<type>[(scope)][!]: <description>', "
                "e.g. 'fix(rpc): handle empty pages'")
    if m["type"] not in TYPES:
        return f"unknown type '{m['type']}': use one of {', '.join(TYPES)}"
    return None


def parse_version(text):
    m = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", text)
    if not m:
        raise ValueError(f"not <major>.<minor>.<patch>: {text}")
    return tuple(int(x) for x in m.groups())


def next_version(crate_version, last, commits):
    floor = parse_version(crate_version.split("-")[0].split("+")[0])
    if not last:
        return "%d.%d.%d" % floor
    major, minor, patch = parse_version(last)
    breaking = any(c.breaking() for c in commits)
    if major == 0:
        bumped = (0, minor + 1, 0) if breaking else (0, minor, patch + 1)
    elif breaking:
        bumped = (major + 1, 0, 0)
    elif any(c.type() == "feat" for c in commits):
        bumped = (major, minor + 1, 0)
    else:
        bumped = (major, minor, patch + 1)
    if floor > (major, minor, patch):
        bumped = floor
    return "%d.%d.%d" % bumped


def release_notes(commits):
    if not commits:
        return "No changes.\n"
    groups = {"Breaking changes": [], "Features": [], "Fixes": [], "Other changes": []}
    for c in commits:
        m = c.parsed()
        if c.breaking():
            group = "Breaking changes"
        elif m and m["type"] == "feat":
            group = "Features"
        elif m and m["type"] == "fix":
            group = "Fixes"
        else:
            groups["Other changes"].append(c.subject)
            continue
        line = m["desc"] if m else c.subject
        if m and m["scope"]:
            line = f"**{m['scope']}:** {line}"
        groups[group].append(line)
    out = []
    for title, lines in groups.items():
        if lines:
            out += [f"### {title}", ""] + [f"- {line}" for line in lines] + [""]
    return "\n".join(out)


def read_log(path):
    commits = []
    for record in open(path, encoding="utf-8").read().split("\x1e"):
        record = record.strip("\n")
        if not record.strip():
            continue
        subject, _, body = record.partition("\x1f")
        commits.append(Commit(subject.strip(), body.strip()))
    return commits


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check-title").add_argument("title")
    sp = sub.add_parser("next")
    sp.add_argument("--crate-version", required=True)
    sp.add_argument("--last", required=True)
    sp.add_argument("--log", required=True)
    sub.add_parser("notes").add_argument("--log", required=True)
    a = p.parse_args(argv)
    if a.cmd == "check-title":
        error = check_title(a.title)
        if error:
            print(f"PR title {a.title!r}: {error}", file=sys.stderr)
            return 1
        return 0
    if a.cmd == "next":
        print(next_version(a.crate_version, a.last, read_log(a.log)))
        return 0
    sys.stdout.write(release_notes(read_log(a.log)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
