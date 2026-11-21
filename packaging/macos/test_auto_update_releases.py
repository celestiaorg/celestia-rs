#!/usr/bin/env python3
"""Exercise release selection, rollout and rollback without live services."""

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
    if '/releases?' in url:
        if (r/'api-error').exists(): sys.exit(22)
        filename = 'releases-2.json' if '&page=2' in url else 'releases.json'
        print((r/filename).read_text())
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
    if 'rev-parse' in args: print('b'*40)
    elif 'fetch' not in args and 'checkout' not in args: sys.exit('unexpected git operation')
elif tool == 'cargo':
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
        self.releases([
            self.release("v0.1.0-rc.7", None, draft=True),
            self.release("v0.1.0", "2026-11-01T00:00:00Z"),
            self.release("v0.1.0-rc.6", "2026-11-20T00:00:00Z", prerelease=True),
        ])
        self.checks("completed", "success")

    @staticmethod
    def release(tag, published, draft=False, prerelease=False):
        return dict(tag_name=tag, published_at=published, draft=draft, prerelease=prerelease)

    def releases(self, values):
        (self.root/"releases.json").write_text(json.dumps(values))

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

    def test_newest_published_release_includes_candidates_and_excludes_drafts(self):
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        fetch = [c for c in self.calls if c[0] == "git" and "fetch" in c]
        self.assertEqual(fetch[0][-1], "refs/tags/v0.1.0-rc.6")
        resolve = [c for c in self.calls if c[0] == "git" and "rev-parse" in c]
        self.assertEqual(resolve[0][-1], "FETCH_HEAD^{commit}")
        restarts = [c[-1] for c in self.calls if c[0] == "sudo"]
        self.assertEqual(restarts, ["system/org.celestia.celestia-rs", "system/org.celestia.celestia-rs-mainnet"])
        for name in ("celestia-rs", "celestia-rs-mainnet"):
            self.assertIn(NEW, (self.root/"bin"/name).read_text())
            self.assertIn(OLD, (self.root/"bin"/(name+".prev")).read_text())

    def test_empty_draft_only_invalid_and_failed_release_queries_do_not_deploy(self):
        for releases in ([], [self.release("v0.2.0", None, draft=True)],
                         [self.release("../main", "2026-11-20T00:00:00Z")]):
            with self.subTest(releases=releases):
                self.releases(releases)
                result = self.run_update()
                self.assertEqual(result.returncode, 2, result.stdout+result.stderr)
                self.no_deployment()
                self.assertFalse(any(c[0] == "git" for c in self.calls))
        (self.root/"api-error").touch()
        self.assertEqual(self.run_update().returncode, 2)
        self.no_deployment()

    def test_publication_order_across_pages_does_not_use_main_or_tag_creation_order(self):
        self.releases([self.release('v0.1.0', '2026-11-01T00:00:00Z')]*100)
        (self.root/'releases-2.json').write_text(json.dumps([
            self.release('v0.1.0-rc.6', '2026-11-20T00:00:00Z', prerelease=True)]))
        result = self.run_update(DRY_RUN='1')
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn('DRY_RUN: built v0.1.0-rc.6', result.stdout)
        self.assertTrue(any('&page=2' in c[-1] for c in self.calls if c[0] == 'curl'))
        self.no_deployment()

    def test_already_installed_release_does_not_build_or_restart(self):
        for name in ('celestia-rs', 'celestia-rs-mainnet'):
            binary(self.root/'bin'/name, NEW, '0.1.0-rc.6')
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertIn('up to date: every node runs v0.1.0-rc.6', result.stdout)
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
