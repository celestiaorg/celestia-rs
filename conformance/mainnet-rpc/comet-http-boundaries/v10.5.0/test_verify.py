#!/usr/bin/env python3
"""Mutation tests for the offline Comet HTTP evidence verifier."""

from __future__ import annotations

import gzip
import hashlib
import json
import pathlib
import shutil
import tempfile
import unittest

import verify


HERE = pathlib.Path(__file__).parent


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class Bundle:
    def __init__(self, root: pathlib.Path):
        self.root = root

    def json(self, name: str) -> dict:
        return json.loads((self.root / name).read_text())

    def write_json(self, name: str, value: dict) -> None:
        (self.root / name).write_text(json.dumps(value, indent=2) + "\n")
        self.rebind(name)

    def rebind(self, name: str) -> None:
        path = self.root / name
        data = path.read_bytes()
        provenance = self.json("provenance.json")
        record = provenance["files"][name]
        record["sha256"] = sha(data)
        record["size"] = len(data)
        if name.endswith(".gz"):
            raw = gzip.decompress(data)
            record["uncompressed_sha256"] = sha(raw)
            record["uncompressed_size"] = len(raw)
        (self.root / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")


class VerifierMutationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.temp.name) / "bundle"
        shutil.copytree(HERE, root, ignore=shutil.ignore_patterns("__pycache__"))
        self.bundle = Bundle(root)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def rejected(self) -> None:
        with self.assertRaises(verify.VerificationError):
            verify.verify_bundle(self.bundle.root)

    def test_original_bundle(self) -> None:
        verify.verify_bundle(self.bundle.root)

    def test_raw_request_mutation_is_rejected(self) -> None:
        capture = self.bundle.json("comet-http-boundaries.json")
        capture["cases"][0]["request"]["raw_base64"] = "AA=="
        self.bundle.write_json("comet-http-boundaries.json", capture)
        self.rejected()

    def test_self_rehashed_case_rename_is_rejected(self) -> None:
        capture = self.bundle.json("comet-http-boundaries.json")
        capture["cases"][0]["name"] = "renamed-case"
        self.bundle.write_json("comet-http-boundaries.json", capture)
        self.rejected()

    def test_self_rehashed_mainnet_protocol_rebinding_is_rejected(self) -> None:
        observation = self.bundle.json("mainnet-observation.json")
        status = json.loads(observation["responses"]["status"]["body"])
        status["result"]["node_info"]["protocol_version"]["app"] = "11"
        body = json.dumps(status, separators=(",", ":"))
        observation["responses"]["status"] = {"body": body, "sha256": sha(body.encode())}
        self.bundle.write_json("mainnet-observation.json", observation)
        self.rejected()

    def test_self_rehashed_mainnet_height_rebinding_is_rejected(self) -> None:
        observation = self.bundle.json("mainnet-observation.json")
        status = json.loads(observation["responses"]["status"]["body"])
        status["result"]["sync_info"]["latest_block_height"] = "15447467"
        body = json.dumps(status, separators=(",", ":"))
        observation["responses"]["status"] = {"body": body, "sha256": sha(body.encode())}
        self.bundle.write_json("mainnet-observation.json", observation)
        self.rejected()

    def test_self_rehashed_config_limit_mutation_is_rejected(self) -> None:
        name = "custom-config.toml.gz"
        text = gzip.decompress((self.bundle.root / name).read_bytes()).decode()
        text = text.replace("max_body_bytes = 512", "max_body_bytes = 513", 1)
        (self.bundle.root / name).write_bytes(gzip.compress(text.encode(), mtime=0))
        self.bundle.rebind(name)
        self.rejected()

    def test_self_rehashed_header_boundary_mutation_is_rejected(self) -> None:
        capture = self.bundle.json("comet-http-boundaries.json")
        capture["header_search"]["last_accepted_pad"] = 4258
        self.bundle.write_json("comet-http-boundaries.json", capture)
        self.rejected()

    def test_self_rehashed_identity_before_capture_is_rejected(self) -> None:
        identity = self.bundle.json("zero-identity.json")
        identity["observed_at"] = "2026-10-31T01:48:59+00:00"
        self.bundle.write_json("zero-identity.json", identity)
        self.rejected()

    def test_self_rehashed_body_overflow_connection_outcome_mutation_is_rejected(self) -> None:
        capture = self.bundle.json("comet-http-boundaries.json")
        case = next(
            case
            for case in capture["cases"]
            if case["name"] == "keepalive-after-body-error"
        )
        case["outcome"] = "second-response"
        self.bundle.write_json("comet-http-boundaries.json", capture)
        self.rejected()


if __name__ == "__main__":
    unittest.main()
