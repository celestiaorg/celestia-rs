"""The composed #190 closing evidence must remain complete and fail closed."""

import copy
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[3]
MODULE = REPO / "conformance/websocket-lifecycle/verify-closure.py"
SPEC = importlib.util.spec_from_file_location("websocket_closure_verify", MODULE)
verify = importlib.util.module_from_spec(SPEC)
OLD_DONT_WRITE_BYTECODE = sys.dont_write_bytecode
try:
    sys.dont_write_bytecode = True
    SPEC.loader.exec_module(verify)
finally:
    sys.dont_write_bytecode = OLD_DONT_WRITE_BYTECODE

BASE_MODULE = REPO / "conformance/websocket-lifecycle/verify.py"
BASE_SPEC = importlib.util.spec_from_file_location("websocket_lifecycle_verify", BASE_MODULE)
base_verify = importlib.util.module_from_spec(BASE_SPEC)
try:
    sys.dont_write_bytecode = True
    BASE_SPEC.loader.exec_module(base_verify)
finally:
    sys.dont_write_bytecode = OLD_DONT_WRITE_BYTECODE


class WebSocketClosureEvidence(unittest.TestCase):
    def setUp(self):
        self.review = json.loads(verify.REVIEW.read_text())

    def lifecycle_copy(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name) / "websocket-lifecycle"
        shutil.copytree(verify.ROOT, root)
        return root

    def test_complete_closure_passes(self):
        verify.verify_review(self.review)
        verify.verify_lifecycle()
        verify.verify_soak(self.review)

    def test_acceptance_criterion_cannot_disappear(self):
        self.review["acceptance"].pop()
        with self.assertRaisesRegex(ValueError, "acceptance criterion inventory"):
            verify.verify_review(self.review)

    def test_lifecycle_artifact_cannot_be_rebound(self):
        self.review["lifecycle_capture"]["manifest_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "lifecycle evidence identity"):
            verify.verify_review(self.review)

    def test_lifecycle_paths_cannot_alias_one_artifact(self):
        lifecycle = self.review["lifecycle_capture"]
        lifecycle["manifest"] = lifecycle["cors_enabled"]
        lifecycle["manifest_sha256"] = lifecycle["cors_enabled_sha256"]
        lifecycle["cors_disabled"] = lifecycle["cors_enabled"]
        lifecycle["cors_disabled_sha256"] = lifecycle["cors_enabled_sha256"]
        with self.assertRaisesRegex(ValueError, "lifecycle evidence identity"):
            verify.verify_review(self.review)

    def test_soak_candidate_cannot_be_rebound(self):
        self.review["mixed_node_soak"]["capture_commit"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "candidate commit changed"):
            verify.verify_review(self.review)

    def test_websocket_completion_count_cannot_change(self):
        self.review["mixed_node_soak"]["websocket_results"]["comet-ws-subscribe"] = 54
        with self.assertRaisesRegex(ValueError, "completion total changed"):
            verify.verify_soak(self.review)

    def test_websocket_operation_cannot_disappear(self):
        self.review["mixed_node_soak"]["websocket_results"].pop("comet-ws-slow-reconnect")
        with self.assertRaisesRegex(ValueError, "operation inventory"):
            verify.verify_review(self.review)

    def test_acceptance_evidence_cannot_be_rewritten(self):
        for row in self.review["acceptance"]:
            row["evidence"] = ["bogus"]
        with self.assertRaisesRegex(ValueError, "acceptance criterion inventory"):
            verify.verify_review(self.review)

    def test_resource_summary_cannot_be_rewritten(self):
        self.review["mixed_node_soak"]["nodes"][4]["fds_max"] = 33
        with self.assertRaisesRegex(ValueError, "resource sample summary changed"):
            verify.verify_soak(self.review)

    def test_limitations_and_normalizations_cannot_disappear(self):
        for field, message in (
            ("limitations", "limitations changed"),
            ("normalizations", "normalizations changed"),
        ):
            with self.subTest(field=field):
                changed = copy.deepcopy(self.review)
                changed[field].pop()
                with self.assertRaisesRegex(ValueError, message):
                    verify.verify_review(changed)

    def test_symlinked_required_file_is_rejected_before_resolution(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary) / "repo"
            repo.mkdir()
            outside = Path(temporary) / "outside.json"
            outside.write_text("{}")
            (repo / "evidence.json").symlink_to(outside)
            with patch.object(verify, "REPO", repo):
                with self.assertRaisesRegex(ValueError, "symlinked evidence path"):
                    verify.repo_path("evidence.json")

    def test_symlinked_evidence_directory_component_is_rejected(self):
        root = self.lifecycle_copy()
        oracle = root / "testdata/oracle"
        outside = root.parent / "oracle-outside"
        shutil.copytree(oracle, outside)
        shutil.rmtree(oracle)
        oracle.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlinked WebSocket evidence path"):
            base_verify.verify_inventory(root)

    def test_extra_evidence_file_is_rejected(self):
        root = self.lifecycle_copy()
        (root / "unreviewed.txt").write_text("extra")
        with self.assertRaisesRegex(ValueError, "evidence inventory changed"):
            base_verify.verify_inventory(root)

    def test_omitted_manifest_entry_is_rejected(self):
        root = self.lifecycle_copy()
        manifest = root / "testdata/manifest.json"
        value = json.loads(manifest.read_text())
        value["artifacts_sha256"].pop("go.mod")
        manifest.write_text(json.dumps(value, indent=2) + "\n")
        with self.assertRaisesRegex(ValueError, "manifest inventory changed"):
            base_verify.verify_inventory(root)

    def test_deleted_evidence_file_is_rejected(self):
        root = self.lifecycle_copy()
        (root / "main.go").unlink()
        with self.assertRaisesRegex(ValueError, "evidence inventory changed"):
            base_verify.verify_inventory(root)


if __name__ == "__main__":
    unittest.main()
