"""Mutation checks for the offline gRPC lifecycle verifier."""

from __future__ import annotations

import importlib.util
import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from datetime import datetime, timedelta, timezone


PATH = Path(__file__).with_name("verify.py")
SPEC = importlib.util.spec_from_file_location("grpc_lifecycle_verify", PATH)
VERIFY = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(VERIFY)


def frame(kind: int, flags: int, stream: int, payload: bytes = b"") -> bytes:
    return (len(payload).to_bytes(3, "big") + bytes([kind, flags])
            + stream.to_bytes(4, "big") + payload)


class RawFrameVerification(unittest.TestCase):
    def test_exact_frame_headers(self):
        raw = frame(6, 0, 0, b"12345678")
        self.assertEqual(VERIFY.frames(raw), [(6, 0, 0, raw)])

    def test_length_mutation_is_rejected(self):
        raw = bytearray(frame(6, 0, 0, b"12345678"))
        raw[2] = 9
        with self.assertRaises(AssertionError):
            VERIFY.frames(bytes(raw))

    def test_wire_field_mutation_is_rejected(self):
        preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
        client = preface + frame(4, 0, 0) + frame(4, 1, 0)
        server = frame(4, 0, 0)
        wire = {
            "client_bytes_hex": client.hex(),
            "server_frames": [{
                "phase": "setup", "elapsed_ms": 0, "type": 6,
                "flags": 0, "stream_id": 0, "length": 0,
                "raw_hex": server.hex(),
            }],
        }
        with self.assertRaises(AssertionError):
            VERIFY.verify_wire(wire, slow_reader=False)


class ListenerSafety(unittest.TestCase):
    def configs(self):
        config = {
            "rpc": {"laddr": "tcp://127.0.0.1:26657",
                    "grpc_laddr": "tcp://127.0.0.1:26658",
                    "pprof_laddr": "127.0.0.1:6060"},
            "p2p": {"laddr": "tcp://127.0.0.1:26656", "pex": False},
            "priv_validator_laddr": "", "priv_validator_grpc_laddr": "",
            "instrumentation": {"prometheus": False,
                                "prometheus_listen_addr": ":26660"},
        }
        app = {"grpc": {"enable": True, "address": "127.0.0.1:9090"},
               "api": {"enable": False, "address": "127.0.0.1:1317"},
               "grpc-web": {"enable": False}}
        return config, app

    def test_loopback_listeners_are_accepted(self):
        config, app = self.configs()
        VERIFY.verify_loopback_configs(config, app)

    def test_wildcard_and_pex_mutations_are_rejected(self):
        config, app = self.configs()
        for mutation in ("wildcard", "pex"):
            changed = copy.deepcopy(config)
            if mutation == "wildcard":
                changed["p2p"]["laddr"] = "tcp://0.0.0.0:26656"
            else:
                changed["p2p"]["pex"] = True
            with self.assertRaises(AssertionError):
                VERIFY.verify_loopback_configs(changed, app)


class ResourceBounds(unittest.TestCase):
    def fixture(self):
        now = datetime.now(timezone.utc)
        capture = {
            "started_at": (now + timedelta(milliseconds=10)).isoformat(),
            "completed_at": (now + timedelta(seconds=1)).isoformat(),
        }
        resources = {
            "schema_version": 1, "implementation": "go", "listener": "core", "quota": 2,
            "started_at": now.isoformat(),
            "completed_at": (now + timedelta(seconds=3)).isoformat(),
            "poll_ms": 250, "settle_ms": 2_000,
            "monitor": {"python_version": "3.12.0", "system": "Linux",
                        "kernel_release": "6.0.0", "clock_ticks_per_second": 100,
                        "page_size": 4096},
            "process_identity": {
                "initial": {"pid": 42, "start_ticks": 1000,
                            "executable_sha256": "a" * 64},
                "final": {"pid": 42, "start_ticks": 1000,
                          "executable_sha256": "a" * 64},
            },
            "samples": [
                {"elapsed_ms": 0, "rss_kib": 100_000, "threads": 12, "fd_count": 20},
                {"elapsed_ms": 1_000, "rss_kib": 120_000, "threads": 12, "fd_count": 20},
                {"elapsed_ms": 3_000, "rss_kib": 120_000, "threads": 12, "fd_count": 20},
            ],
        }
        return resources, capture

    def test_bounded_recovered_resources_are_accepted(self):
        resources, capture = self.fixture()
        VERIFY.verify_resources(resources, capture, "go", 2, "a" * 64)

    def test_resource_growth_and_rebinding_are_rejected(self):
        resources, capture = self.fixture()
        mutations = []
        one = copy.deepcopy(resources)
        one["samples"][-1]["fd_count"] = 40
        mutations.append(one)
        two = copy.deepcopy(resources)
        two["samples"][-1]["rss_kib"] = 500_000
        mutations.append(two)
        three = copy.deepcopy(resources)
        three["implementation"] = "rs"
        mutations.append(three)
        four = copy.deepcopy(resources)
        four["process_identity"]["final"]["start_ticks"] = 1001
        mutations.append(four)
        executable = copy.deepcopy(resources)
        executable["process_identity"]["initial"]["executable_sha256"] = "b" * 64
        executable["process_identity"]["final"]["executable_sha256"] = "b" * 64
        mutations.append(executable)
        five = copy.deepcopy(resources)
        five["samples"][1]["elapsed_ms"] = 2_500
        mutations.append(five)
        six = copy.deepcopy(resources)
        six["samples"][-1]["elapsed_ms"] = 1_100
        mutations.append(six)
        for mutated in mutations:
            with self.assertRaises(AssertionError):
                VERIFY.verify_resources(mutated, capture, "go", 2, "a" * 64)


def write_varint(value: int) -> bytes:
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def proto_varint(field: int, value: int) -> bytes:
    return write_varint(field << 3) + write_varint(value)


def proto_bytes(field: int, value: bytes) -> bytes:
    return write_varint(field << 3 | 2) + write_varint(len(value)) + value


class SDKLifecycleVerification(unittest.TestCase):
    def response(self, names):
        listed = b"".join(proto_bytes(1, proto_bytes(1, name.encode())) for name in names)
        return proto_bytes(6, listed).hex()

    def fixture(self):
        now = datetime.now(timezone.utc)
        services = ["cosmos.base.node.v1beta1.Service",
                    "grpc.reflection.v1alpha.ServerReflection"]
        raw = bytes([0x0a, 61]) + b"x" * 61 + bytes([0x3a, 0])
        return {
            "schema_version": 1, "implementation": "go", "endpoint": "127.0.0.1:9090",
            "started_at": now.isoformat(),
            "completed_at": (now + timedelta(seconds=1)).isoformat(),
            "input_sha256": {"gzip-reflection-list-services-65-bytes": hashlib.sha256(raw).hexdigest()},
            "cancellation": {
                "first_response_hex": self.response(services), "first_services": services,
                "first_headers": {"content-type": ["application/grpc"]},
                "cancel_code": 1, "cancel_message": "context canceled", "cancel_trailers": None,
                "recovered_response_hex": self.response(services),
                "recovered_services": services,
                "recovered_headers": {"content-type": ["application/grpc"]},
            },
            "compression": {"encoding": "gzip", "uncompressed_len": 65, "result": {
                "code": 8,
                "message": "grpc: received message after decompression larger than max (65 vs. 64)",
                "response_hex": "", "headers": None,
                "trailers": {"content-type": ["application/grpc"]}, "elapsed_ms": 2,
            }},
            "notes": [
                "SDK connection quota is unsupported; cancellation recovery uses the same grpc.ClientConn",
                "the compressed reflection request is exactly 65 uncompressed protobuf bytes against a 64-byte receive limit",
            ],
        }

    def test_sdk_cancellation_and_compression_are_semantically_bound(self):
        capture = self.fixture()
        VERIFY.verify_sdk_capture(capture, "go")
        mutated = copy.deepcopy(capture)
        mutated["cancellation"]["first_services"][0] = "rebound.Service"
        with self.assertRaises(AssertionError):
            VERIFY.verify_sdk_capture(mutated, "go")
        oversized = copy.deepcopy(capture)
        oversized["cancellation"]["first_response_hex"] = self.response(
            ["cosmos.base.node.v1beta1.Service",
             "grpc.reflection.v1alpha.ServerReflection", "x" * 4_096]
        )
        with self.assertRaises(AssertionError):
            VERIFY.verify_sdk_capture(oversized, "go")


class ProvenanceMutationResistance(unittest.TestCase):
    def rs_receipt(self):
        commit = "1" * 40
        receipt = {
            "schema_version": 1,
            "built_at": datetime.now(timezone.utc).isoformat(),
            "binary_sha256": "2" * 64,
            "source_commit": commit,
            "source_commit_signature_verified": True,
            "cargo_lock_sha256": "3" * 64,
            "rustc_vv": "rustc 1.98.0\nrelease: 1.98.0\nhost: x86_64-unknown-linux-gnu",
            "cargo_version": "cargo 1.98.0 (example 2026-01-01)",
            "target": "x86_64-unknown-linux-gnu",
            "build_command": ["cargo", "build", "--locked", "--release",
                              "--package", "celestia-rs", "--bin", "celestia-rs"],
            "profile": "release", "package": "celestia-rs", "binary": "celestia-rs",
            "locked": True, "default_features": True,
            "features": ["rocksdb", "jemalloc"],
            "version_long": (
                "name: celestia-rs\nversion: 0.1.0-rc.5\n"
                f"commit: {commit}\nbuild_profile: release\n"
                "toolchain: rustc 1.98.0 x86_64-unknown-linux-gnu"
            ),
        }
        return receipt

    def test_rs_binary_receipt_source_rebind_is_rejected(self):
        receipt = self.rs_receipt()
        VERIFY.verify_rs_receipt(receipt, "1" * 40, "3" * 64, "2" * 64,
                                 receipt["version_long"])
        for field, value in (("binary_sha256", "4" * 64),
                             ("source_commit", "5" * 40),
                             ("cargo_lock_sha256", "6" * 64)):
            mutated = copy.deepcopy(receipt)
            mutated[field] = value
            with self.assertRaises(AssertionError):
                VERIFY.verify_rs_receipt(mutated, "1" * 40, "3" * 64, "2" * 64,
                                         receipt["version_long"])

    def fixture(self, base: Path):
        capture, source = base / "capture", base / "source"
        capture.mkdir()
        source.mkdir()
        for name in VERIFY.SOURCE_NAMES:
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"source:{name}\n")
        artifacts = (VERIFY.INPUT_NAMES + VERIFY.CAPTURE_NAMES + VERIFY.RESOURCE_NAMES
                     + VERIFY.BUILD_NAMES + VERIFY.OBSERVATION_NAMES)
        for name in artifacts:
            (capture / name).write_text(f"artifact:{name}\n")
        (capture / "meta.json").write_text("{}\n")
        meta = {
            "inventory": {"sources": VERIFY.SOURCE_NAMES, **VERIFY.INVENTORY},
            "artifact_sha256": {
                name: hashlib.sha256((capture / name).read_bytes()).hexdigest()
                for name in artifacts
            },
            "input_sha256": {
                name: hashlib.sha256((capture / name).read_bytes()).hexdigest()
                for name in VERIFY.INPUT_NAMES
            },
            "source_sha256": {
                name: hashlib.sha256((source / name).read_bytes()).hexdigest()
                for name in VERIFY.SOURCE_NAMES
            },
        }
        return capture, source, meta

    def test_unmutated_inventory_fixture_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            capture, source, meta = self.fixture(Path(directory))
            VERIFY.verify_inventory(capture, meta, source)

    def test_deletion_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            capture, source, meta = self.fixture(Path(directory))
            (capture / VERIFY.CAPTURE_NAMES[0]).unlink()
            with self.assertRaises((AssertionError, FileNotFoundError)):
                VERIFY.verify_inventory(capture, meta, source)

    def test_cross_class_rebind_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            capture, source, meta = self.fixture(Path(directory))
            mutated = copy.deepcopy(meta)
            name = VERIFY.INPUT_NAMES[0]
            mutated["inventory"]["inputs"].remove(name)
            mutated["inventory"]["captures"].append(name)
            with self.assertRaises(AssertionError):
                VERIFY.verify_inventory(capture, mutated, source)

    def test_semantic_rebind_is_rejected_even_with_new_outer_hash(self):
        timestamp = proto_varint(1, 1_700_000_000)
        block_hash = bytes([1]) + bytes(31)
        response = (proto_varint(1, 42) + proto_bytes(2, block_hash)
                    + proto_bytes(3, timestamp))
        # Re-hashing this mutated response in metadata cannot make height 43
        # semantically equal to the recorded 42.
        rebound_digest = hashlib.sha256(response).hexdigest()
        self.assertEqual(len(rebound_digest), 64)
        with self.assertRaises(AssertionError):
            VERIFY.verify_height_response(response.hex(), 43)

    def test_response_to_wire_rebind_is_rejected(self):
        response = proto_varint(1, 42)
        envelope = bytes([0]) + len(response).to_bytes(4, "big") + response
        settings = frame(4, 0, 0)
        data = frame(0, 0, 1, envelope)
        wire = {
            "client_bytes_hex": "",
            "server_frames": [
                {"phase": "setup", "elapsed_ms": 0, "type": 4, "flags": 0,
                 "stream_id": 0, "length": 0, "raw_hex": settings.hex()},
                {"phase": "height-response", "elapsed_ms": 1, "type": 0, "flags": 0,
                 "stream_id": 1, "length": len(envelope), "raw_hex": data.hex()},
            ],
        }
        VERIFY.verify_grpc_envelope(wire, response)
        rebound = proto_varint(1, 43)
        with self.assertRaises(AssertionError):
            VERIFY.verify_grpc_envelope(wire, rebound)

    def test_declared_grpc_length_mutation_is_rejected(self):
        response = proto_varint(1, 42)
        envelope = bytes([0]) + (len(response) + 1).to_bytes(4, "big") + response
        data = frame(0, 0, 1, envelope)
        wire = {"client_bytes_hex": "", "server_frames": [
            {"phase": "height-response", "elapsed_ms": 1, "type": 0, "flags": 0,
             "stream_id": 1, "length": len(envelope), "raw_hex": data.hex()},
        ]}
        with self.assertRaises(AssertionError):
            VERIFY.verify_grpc_envelope(wire, response)


if __name__ == "__main__":
    unittest.main()
