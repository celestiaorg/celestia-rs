#!/usr/bin/env python3
"""Verify the frozen v10.5.0 REST POST/body boundary evidence offline."""

from __future__ import annotations

import base64
import datetime
import gzip
import hashlib
import json
import pathlib
from typing import Any


HERE = pathlib.Path(__file__).parent / "v10.5.0"
REPOSITORY = pathlib.Path(__file__).resolve().parents[3]
DYNAMIC_HEADERS = {
    "connection",
    "content-length",
    "date",
    "trailer",
    "transfer-encoding",
    "x-server-time",
}
EXPECTED_CATEGORIES = {
    "content-type": 7,
    "escaping": 8,
    "method": 7,
    "protobuf-json": 32,
    "success-control": 3,
    "supported-route": 18,
    "unknown-route": 7,
}
ROUTES = {
    "batch": "/celestia/core/v1/tx/batch",
    "decode": "/cosmos/tx/v1beta1/decode",
    "decode-amino": "/cosmos/tx/v1beta1/decode/amino",
    "encode": "/cosmos/tx/v1beta1/encode",
    "encode-amino": "/cosmos/tx/v1beta1/encode/amino",
    "simulate": "/cosmos/tx/v1beta1/simulate",
    "broadcast": "/cosmos/tx/v1beta1/txs",
    "fibre-validate": "/fibre/v1/validate-payment-promise",
    "ibc-membership": "/ibc/core/client/v1/verify_membership",
}
EXPECTED_NAMES = {
    "content-type": {
        "content-type-absent",
        "content-type-json",
        "content-type-json-charset",
        "content-type-text",
        "content-type-octet",
        "content-type-empty",
        "content-type-repeated",
    },
    "method": {
        "method-get",
        "method-put",
        "method-patch",
        "method-delete",
        "method-options",
        "method-head",
        "method-post",
    },
    "unknown-route": {
        "unknown-root",
        "unknown-known-service",
        "unknown-trailing-slash",
        "unknown-double-slash",
        "unknown-encoded-name",
        "unknown-encoded-slash",
        "unknown-invalid-percent",
    },
    "escaping": {
        "escape-post-unused",
        "escape-post-field",
        "escape-get-unknown",
        "escape-get-pagination",
        "escape-path-encoded-slash",
        "escape-path-percent",
        "escape-path-plus",
        "escape-query-repeated",
    },
}
REQUIRED_PROTOBUF_JSON = {
    "json-malformed",
    "json-trailing-json",
    "json-duplicate-alias",
    "json-bytes-number",
    "json-bytes-array",
    "json-bytes-object",
    "json-bytes-null",
    "json-bytes-invalid-base64",
    "broadcast-enum-unknown",
    "broadcast-enum-large",
    "broadcast-enum-float",
    "ibc-int-overflow",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_digest(path: pathlib.Path) -> str:
    return digest(path.read_bytes())


def decode_b64(value: str, label: str) -> bytes:
    try:
        return base64.b64decode(value, validate=True)
    except Exception as error:
        fail(f"{label}: invalid base64: {error}")


def parse_headers(raw: bytes, label: str) -> tuple[bytes, list[list[str]], bytes]:
    head, separator, body = raw.partition(b"\r\n\r\n")
    if not separator:
        fail(f"{label}: missing HTTP header terminator")
    lines = head.split(b"\r\n")
    headers: list[list[str]] = []
    for line in lines[1:]:
        if b":" not in line:
            fail(f"{label}: malformed HTTP header")
        name, value = line.split(b":", 1)
        headers.append([name.decode("latin1"), value.strip().decode("latin1")])
    return lines[0], headers, body


def values(headers: list[list[str]], wanted: str) -> list[str]:
    return [value for name, value in headers if name.lower() == wanted]


def parse_response(
    raw: bytes, method: str, label: str
) -> tuple[int, str, list[list[str]], list[list[str]], bytes]:
    first, headers, encoded = parse_headers(raw, label)
    try:
        _, status_raw, reason_raw = first.split(b" ", 2)
        status = int(status_raw)
        reason = reason_raw.decode("latin1")
    except (ValueError, UnicodeDecodeError) as error:
        fail(f"{label}: malformed response status line: {error}")
    trailers: list[list[str]] = []
    if method == "HEAD":
        body = b""
    elif "chunked" in ",".join(values(headers, "transfer-encoding")).lower():
        chunks = bytearray()
        offset = 0
        while True:
            end = encoded.find(b"\r\n", offset)
            if end < 0:
                fail(f"{label}: truncated chunk size")
            try:
                size = int(encoded[offset:end].split(b";", 1)[0], 16)
            except ValueError as error:
                fail(f"{label}: malformed chunk size: {error}")
            offset = end + 2
            if size == 0:
                trailer_end = encoded.find(b"\r\n\r\n", offset)
                trailer_raw = encoded[offset:] if trailer_end < 0 else encoded[offset:trailer_end]
                for line in trailer_raw.split(b"\r\n"):
                    if not line:
                        continue
                    if b":" not in line:
                        fail(f"{label}: malformed trailer")
                    name, value = line.split(b":", 1)
                    trailers.append(
                        [name.decode("latin1"), value.strip().decode("latin1")]
                    )
                break
            chunk = encoded[offset : offset + size]
            if len(chunk) != size or encoded[offset + size : offset + size + 2] != b"\r\n":
                fail(f"{label}: truncated chunk")
            chunks.extend(chunk)
            offset += size + 2
        body = bytes(chunks)
    else:
        lengths = values(headers, "content-length")
        body = encoded[: int(lengths[-1])] if lengths else encoded
    return status, reason, headers, trailers, body


def verify_request(case: dict[str, Any]) -> None:
    name = case["name"]
    request = case["request"]
    raw = decode_b64(request["raw_base64"], f"{name} request raw")
    if len(raw) != request["raw_size"] or digest(raw) != request["raw_sha256"]:
        fail(f"{name}: request raw hash/size mismatch")
    body_bytes = decode_b64(request["body_base64"], f"{name} request body")
    complete_headers = [
        ["Host", "127.0.0.1:6317"],
        ["Connection", "close"],
        *request["headers"],
    ]
    if not any(
        header.lower() in {"content-length", "transfer-encoding"}
        for header, _ in complete_headers
    ):
        complete_headers.append(["Content-Length", str(len(body_bytes))])
    expected = (
        "\r\n".join(
            [
                f"{request['method']} {request['target']} HTTP/1.1",
                *(f"{header}: {value}" for header, value in complete_headers),
            ]
        )
        + "\r\n\r\n"
    ).encode("latin1") + body_bytes
    if raw != expected:
        fail(f"{name}: raw request does not match structured request fields")
    first, _, body = parse_headers(raw, f"{name} request")
    try:
        method, target, version = first.decode("latin1").split(" ", 2)
    except ValueError as error:
        fail(f"{name}: malformed request line: {error}")
    if version != "HTTP/1.1" or method != request["method"] or target != request["target"]:
        fail(f"{name}: request line does not match structured fields")
    if body != body_bytes:
        fail(f"{name}: request body does not match raw request")


def verify_response(case: dict[str, Any]) -> None:
    name = case["name"]
    response = case["response"]
    raw = decode_b64(response["raw_base64"], f"{name} response raw")
    if len(raw) != response["raw_size"] or digest(raw) != response["raw_sha256"]:
        fail(f"{name}: response raw hash/size mismatch")
    status, reason, headers, trailers, body = parse_response(
        raw, case["request"]["method"], f"{name} response"
    )
    if status != response["status"] or reason != response["reason"]:
        fail(f"{name}: response status does not match raw response")
    if headers != response["headers"] or trailers != response["trailers"]:
        fail(f"{name}: response headers/trailers do not match raw response")
    stable = [pair for pair in headers if pair[0].lower() not in DYNAMIC_HEADERS]
    if stable != response["stable_headers"]:
        fail(f"{name}: stable response header projection is wrong")
    if any(name.lower() in DYNAMIC_HEADERS for name, _ in stable):
        fail(f"{name}: dynamic response header retained as stable")
    if body != decode_b64(response["body_base64"], f"{name} response body"):
        fail(f"{name}: response body does not match raw response")
    if digest(body) != response["body_sha256"]:
        fail(f"{name}: response body hash mismatch")
    if "body_text" in response and response["body_text"].encode() != body:
        fail(f"{name}: response body text does not match response bytes")
    if not headers:
        fail(f"{name}: response has no captured headers")


def verify_mainnet_observation(observation: dict[str, Any]) -> None:
    if observation["chain_id"] != "celestia" or observation["version"] != "10.5.0":
        fail("mainnet observation does not select celestia-app v10.5.0")
    decoded_responses = {}
    for name, response in observation["responses"].items():
        body = response["body"]
        if digest(body.encode()) != response["sha256"]:
            fail(f"mainnet {name} response body hash mismatch")
        decoded_responses[name] = json.loads(body)["result"]
    status = decoded_responses["status"]
    abci = decoded_responses["abci_info"]["response"]
    if status["node_info"]["network"] != observation["chain_id"]:
        fail("mainnet status chain does not match outer observation")
    if abci["version"] != observation["version"]:
        fail("mainnet ABCI version does not match outer observation")
    if str(status["node_info"]["protocol_version"]["app"]) != str(
        abci["app_version"]
    ):
        fail("mainnet status and ABCI protocol app versions disagree")
    if str(status["sync_info"]["latest_block_height"]) != str(
        abci["last_block_height"]
    ):
        fail("mainnet status and ABCI heights disagree")


def verify_provenance(provenance: dict[str, Any]) -> None:
    for name, expected in provenance["files"].items():
        if file_digest(HERE / name) != expected:
            fail(f"provenance hash mismatch: {name}")
    for name, expected in provenance["repository_inputs"].items():
        if file_digest(REPOSITORY / name) != expected:
            fail(f"repository input hash mismatch: {name}")
    cross_checks = {
        "oracle build recipe": (
            provenance["oracle"]["build_recipe_sha256"],
            provenance["files"]["oracle-build.json"],
        ),
        "oracle module file": (
            provenance["oracle"]["module_file_sha256"],
            provenance["files"]["oracle-module.txt"],
        ),
        "mainnet observation": (
            provenance["release_selection"]["observation_sha256"],
            provenance["files"]["mainnet-observation.json"],
        ),
        "Go state": (
            provenance["state"]["state_sha256"],
            provenance["files"]["go-state.json"],
        ),
        "genesis": (
            provenance["state"]["genesis_sha256"],
            provenance["files"]["genesis.json"],
        ),
        "capture": (
            provenance["capture"]["capture_sha256"],
            provenance["files"]["rest-post-boundaries.json"],
        ),
        "capture source": (
            provenance["capture"]["source_sha256"],
            provenance["files"]["capture-source.py"],
        ),
        "start source": (
            provenance["capture"]["start_source_sha256"],
            provenance["files"]["start-source.sh"],
        ),
        "replay receipt": (
            provenance["replay"]["receipt_sha256"],
            provenance["files"]["replay-receipt.json"],
        ),
        "frozen replay source": (
            provenance["replay"]["frozen_source_sha256"],
            provenance["files"]["replay-source.py"],
        ),
        "RPC recorder build receipt": (
            provenance["rpc_recorder"]["build_receipt_sha256"],
            provenance["files"]["rpc-recorder-build.json"],
        ),
    }
    for label, (left, right) in cross_checks.items():
        if left != right:
            fail(f"{label} hashes disagree within provenance")

    build = json.loads((HERE / "oracle-build.json").read_text())
    oracle = provenance["oracle"]
    for key in ("module", "source_commit", "compiler", "build_tags", "binary_sha256"):
        if build[key] != oracle[key]:
            fail(f"oracle {key} does not match build recipe")
    if build["module"] != "github.com/celestiaorg/celestia-app/v10@v10.5.0":
        fail("unexpected oracle module")
    if build["source_commit"] != "6013e758d5dceb75827ad191c8d739d7c43cfa04":
        fail("unexpected oracle source commit")
    if oracle["standalone"] is not True or oracle["with_comet"] is not False:
        fail("oracle execution mode is not explicitly bound")
    if (HERE / "oracle-module.txt").read_text().strip() != build["module"]:
        fail("oracle module file does not match build recipe")

    state = json.loads((HERE / "go-state.json").read_text())
    for key in ("chain_id", "version", "height", "app_hash", "replayed_requests"):
        if state[key] != provenance["state"][key]:
            fail(f"state {key} does not match provenance")
    if state["version"] != "10.5.0" or state["height"] != 20:
        fail("unexpected replayed Go state")

    observation = json.loads((HERE / "mainnet-observation.json").read_text())
    selection = provenance["release_selection"]
    for key in ("endpoint", "observed_at", "chain_id", "version"):
        if observation[key] != selection[key]:
            fail(f"mainnet selection {key} does not match observation")
    verify_mainnet_observation(observation)

    build_info_bytes = gzip.decompress((HERE / "oracle-build-info.txt.gz").read_bytes())
    if digest(build_info_bytes) != oracle["build_info_sha256"]:
        fail("uncompressed oracle build information hash mismatch")
    build_info = build_info_bytes.decode()
    for marker in ("go1.26.9", "github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd"):
        if marker not in build_info:
            fail(f"oracle build information is missing {marker}")
    app_config = gzip.decompress((HERE / "app.toml.gz").read_bytes())
    if digest(app_config) != provenance["state"]["app_config_sha256"]:
        fail("uncompressed application configuration hash mismatch")

    replay = json.loads((HERE / "replay-receipt.json").read_text())
    expected_replay_command = (
        "python3 /workspace/worktrees/rpc-191/conformance/mainnet-rpc/simulation/"
        "capture-state.py --address 127.0.0.1:40658 --replay /workspace/"
        "worktrees/rpc-191/conformance/fixtures/devnet/empty/abci.rec --genesis "
        "/workspace/scratch/rpc-181/rest-post/home/config/genesis.json --out "
        "/workspace/scratch/rpc-181/rest-post/go-state.json"
    )
    if replay["command"] != expected_replay_command:
        fail("exact replay command changed")
    if provenance["replay"]["capture_state_source_sha256"] != provenance[
        "replay"
    ]["frozen_source_sha256"]:
        fail("frozen replay source does not match the invoked source")
    if replay["request_count"] != 194 or replay["result"] != state:
        fail("replay receipt does not bind the accepted 194-request state")
    if replay["output_sha256"] != provenance["state"]["state_sha256"]:
        fail("replay output hash does not match accepted Go state")
    replay_paths = {
        "capture_state_source": "conformance/mainnet-rpc/simulation/capture-state.py",
        "abci_recording": "conformance/fixtures/devnet/empty/abci.rec",
        "genesis": "conformance/fixtures/devnet/empty/genesis.json",
    }
    for label, path in replay_paths.items():
        entry = replay["inputs"][label]
        if entry["path"] != path or entry["sha256"] != provenance["repository_inputs"][path]:
            fail(f"replay {label} identity mismatch")
    if replay["inputs"]["start_source"]["sha256"] != provenance["capture"][
        "start_source_sha256"
    ]:
        fail("replay start source identity mismatch")
    if replay["inputs"]["app_config_uncompressed_sha256"] != provenance["state"][
        "app_config_sha256"
    ]:
        fail("replay application configuration identity mismatch")
    if replay["inputs"]["oracle_binary_sha256"] != provenance["oracle"][
        "binary_sha256"
    ]:
        fail("replay oracle binary identity mismatch")
    lineage = replay["recording_lineage"]
    if lineage["fixture_metadata_sha256"] != provenance["repository_inputs"][
        "conformance/fixtures/devnet/empty/meta.json"
    ]:
        fail("ABCI recording fixture metadata identity mismatch")
    if lineage["devnet_source_sha256"] != provenance["repository_inputs"][
        "conformance/devnet/devnet.sh"
    ]:
        fail("ABCI recording devnet source identity mismatch")

    recorder = json.loads((HERE / "rpc-recorder-build.json").read_text())
    recorder_provenance = provenance["rpc_recorder"]
    for key in ("binary_sha256", "build_info_sha256"):
        if recorder[key] != recorder_provenance[key]:
            fail(f"RPC recorder {key} does not match provenance")
    if recorder["compiler"] != "go version go1.26.9 linux/amd64":
        fail("unexpected RPC recorder compiler")
    recorder_info = gzip.decompress(
        (HERE / "rpc-recorder-build-info.txt.gz").read_bytes()
    )
    if digest(recorder_info) != recorder["build_info_sha256"]:
        fail("uncompressed RPC recorder build information hash mismatch")
    for marker in ("go1.26.9", recorder["module"]):
        if marker.encode() not in recorder_info:
            fail(f"RPC recorder build information is missing {marker}")
    for path, expected in recorder["source_sha256"].items():
        if file_digest(REPOSITORY / path) != expected:
            fail(f"RPC recorder source hash mismatch: {path}")


def verify_capture(capture: dict[str, Any], provenance: dict[str, Any]) -> None:
    if capture["schema_version"] != 1:
        fail("unexpected capture schema")
    if set(capture["dynamic_response_headers_excluded"]) != DYNAMIC_HEADERS:
        fail("dynamic response header exclusion does not match verifier")
    if capture["captured_at"] != provenance["capture"]["captured_at"]:
        fail("capture timestamp does not match provenance")
    observed_at = datetime.datetime.fromisoformat(provenance["release_selection"]["observed_at"])
    captured_at = datetime.datetime.fromisoformat(capture["captured_at"])
    if not datetime.timedelta(0) <= captured_at - observed_at <= datetime.timedelta(hours=24):
        fail("mainnet release selection was not observed within 24 hours before capture")

    cases = capture["cases"]
    if len(cases) != 82 or len(cases) != provenance["capture"]["case_count"]:
        fail("expected 82 REST boundary cases")
    names = [case["name"] for case in cases]
    if len(names) != len(set(names)):
        fail("case names are not unique")
    counts = {
        category: sum(case["category"] == category for case in cases)
        for category in EXPECTED_CATEGORIES
    }
    if counts != EXPECTED_CATEGORIES:
        fail(f"unexpected category coverage: {counts}")
    inventory = json.loads((HERE / "case-inventory.json").read_text())
    actual_inventory: dict[str, list[str]] = {}
    for case in cases:
        actual_inventory.setdefault(case["category"], []).append(case["name"])
    actual_inventory = {
        category: sorted(category_names)
        for category, category_names in sorted(actual_inventory.items())
    }
    if (
        inventory["case_count"] != len(cases)
        or inventory["categories"] != actual_inventory
    ):
        fail("complete REST case inventory does not match the capture")
    for category, expected in EXPECTED_NAMES.items():
        actual = {case["name"] for case in cases if case["category"] == category}
        if actual != expected:
            fail(f"unexpected {category} coverage: {sorted(actual)}")
    protobuf_names = {
        case["name"] for case in cases if case["category"] == "protobuf-json"
    }
    if not REQUIRED_PROTOBUF_JSON <= protobuf_names:
        fail("required malformed/edge protobuf JSON cases are missing")

    supported_names = {
        case["name"].removeprefix("route-").removesuffix(suffix)
        for case in cases
        if case["category"] == "supported-route"
        for suffix in ("-empty-object", "-empty-body")
        if case["name"].endswith(suffix)
    }
    if supported_names != set(ROUTES):
        fail(f"unexpected supported POST route coverage: {sorted(supported_names)}")
    by_name = {case["name"]: case for case in cases}
    for route, target in ROUTES.items():
        for suffix, expected_body in (("empty-object", b"{}"), ("empty-body", b"")):
            name = f"route-{route}-{suffix}"
            if name not in by_name:
                fail(f"missing supported route case: {route} {suffix}")
            request = by_name[name]["request"]
            if request["method"] != "POST" or request["target"] != target:
                fail(f"{name}: supported route request is wrong")
            if decode_b64(request["body_base64"], f"{name} request body") != expected_body:
                fail(f"{name}: supported route body is wrong")

    for case in cases:
        verify_request(case)
        verify_response(case)
        if case["response"]["status"] not in range(100, 600):
            fail(f"{case['name']}: invalid HTTP response status")
        body = case["response"].get("body_text", "").lower()
        if "connection refused" in body or "127.0.0.1:26657" in body:
            fail(f"{case['name']}: environment-specific Comet failure admitted")
        if case["response"]["trailers"]:
            fail(f"{case['name']}: unexpected emitted trailer fields")

    for name in ("decode-valid-tx", "simulate-valid-tx", "encode-decoded-roundtrip"):
        if by_name[name]["response"]["status"] != 200:
            fail(f"{name}: success control did not succeed")
    original = json.loads(
        decode_b64(
            by_name["decode-valid-tx"]["request"]["body_base64"],
            "decode-valid-tx request body",
        )
    )["tx_bytes"]
    roundtrip = json.loads(by_name["encode-decoded-roundtrip"]["response"]["body_text"])[
        "tx_bytes"
    ]
    if original != roundtrip:
        fail("decode/encode success control did not round-trip the transaction bytes")

    source_hashes = set(capture["source_sha256"].values())
    required_sources = {
        provenance["capture"]["source_sha256"],
        provenance["repository_inputs"][
            "conformance/mainnet-rpc/simulation/requests.json"
        ],
        provenance["repository_inputs"][
            "conformance/fixtures/devnet/empty/abci.rec"
        ],
        provenance["repository_inputs"][
            "conformance/fixtures/devnet/empty/genesis.json"
        ],
    }
    if not required_sources <= source_hashes:
        fail("capture source/input hashes are not bound inside the capture")


def main() -> None:
    provenance = json.loads((HERE / "provenance.json").read_text())
    verify_provenance(provenance)
    capture = json.loads((HERE / "rest-post-boundaries.json").read_text())
    verify_capture(capture, provenance)
    print("verified 82 v10.5.0 REST POST/body boundary cases and provenance")


if __name__ == "__main__":
    main()
