#!/usr/bin/env python3
"""Tests for release_channels.py: channel promotion, freeze, yank and pin."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import release_channels as rc  # noqa: E402

POLICY = {"channels": [
    {"name": "quarto", "source": "releases", "soak_hours": 0},
    {"name": "mocha", "source": "quarto", "soak_hours": 24},
    {"name": "mainnet", "source": "mocha", "soak_hours": 72},
]}


def release(tag, published, draft=False):
    return {"tag_name": tag, "published_at": published, "draft": draft}


def t(day, hour=0):
    return f"2026-12-{day:02d}T{hour:02d}:00:00Z"


class PromoteTests(unittest.TestCase):
    def setUp(self):
        self.state = rc.empty_state(POLICY)
        self.releases = [
            release("v0.1.0-main.1", t(1)),
            release("v0.1.0-main.2", t(2)),
        ]

    def promote(self, now):
        return rc.promote(self.state, POLICY, self.releases, now)

    def current(self, name):
        return self.state["channels"][name]["tag"]

    def test_first_channel_takes_newest_published_release(self):
        changes = self.promote(t(2, 1))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.2")
        self.assertIsNone(self.current("mocha"))
        self.assertEqual(changes, ["quarto: (none) -> v0.1.0-main.2"])

    def test_drafts_unpublished_and_invalid_tags_are_ignored(self):
        self.releases += [
            release("v0.1.0-main.3", None),
            release("v0.1.0-main.4", t(2, 1), draft=True),
            release("../main", t(2, 2)),
        ]
        self.promote(t(3))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.2")

    def test_downstream_channel_waits_for_soak_on_its_source(self):
        self.promote(t(2, 1))  # quarto gets main.2 at day 2 01:00
        self.promote(t(3, 0))  # 23h later
        self.assertIsNone(self.current("mocha"))
        self.promote(t(3, 1))  # 24h later
        self.assertEqual(self.current("mocha"), "v0.1.0-main.2")
        self.assertIsNone(self.current("mainnet"))
        self.promote(t(6, 1))  # 72h on mocha
        self.assertEqual(self.current("mainnet"), "v0.1.0-main.2")

    def test_frequent_releases_do_not_starve_downstream_channels(self):
        # A release every hour: quarto never keeps one for 24h, but each
        # one entered quarto and every later release contains it.
        for hour in range(30):
            self.releases.append(release(f"v0.1.0-main.{10 + hour}", f"2026-12-03T{hour % 24:02d}:00:00Z"
                                         if hour < 24 else f"2026-12-04T{hour - 24:02d}:00:00Z"))
            self.promote(self.releases[-1]["published_at"])
        # At day 4 05:00, main.15 has been in quarto (inside every later
        # release) for 24h.
        self.assertEqual(self.current("mocha"), "v0.1.0-main.15")
        self.assertEqual(self.current("quarto"), "v0.1.0-main.39")

    def test_never_promotes_past_what_the_source_runs_now(self):
        self.promote(t(2, 1))
        self.promote(t(3, 1))
        self.assertEqual(self.current("mocha"), "v0.1.0-main.2")
        # Roll mocha back to main.1; mainnet must not get main.2.
        rc.pin(self.state, "mocha", "v0.1.0-main.1", t(3, 2))
        self.promote(t(6, 1))  # main.2 has 72h on mocha, but mocha left it
        self.assertEqual(self.current("mocha"), "v0.1.0-main.1")
        self.assertIsNone(self.current("mainnet"))
        self.promote(t(6, 2))  # main.1 has 72h on mocha since the pin
        self.assertEqual(self.current("mainnet"), "v0.1.0-main.1")

    def test_no_automatic_downgrade(self):
        self.state["channels"]["mainnet"] = {"tag": "v0.1.0-main.2", "history": []}
        self.state["channels"]["mocha"] = {"tag": "v0.1.0-main.1",
                                           "history": [{"tag": "v0.1.0-main.1", "at": t(1)}]}
        self.state["channels"]["quarto"] = {"tag": "v0.1.0-main.1",
                                            "history": [{"tag": "v0.1.0-main.1", "at": t(1)}]}
        rc.freeze(self.state, "quarto")
        self.promote(t(20))
        self.assertEqual(self.current("mainnet"), "v0.1.0-main.2")

    def test_a_late_published_older_commit_does_not_move_a_channel_back(self):
        self.releases.append(release("v0.1.0-main.4", t(2, 1)))
        self.promote(t(2, 1))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.4")
        # main.3's build was slower: published after main.4.
        self.releases.append(release("v0.1.0-main.3", t(2, 2)))
        self.assertEqual(self.promote(t(2, 3)), [])
        self.assertEqual(self.current("quarto"), "v0.1.0-main.4")

    def test_human_releases_order_by_their_position_field(self):
        self.releases.append(dict(release("v0.1.0", t(2, 2)), position=3))
        self.releases.append(release("v0.1.0-main.4", t(2, 1)))
        self.promote(t(2, 3))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.4")
        # A release with no main position cannot be ordered: never taken.
        self.releases = [release("v9.0.0", t(1))]
        self.state = rc.empty_state(POLICY)
        self.assertEqual(self.promote(t(2)), [])

    def test_frozen_channel_does_not_move(self):
        rc.freeze(self.state, "quarto")
        self.assertEqual(self.promote(t(3)), [])
        self.assertIsNone(self.current("quarto"))
        rc.unfreeze(self.state, "quarto")
        self.promote(t(3))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.2")

    def test_yanked_release_is_skipped_in_favor_of_an_older_soaked_one(self):
        self.promote(t(1, 1))  # quarto: main.1
        self.promote(t(2, 1))  # quarto: main.2
        rc.yank(self.state, "v0.1.0-main.2")
        self.promote(t(3, 1))
        self.assertEqual(self.current("mocha"), "v0.1.0-main.1")
        self.releases.append(release("v0.1.0-main.3", t(3, 2)))
        self.promote(t(3, 2))
        self.assertEqual(self.current("quarto"), "v0.1.0-main.3")

    def test_pin_sets_and_freezes(self):
        rc.pin(self.state, "mainnet", "v0.1.0-main.1", t(5))
        self.assertEqual(self.current("mainnet"), "v0.1.0-main.1")
        self.assertIn("mainnet", self.state["frozen"])
        self.assertEqual(self.state["channels"]["mainnet"]["history"][-1],
                         {"tag": "v0.1.0-main.1", "at": t(5), "by": "pin"})

    def test_unknown_channel_and_bad_tag_are_rejected(self):
        with self.assertRaises(ValueError):
            rc.freeze(self.state, "nope")
        with self.assertRaises(ValueError):
            rc.pin(self.state, "mocha", "main", t(1))

    def test_policy_must_be_a_chain_of_known_sources(self):
        with self.assertRaises(ValueError):
            rc.check_policy({"channels": [{"name": "mocha", "source": "quarto", "soak_hours": 1}]})
        with self.assertRaises(ValueError):
            rc.check_policy({"channels": [{"name": "releases", "source": "releases", "soak_hours": 1}]})
        rc.check_policy(POLICY)


class CliTests(unittest.TestCase):
    def test_promote_writes_state_and_pointer_files(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "policy.json").write_text(json.dumps(POLICY))
            (d / "releases.json").write_text(json.dumps([release("v0.1.0-main.1", t(1))]))
            out = d / "out"
            args = [sys.executable, str(Path(__file__).with_name("release_channels.py")),
                    "--dir", str(out), "--policy", str(d / "policy.json")]
            r = subprocess.run(args + ["promote", "--releases", str(d / "releases.json"),
                                       "--now", t(2)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(r.stdout.strip(), "quarto: (none) -> v0.1.0-main.1")
            self.assertEqual((out / "channels" / "quarto").read_text(), "v0.1.0-main.1\n")
            self.assertFalse((out / "channels" / "mocha").exists())
            state = json.loads((out / "channels.json").read_text())
            self.assertEqual(state["channels"]["quarto"]["tag"], "v0.1.0-main.1")

            r = subprocess.run(args + ["yank", "v0.1.0-main.1"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(json.loads((out / "channels.json").read_text())["yanked"],
                             ["v0.1.0-main.1"])
            r = subprocess.run(args + ["freeze", "nope"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 1)


if __name__ == "__main__":
    unittest.main()
