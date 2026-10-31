#!/usr/bin/env python3
"""Positive and self-rehashed mutation tests for the offline verifier."""

import base64
import gzip
import hashlib
import json
import pathlib
import shutil
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
import verify


HERE = pathlib.Path(__file__).parent


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class VerifierTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        provenance = json.loads((HERE / "provenance.json").read_text())
        for name in ["provenance.json", *provenance["published_files"]]:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(HERE / name, destination)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def rehash(self, name: str) -> None:
        provenance_path = self.root / "provenance.json"
        provenance = json.loads(provenance_path.read_text())
        data = (self.root / name).read_bytes()
        entry = {"sha256": sha(data), "size": len(data)}
        if name.endswith(".gz"):
            raw = gzip.decompress(data)
            entry |= {"uncompressed_sha256": sha(raw), "uncompressed_size": len(raw)}
        provenance["files"][name] = entry
        provenance_path.write_text(json.dumps(provenance, indent=2) + "\n")

    def mutate_capture(self, callback) -> None:
        path = self.root / "proxy-tls-framing.json"
        capture = json.loads(path.read_text())
        callback(capture)
        path.write_text(json.dumps(capture, indent=2) + "\n")
        self.rehash("proxy-tls-framing.json")

    def test_positive(self) -> None:
        verify.verify(self.root)

    def test_unlisted_file_rejected(self) -> None:
        (self.root / "unlisted.txt").write_text("not in the manifest\n")
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_symlink_rejected(self) -> None:
        (self.root / "linked-capture.json").symlink_to("proxy-tls-framing.json")
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_inventory_rename_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][0].__setitem__("name", "renamed"))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_raw_request_rejected(self) -> None:
        def mutate(value) -> None:
            request = value["cases"][9]["request"]
            raw = bytearray(base64.b64decode(request["raw_base64"]))
            raw[0] ^= 1
            request["raw_base64"] = base64.b64encode(raw).decode()
            request["raw_sha256"] = sha(raw)
        self.mutate_capture(mutate)
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_status_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][0]["response"].__setitem__("status", 201))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_body_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][0]["response"].__setitem__("body_text", "{}"))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_stable_header_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][0]["response"]["stable_headers"][0].__setitem__(1, "text/plain"))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_h2_protocol_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][1]["response"].__setitem__("http_version", "HTTP/1.1"))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_wss_fin_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][4]["response_frame"].__setitem__("fin", False))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_wss_close_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][4]["close_response"].__setitem__("code", 1001))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_config_outcome_rejected(self) -> None:
        self.mutate_capture(lambda value: value["cases"][-1].__setitem__("expected_transport", "plain"))
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_observation_order_rejected(self) -> None:
        path = self.root / "mainnet-observation.json"
        value = json.loads(path.read_text())
        value["observed_at"] = "2126-01-01T00:00:00+00:00"
        path.write_text(json.dumps(value, indent=2) + "\n")
        self.rehash("mainnet-observation.json")
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_observation_height_rejected(self) -> None:
        path = self.root / "mainnet-observation.json"
        value = json.loads(path.read_text())
        value["height"] = "1"
        path.write_text(json.dumps(value, indent=2) + "\n")
        self.rehash("mainnet-observation.json")
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)

    def test_rehashed_tagged_source_rejected(self) -> None:
        name = "go-source/core-http-server.go.gz"
        path = self.root / name
        raw = gzip.decompress(path.read_bytes()) + b"\n// mutation\n"
        path.write_bytes(gzip.compress(raw, mtime=0))
        self.rehash(name)
        with self.assertRaises(verify.VerificationError):
            verify.verify(self.root)


if __name__ == "__main__":
    unittest.main()
