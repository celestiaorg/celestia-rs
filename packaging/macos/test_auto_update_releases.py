#!/usr/bin/env python3
"""Exercise channel-driven release selection, rollout and rollback without live services."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("celestia-rs-auto-update.sh")
OLD = "a" * 40
NEW = "b" * 40


def binary(path, commit, version):
    path.write_text(f'''#!/usr/bin/env bash
if [[ ${{2:-}} == --long ]]; then
  printf 'commit: {commit}\\n'
else
  printf '{version}\\n'
fi
''')
    path.chmod(0o755)


STUB = '''#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path
r = Path(os.environ['TEST_ROOT'])
tool = Path(sys.argv[0]).name
args = sys.argv[1:]
with (r/'calls').open('a') as f:
    f.write(json.dumps([tool, *args])+'\\n')
if tool == 'curl':
    url = args[-1]
    if '/channels/' in url:
        f = r/'channels'/url.rsplit('/', 1)[1]
        if not f.exists(): sys.exit(22)
        print(f.read_text())
    elif '/check-runs?' in url:
        print((r/'checks.json').read_text())
    elif url.endswith('/status'):
        state = r/('mocha-height' if ':26657' in url else 'mainnet-height')
        height = int(state.read_text()) if state.exists() else 100
        state.write_text(str(height+1))
        print(json.dumps({'result': {'sync_info': {'latest_block_height': str(height),
            'catching_up': False}, 'validator_info': {'address': 'VAL', 'voting_power': '0'}}}))
    else: sys.exit('unexpected URL: '+url)
elif tool == 'git':
    # Tag v0.1.0-rc.6 is commit b..., any other tag commit a...
    if 'fetch' in args: (r/'fetched').write_text(args[-1])
    elif 'rev-parse' in args:
        print('b'*40 if 'v0.1.0-rc.6' in (r/'fetched').read_text() else 'a'*40)
    elif 'checkout' not in args: sys.exit('unexpected git operation')
elif tool == 'cargo':
    (r/'cargo-env').write_text(os.environ.get('CELESTIA_RS_RELEASE_VERSION', '<unset>'))
    shutil.copyfile(r/'built', r/'src/target/release/celestia-rs')
    (r/'src/target/release/celestia-rs').chmod(0o755)
elif tool == 'check-apphash':
    if (r/'fail-canary').exists() and ':26657' in args[0]: sys.exit(1)
elif tool != 'sudo': sys.exit('unexpected tool: '+tool)
'''


class ReleaseUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ("tools", "bin", "src/.git", "src/target/release"):
            (self.root/name).mkdir(parents=True, exist_ok=True)
        for tool in ("curl", "git", "cargo", "sudo", "check-apphash"):
            path = self.root/"tools"/tool
            path.write_text(STUB)
            path.chmod(0o755)
        for name in ("celestia-rs", "celestia-rs-mainnet"):
            binary(self.root/"bin"/name, OLD, "0.1.0-rc.5")
        binary(self.root/"built", NEW, "0.1.0-rc.6")
        self.channel("mocha", "v0.1.0-rc.6")
        self.channel("mainnet", "v0.1.0-rc.6")
        self.checks("completed", "success")

    def channel(self, name, tag):
        (self.root/"channels").mkdir(exist_ok=True)
        path = self.root/"channels"/name
        if tag is None:
            path.unlink(missing_ok=True)
        else:
            path.write_text(tag+"\n")

    def checks(self, status, conclusion):
        (self.root/"checks.json").write_text(json.dumps({"check_runs": [
            dict(name="test", status=status, conclusion=conclusion)]}))

    def run_update(self, **extra):
        env = dict(os.environ, TEST_ROOT=str(self.root), CELESTIA_RS_DIR=str(self.root),
                   PATH=str(self.root/"tools")+os.pathsep+os.environ["PATH"],
                   CHECK_APPHASH=str(self.root/"tools/check-apphash"), SOAK_SECS="0",
                   MIN_BLOCKS="0", HEALTH_TIMEOUT="10", HEALTH_POLL="0", **extra)
        result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True, capture_output=True)
        self.calls = [json.loads(line) for line in (self.root/"calls").read_text().splitlines()]
        return result

    def no_deployment(self):
        self.assertFalse(any(call[0] == "sudo" for call in self.calls))
        for name in ("celestia-rs", "celestia-rs-mainnet"):
            self.assertIn(OLD, (self.root/"bin"/name).read_text())

    def test_each_node_gets_its_channels_release(self):
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        urls = [c[-1] for c in self.calls if c[0] == "curl" and "/channels/" in c[-1]]
        self.assertEqual(urls, [
            "https://raw.githubusercontent.com/celestiaorg/celestia-rs/release-channels/channels/mocha",
            "https://raw.githubusercontent.com/celestiaorg/celestia-rs/release-channels/channels/mainnet"])
        fetch = [c for c in self.calls if c[0] == "git" and "fetch" in c]
        self.assertEqual([c[-1] for c in fetch], ["refs/tags/v0.1.0-rc.6"])
        resolve = [c for c in self.calls if c[0] == "git" and "rev-parse" in c]
        self.assertEqual(resolve[0][-1], "FETCH_HEAD^{commit}")
        restarts = [c[-1] for c in self.calls if c[0] == "sudo"]
        self.assertEqual(restarts, ["system/org.celestia.celestia-rs", "system/org.celestia.celestia-rs-mainnet"])
        for name in ("celestia-rs", "celestia-rs-mainnet"):
            self.assertIn(NEW, (self.root/"bin"/name).read_text())
            self.assertIn(OLD, (self.root/"bin"/(name+".prev")).read_text())

    def test_mainnet_stays_on_its_channel_while_mocha_moves_ahead(self):
        self.channel("mainnet", "v0.1.0-rc.5")  # what mainnet runs (commit a...)
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        restarts = [c[-1] for c in self.calls if c[0] == "sudo"]
        self.assertEqual(restarts, ["system/org.celestia.celestia-rs"])
        self.assertIn(NEW, (self.root/"bin/celestia-rs").read_text())
        self.assertIn(OLD, (self.root/"bin/celestia-rs-mainnet").read_text())
        self.assertIn("up to date: v0.1.0-rc.5", result.stdout)

    def test_automatic_release_is_built_with_its_release_version(self):
        for name in ("mocha", "mainnet"):
            self.channel(name, "v0.1.0-rc.6.main.42")
        binary(self.root/"built", NEW, "0.1.0-rc.6.main.42")
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertEqual((self.root/"cargo-env").read_text(), "0.1.0-rc.6.main.42")
        self.assertIn(NEW, (self.root/"bin/celestia-rs-mainnet").read_text())

    def test_human_release_is_built_without_a_release_version(self):
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        # build.rs treats an empty value as unset.
        self.assertIn((self.root/"cargo-env").read_text(), ("", "<unset>"))

    def test_channel_override_variable(self):
        self.channel("stable", "v0.1.0-rc.5")
        result = self.run_update(CHANNEL_mainnet="stable")
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn(OLD, (self.root/"bin/celestia-rs-mainnet").read_text())
        self.assertTrue(any(c[-1].endswith("/channels/stable") for c in self.calls if c[0] == "curl"))

    def test_missing_or_invalid_channel_deploys_nothing(self):
        for tag in (None, "../main", ""):
            with self.subTest(tag=tag):
                self.channel("mainnet", tag)
                result = self.run_update()
                self.assertEqual(result.returncode, 2, result.stdout+result.stderr)
                self.no_deployment()
                self.assertFalse(any(c[0] == "git" for c in self.calls))

    def test_already_installed_release_does_not_build_or_restart(self):
        for name in ('celestia-rs', 'celestia-rs-mainnet'):
            binary(self.root/'bin'/name, NEW, '0.1.0-rc.6')
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn('up to date: v0.1.0-rc.6 (bbbbbbb) on its nodes', result.stdout)
        self.assertFalse(any(c[0] in ('cargo', 'sudo') for c in self.calls))

    def test_failed_release_is_not_retried(self):
        state = self.root/'auto-update'
        state.mkdir()
        (state/'bad-commits').write_text(NEW+'\n')
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn('rolled back before', result.stdout)
        self.no_deployment()
        self.assertFalse(any(c[0] == 'cargo' for c in self.calls))

    def test_pending_and_failed_ci_do_not_build_or_restart(self):
        for status, conclusion in (("in_progress", None), ("completed", "failure")):
            with self.subTest(status=status):
                self.checks(status, conclusion)
                result = self.run_update()
                self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
                self.no_deployment()
                self.assertFalse(any(c[0] == "cargo" for c in self.calls))

    def test_wrong_built_version_or_commit_never_replaces_a_node(self):
        for commit, version in ((NEW, "0.1.0-rc.5"), (OLD, "0.1.0-rc.6")):
            with self.subTest(commit=commit, version=version):
                binary(self.root/"built", commit, version)
                result = self.run_update()
                self.assertEqual(result.returncode, 2, result.stdout+result.stderr)
                self.no_deployment()

    def test_dry_run_builds_without_replacing_nodes(self):
        result = self.run_update(DRY_RUN="1")
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn("DRY_RUN: built v0.1.0-rc.6", result.stdout)
        self.no_deployment()

    def test_canary_health_failure_rolls_back_and_leaves_mainnet_untouched(self):
        (self.root/"fail-canary").touch()
        result = self.run_update()
        self.assertEqual(result.returncode, 1, result.stdout+result.stderr)
        restarts = [c[-1] for c in self.calls if c[0] == "sudo"]
        self.assertEqual(restarts, ["system/org.celestia.celestia-rs"]*2)
        self.assertEqual((self.root/"auto-update/bad-commits").read_text().strip(), NEW)
        self.no_nodes_replaced()

    def no_nodes_replaced(self):
        for name in ("celestia-rs", "celestia-rs-mainnet"):
            self.assertIn(OLD, (self.root/"bin"/name).read_text())


if __name__ == "__main__":
    unittest.main()
