"""Exercise the oracle build recipe/cache without network or a Go compiler."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


FAKE_GO = '''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
if args == ["version"]:
    print("go version " + os.environ.get("TEST_GO_VERSION", "go1.26.9") + " linux/amd64")
elif args[:2] == ["version", "-m"]:
    print("fake build info")
elif args[:2] == ["env", "-json"]:
    print(json.dumps({"GOOS": "linux", "GOARCH": "amd64", "CGO_ENABLED": "1",
                      "GOAMD64": os.environ.get("TEST_GOAMD64", "v1")}))
elif args[:3] == ["mod", "download", "-json"]:
    marker = pathlib.Path(os.environ["TEST_BUILD_LOG"] + ".download-attempt")
    if os.environ.get("TEST_DOWNLOAD_FAIL_ONCE") and not marker.exists():
        marker.touch()
        print(json.dumps({"Error": "transient module proxy failure"}))
        sys.exit(1)
    module, version = args[3].split("@")
    print(json.dumps({"Path": module, "Version": version, "Dir": os.environ["TEST_MODULE_DIR"],
                      "Sum": "h1:module", "Origin": {"Hash": "1" * 40}}))
elif args[:2] == ["mod", "download"]:
    pass
elif args[0] == "build":
    with open(os.environ["TEST_BUILD_LOG"], "a") as log:
        log.write(json.dumps(args) + "\\n")
    out = pathlib.Path(args[args.index("-o") + 1])
    flags = next((item.split("=", 1)[1] for item in args if item.startswith("-ldflags=")), "")
    version = next((item.split("=", 1)[1] for item in flags.split() if item.startswith("github.com/cosmos/cosmos-sdk/version.Version=")), "")
    if os.environ.get("TEST_BAD_VERSION"):
        version = "wrong"
    out.write_text("#!/bin/sh\\nprintf '%s\\\\n' '" + version + "'\\n")
    out.chmod(0o755)
else:
    raise RuntimeError(args)
'''


class OracleBuildCache(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        scripts = self.root / "conformance/devnet"
        scripts.mkdir(parents=True)
        (self.root / "conformance/devnet-oracle").mkdir()
        shutil.copy(Path(__file__).with_name("build-bins.sh"), scripts)
        self.script = scripts / "build-bins.sh"
        tools = self.root / "tools"
        tools.mkdir()
        go = tools / "go"
        go.write_text(FAKE_GO)
        go.chmod(0o755)
        sleep = tools / "sleep"
        sleep.write_text("#!/bin/sh\nexit 0\n")
        sleep.chmod(0o755)
        module = self.root / "module"
        module.mkdir()
        (module / "go.mod").write_text("module fake\n")
        self.binary = self.root / "bin/celestia-appd"
        self.log = self.root / "builds.jsonl"
        self.env = dict(os.environ, PATH=f"{tools}:{os.environ['PATH']}", GOTOOLCHAIN="local",
                        APPD_ONLY="1", DEVNET_BIN=str(self.binary.parent), APP_VERSION="v10.5.0",
                        TEST_MODULE_DIR=str(module), TEST_BUILD_LOG=str(self.log))

    def build(self, **changes):
        return subprocess.run(["bash", str(self.script)], env={**self.env, **changes},
                              text=True, capture_output=True, timeout=15)

    def test_legacy_module_stamp_is_rebuilt_and_release_recipe_is_recorded(self):
        self.binary.parent.mkdir()
        self.binary.write_text("#!/bin/sh\necho old\n")
        self.binary.chmod(0o755)
        self.binary.with_name("celestia-appd.module").write_text("github.com/celestiaorg/celestia-app/v10@v10.5.0\n")
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(subprocess.check_output([str(self.binary), "version"], text=True).strip(), "10.5.0")
        recipe = json.loads(self.binary.with_name("celestia-appd.build.json").read_text())
        self.assertEqual(recipe["build_tags"], ["ledger"])
        self.assertIn("version.Commit=" + "1" * 40, recipe["ldflags"])
        self.assertIn("cmd.v2UpgradeHeight=0", recipe["ldflags"])
        self.assertEqual(len(recipe["binary_sha256"]), 64)

    def test_identical_recipe_reuses_binary_but_compiler_and_content_changes_rebuild(self):
        for options, expected_builds in (({}, 1), ({}, 1), ({"TEST_GO_VERSION": "go1.26.10"}, 2),
                                        ({"TEST_GO_VERSION": "go1.26.10", "TEST_GOAMD64": "v3"}, 3)):
            result = self.build(**options)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(self.log.read_text().splitlines()), expected_builds)
        self.binary.write_text("#!/bin/sh\necho corrupt\n")
        result = self.build(TEST_GO_VERSION="go1.26.10", TEST_GOAMD64="v3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.log.read_text().splitlines()), 4)

    def test_version_mismatch_never_publishes_binary_or_success_stamp(self):
        result = self.build(TEST_BAD_VERSION="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("oracle version stamp mismatch", result.stderr)
        self.assertFalse(self.binary.exists())
        self.assertFalse(self.binary.with_name("celestia-appd.module").exists())

    def test_failed_download_json_is_replaced_on_successful_retry(self):
        result = self.build(TEST_DOWNLOAD_FAIL_ONCE="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("attempt 1", result.stderr)
        self.assertEqual(len(self.log.read_text().splitlines()), 1)


if __name__ == "__main__":
    unittest.main()
