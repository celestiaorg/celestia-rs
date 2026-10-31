#!/usr/bin/env python3
"""Offline verification for the v10.5.0 Comet HTTP boundary evidence."""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import gzip
import hashlib
import json
import pathlib
import re
from typing import Any


HERE = pathlib.Path(__file__).parent
DYNAMIC_HEADERS = [
    "connection",
    "content-length",
    "date",
    "transfer-encoding",
    "x-server-time",
]
CASE_INVENTORY = {
    "batch": [
        "custom-batch-at-limit",
        "custom-batch-over-limit",
        "custom-notifications-over-limit",
        "custom-over-limit-with-scalar",
        "zero-batch-four",
        "zero-batch-fifty",
    ],
    "notification": [
        "single-notification",
        "notification-only-batch",
        "mixed-notification-batch",
        "empty-batch",
    ],
    "body-boundary": [
        "body-content-length-511",
        "body-chunked-511",
        "body-content-length-512",
        "body-chunked-512",
        "body-content-length-513",
        "body-chunked-513",
    ],
    "precedence": [
        "preflight-normal-allowed",
        "preflight-normal-denied-origin",
        "preflight-disallowed-request-header",
        "preflight-over-batch",
        "preflight-over-body",
        "zero-preflight-over-body-unread",
        "post-normal-cors",
        "post-over-batch-cors",
        "post-over-body-cors",
        "missing-path-over-batch",
        "malformed-json-under-body",
    ],
    "header-boundary-search": [
        "header-search-pad-4096",
        "header-search-pad-4224",
        "header-search-pad-4256",
        "header-search-pad-4258",
        "header-search-pad-4259",
        "header-search-pad-4260",
        "header-search-pad-4264",
        "header-search-pad-4272",
        "header-search-pad-4288",
        "header-search-pad-4352",
        "header-search-pad-4608",
        "header-search-pad-5120",
        "header-search-pad-6144",
    ],
    "keep-alive": [
        "keepalive-success",
        "keepalive-after-batch-error",
        "keepalive-after-body-error",
    ],
}


class VerificationError(ValueError):
    pass


def require(value: bool, message: str) -> None:
    if not value:
        raise VerificationError(message)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_json(path: pathlib.Path) -> Any:
    return json.loads(path.read_text())


def parse_time(value: str) -> dt.datetime:
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(parsed.tzinfo is not None, f"timestamp lacks timezone: {value}")
    return parsed


def header_lookup(headers: list[list[str]]) -> dict[str, list[str]]:
    result: dict[str, list[str]] = {}
    for name, value in headers:
        result.setdefault(name.lower(), []).append(value)
    return result


def decode_chunks(encoded: bytes, label: str) -> tuple[bytes, list[list[str]]]:
    body = bytearray()
    trailers: list[list[str]] = []
    offset = 0
    while True:
        end = encoded.find(b"\r\n", offset)
        require(end >= 0, f"{label}: unterminated chunk size")
        size = int(encoded[offset:end].split(b";", 1)[0], 16)
        offset = end + 2
        if size == 0:
            while True:
                end = encoded.find(b"\r\n", offset)
                require(end >= 0, f"{label}: unterminated trailer")
                line = encoded[offset:end]
                offset = end + 2
                if not line:
                    require(offset == len(encoded), f"{label}: bytes after trailers")
                    return bytes(body), trailers
                require(b":" in line, f"{label}: malformed trailer")
                name, value = line.split(b":", 1)
                trailers.append([name.decode("latin1"), value.strip().decode("latin1")])
        require(offset + size + 2 <= len(encoded), f"{label}: truncated chunk")
        body.extend(encoded[offset : offset + size])
        offset += size
        require(encoded[offset : offset + 2] == b"\r\n", f"{label}: bad chunk terminator")
        offset += 2


def split_http(raw: bytes, label: str) -> tuple[list[bytes], list[list[str]], bytes]:
    require(b"\r\n\r\n" in raw, f"{label}: missing HTTP header delimiter")
    head, encoded = raw.split(b"\r\n\r\n", 1)
    lines = head.split(b"\r\n")
    headers: list[list[str]] = []
    for line in lines[1:]:
        require(b":" in line, f"{label}: malformed header")
        name, value = line.split(b":", 1)
        headers.append([name.decode("latin1"), value.strip().decode("latin1")])
    return lines, headers, encoded


def verify_request(record: dict[str, Any], label: str) -> None:
    raw = base64.b64decode(record["raw_base64"], validate=True)
    require(sha(raw) == record["raw_sha256"], f"{label}: request hash")
    require(len(raw) == record["raw_size"], f"{label}: request size")
    lines, headers, encoded = split_http(raw, label)
    request_line = lines[0].decode("latin1").split(" ")
    require(request_line == [record["method"], record["target"], "HTTP/1.1"], f"{label}: request line")
    lookup = header_lookup(headers)
    if "chunked" in ",".join(lookup.get("transfer-encoding", [])).lower():
        body, trailers = decode_chunks(encoded, label)
        require(not trailers, f"{label}: unexpected request trailers")
    elif lookup.get("content-length"):
        length = int(lookup["content-length"][-1])
        require(len(encoded) == length, f"{label}: content length")
        body = encoded
    else:
        body = encoded
    require(len(body) == record["decoded_body_size"], f"{label}: decoded body size")


def verify_response(record: dict[str, Any], request_method: str, label: str) -> None:
    raw = base64.b64decode(record["raw_base64"], validate=True)
    require(sha(raw) == record["raw_sha256"], f"{label}: response hash")
    require(len(raw) == record["raw_size"], f"{label}: response size")
    lines, headers, encoded = split_http(raw, label)
    status_line = lines[0].decode("latin1").split(" ", 2)
    require(status_line[0] == record["http_version"], f"{label}: HTTP version")
    require(int(status_line[1]) == record["status"], f"{label}: status")
    require(status_line[2] == record["reason"], f"{label}: reason")
    require(headers == record["headers"], f"{label}: response headers")
    lookup = header_lookup(headers)
    if request_method == "HEAD" or 100 <= record["status"] < 200 or record["status"] in (204, 304):
        require(encoded == b"", f"{label}: body forbidden for response")
        body, trailers = b"", []
    elif "chunked" in ",".join(lookup.get("transfer-encoding", [])).lower():
        body, trailers = decode_chunks(encoded, label)
    elif lookup.get("content-length"):
        length = int(lookup["content-length"][-1])
        require(len(encoded) == length, f"{label}: response content length")
        body, trailers = encoded, []
    else:
        body, trailers = encoded, []
    require(trailers == record["trailers"], f"{label}: response trailers")
    require(base64.b64encode(body).decode() == record["body_base64"], f"{label}: response body bytes")
    require(sha(body) == record["body_sha256"], f"{label}: response body hash")
    if "body_text" in record:
        require(body.decode() == record["body_text"], f"{label}: response body text")
    stable = [pair for pair in headers if pair[0].lower() not in DYNAMIC_HEADERS]
    require(stable == record["stable_headers"], f"{label}: stable response headers")


def verify_exchange(exchange: dict[str, Any], label: str) -> None:
    verify_request(exchange["request"], label)
    verify_response(exchange["response"], exchange["request"]["method"], label)


def stable_headers(case: dict[str, Any]) -> dict[str, list[str]]:
    return header_lookup(case["response"]["stable_headers"])


def rpc_body(case: dict[str, Any]) -> Any:
    return json.loads(case["response"]["body_text"])


def verify_file_manifest(root: pathlib.Path, provenance: dict[str, Any]) -> None:
    for name, expected in provenance["files"].items():
        data = (root / name).read_bytes()
        require(len(data) == expected["size"], f"{name}: file size")
        require(sha(data) == expected["sha256"], f"{name}: file hash")
        if name.endswith(".gz"):
            raw = gzip.decompress(data)
            require(len(raw) == expected["uncompressed_size"], f"{name}: uncompressed size")
            require(sha(raw) == expected["uncompressed_sha256"], f"{name}: uncompressed hash")


def verify_release(root: pathlib.Path, provenance: dict[str, Any], capture: dict[str, Any]) -> None:
    oracle = provenance["oracle"]
    build = load_json(root / oracle["build_manifest"])
    expected = {
        "binary_sha256": oracle["binary_sha256"],
        "compiler": oracle["compiler"],
        "module": oracle["app_release"],
        "source_commit": oracle["app_source_commit"],
    }
    for key, value in expected.items():
        require(build[key] == value, f"build {key}")
    require(build["build_tags"] == ["ledger"], "build tags")
    require(build["target"] == {"CGO_ENABLED": "1", "GOAMD64": "v1", "GOARCH": "amd64", "GOARM": "", "GOEXPERIMENT": "", "GOFIPS140": "off", "GOOS": "linux"}, "build target")
    require((root / oracle["module_receipt"]).read_text() == oracle["app_release"] + "\n", "module receipt")
    build_info = gzip.decompress((root / oracle["build_info"]).read_bytes()).decode()
    require("/workspace/scratch/rpc-181/bin-release/celestia-appd: go1.26.9" in build_info, "build info compiler")
    require("=>\tgithub.com/celestiaorg/celestia-core\tv0.42.3" in build_info, "embedded celestia-core release")
    require("dep\tgithub.com/rs/cors\tv1.11.1" in build_info, "embedded CORS release")

    expected_source_origins = {
        "go-source/core-config.go.gz": "github.com/celestiaorg/celestia-core@v0.42.3/config/config.go",
        "go-source/core-http-json-handler.go.gz": "github.com/celestiaorg/celestia-core@v0.42.3/rpc/jsonrpc/server/http_json_handler.go",
        "go-source/core-http-server.go.gz": "github.com/celestiaorg/celestia-core@v0.42.3/rpc/jsonrpc/server/http_server.go",
        "go-source/core-node.go.gz": "github.com/celestiaorg/celestia-core@v0.42.3/node/node.go",
        "go-source/core-toml.go.gz": "github.com/celestiaorg/celestia-core@v0.42.3/config/toml.go",
        "go-source/cors-utils.go.gz": "github.com/rs/cors@v1.11.1/utils.go",
        "go-source/cors.go.gz": "github.com/rs/cors@v1.11.1/cors.go",
    }
    require(provenance["source_origins"] == expected_source_origins, "frozen source origins")

    sources = {
        name: gzip.decompress((root / name).read_bytes()).decode()
        for name in provenance["files"]
        if name.startswith("go-source/")
    }
    require("func PreChecksHandler(next http.Handler, config *Config) http.Handler" in sources["go-source/core-http-server.go.gz"], "frozen precheck source")
    require("r.Body = http.MaxBytesReader(w, r.Body, config.MaxBodyBytes)" in sources["go-source/core-http-server.go.gz"], "frozen body-limit source")
    require("if config.MaxRequestBatchSize > 0" in sources["go-source/core-http-server.go.gz"], "frozen batch-limit source")
    require("Handler:           PreChecksHandler" in sources["go-source/core-http-server.go.gz"], "frozen precheck wrapper source")
    require("rootHandler = corsMiddleware.Handler(mux)" in sources["go-source/core-node.go.gz"], "frozen CORS wrapper source")
    require("rpcserver.Serve(\n\t\t\t\t\tlistener,\n\t\t\t\t\trootHandler" in sources["go-source/core-node.go.gz"], "frozen node serve source")
    for key in ("max_request_batch_size", "max_body_bytes", "max_header_bytes"):
        require(key in sources["go-source/core-toml.go.gz"], f"frozen TOML source {key}")
    require("MaxHeaderBytes int `mapstructure:\"max_header_bytes\"`" in sources["go-source/core-config.go.gz"], "frozen header config source")
    require("func (c *Cors) handlePreflight" in sources["go-source/cors.go.gz"], "frozen CORS preflight source")

    observation = load_json(root / provenance["release_selection"]["observation"])
    require(observation["chain_id"] == "celestia", "mainnet chain")
    require(observation["version"] == "10.5.0", "mainnet app version")
    parsed = {}
    for name in ("status", "abci_info"):
        response = observation["responses"][name]
        require(sha(response["body"].encode()) == response["sha256"], f"mainnet {name} body hash")
        parsed[name] = json.loads(response["body"])["result"]
    status = parsed["status"]
    abci = parsed["abci_info"]["response"]
    require(status["node_info"]["network"] == observation["chain_id"], "mainnet response chain")
    require(abci["version"] == observation["version"], "mainnet response release")
    require(status["node_info"]["protocol_version"]["app"] == abci["app_version"], "mainnet app protocol agreement")
    require(status["sync_info"]["latest_block_height"] == abci["last_block_height"], "mainnet observation height agreement")
    require(abci["app_version"] == provenance["release_selection"]["observed_protocol_app"], "mainnet selected app protocol")
    require(abci["last_block_height"] == provenance["release_selection"]["observed_height"], "mainnet selected height")
    selected_at = parse_time(observation["observed_at"])
    captured_at = parse_time(capture["captured_at"])
    delta = captured_at - selected_at
    require(dt.timedelta(0) <= delta <= dt.timedelta(hours=24), "mainnet selection was not fresh and preceding")


def toml_scalar(text: str, section: str, key: str) -> str:
    match = re.search(rf"(?ms)^\[{re.escape(section)}\]\s*$((?:(?!^\[).*$\n?)*)", text)
    require(match is not None, f"missing TOML section {section}")
    value = re.search(rf"(?m)^{re.escape(key)}\s*=\s*(.+)$", match.group(1))
    require(value is not None, f"missing TOML {section}.{key}")
    return value.group(1).strip()


def verify_local_oracles(root: pathlib.Path, provenance: dict[str, Any], capture: dict[str, Any]) -> None:
    capture_time = parse_time(capture["captured_at"])
    for name, run in provenance["runs"].items():
        require(capture["oracles"][name]["endpoint"] == run["rpc_endpoint"], f"{name}: endpoint binding")
        require(capture["oracles"][name]["limits"] == {"batch": run["limits"]["max_request_batch_size"], "body": run["limits"]["max_body_bytes"], "header": run["limits"]["max_header_bytes"]}, f"{name}: limit binding")
        identity = load_json(root / run["identity"])
        require(identity["network"] == run["chain_id"], f"{name}: identity chain")
        require(identity["endpoint"] == run["rpc_endpoint"], f"{name}: identity endpoint")
        require(identity["comet_version"] == provenance["oracle"]["comet_reported_version"], f"{name}: Comet version")
        require(identity["app_version"] == "10.5.0", f"{name}: app version")
        require(identity["protocol_app"] == identity["abci_app_version"] == "10", f"{name}: app protocol")
        parsed = {}
        for response_name in ("status", "abci_info"):
            response = identity["responses"][response_name]
            require(sha(response["body"].encode()) == response["sha256"], f"{name}: {response_name} hash")
            parsed[response_name] = json.loads(response["body"])["result"]
        status = parsed["status"]
        abci = parsed["abci_info"]["response"]
        require(status["node_info"]["network"] == identity["network"], f"{name}: status chain")
        require(status["node_info"]["version"] == identity["comet_version"], f"{name}: status Comet version")
        require(status["node_info"]["protocol_version"]["app"] == identity["protocol_app"], f"{name}: status protocol")
        require(abci["version"] == identity["app_version"], f"{name}: ABCI app version")
        require(abci["app_version"] == identity["abci_app_version"], f"{name}: ABCI protocol")
        require(status["sync_info"]["latest_block_height"] == identity["height"] == abci["last_block_height"], f"{name}: local height agreement")
        identity_delta = parse_time(identity["observed_at"]) - capture_time
        require(
            dt.timedelta(0) <= identity_delta <= dt.timedelta(minutes=10),
            f"{name}: identity must follow capture within ten minutes",
        )

        config = gzip.decompress((root / run["config"]).read_bytes()).decode()
        require(toml_scalar(config, "rpc", "laddr") == f'"tcp://127.0.0.1:{6320 if name == "zero" else 6330}"', f"{name}: RPC listener")
        require(json.loads(toml_scalar(config, "rpc", "cors_allowed_origins")) == ["https://allowed.example"], f"{name}: CORS origin")
        require(json.loads(toml_scalar(config, "rpc", "cors_allowed_methods")) == ["HEAD", "GET", "POST"], f"{name}: CORS methods")
        require("X-Test" in json.loads(toml_scalar(config, "rpc", "cors_allowed_headers")), f"{name}: CORS header")
        for config_key, value in run["limits"].items():
            require(int(toml_scalar(config, "rpc", config_key)) == value, f"{name}: {config_key}")
        genesis = json.loads(gzip.decompress((root / run["genesis"]).read_bytes()))
        require(genesis["chain_id"] == run["chain_id"], f"{name}: genesis chain")
    app = gzip.decompress((root / "app.toml.gz").read_bytes()).decode()
    require(toml_scalar(app, "api", "enable") == "false", "API disabled")
    require(toml_scalar(app, "grpc", "enable") == "false", "app gRPC disabled")


def verify_cases(capture: dict[str, Any]) -> None:
    require(capture["schema_version"] == 1, "capture schema")
    require(capture["dynamic_response_headers_excluded"] == DYNAMIC_HEADERS, "dynamic header exclusion set")
    actual: dict[str, list[str]] = {category: [] for category in CASE_INVENTORY}
    by_name = {}
    for case in capture["cases"]:
        require(case["category"] in actual, f"unexpected category {case['category']}")
        require(case["name"] not in by_name, f"duplicate case {case['name']}")
        actual[case["category"]].append(case["name"])
        by_name[case["name"]] = case
        if case["category"] == "keep-alive":
            for index, exchange in enumerate(case["exchanges"]):
                verify_exchange(exchange, f"{case['name']}[{index}]")
        else:
            verify_exchange(case, case["name"])
    require(actual == CASE_INVENTORY, "exact case/category inventory")
    require(len(by_name) == 43, "case count")

    for name in ("custom-batch-over-limit", "custom-notifications-over-limit", "preflight-over-batch", "post-over-batch-cors", "missing-path-over-batch"):
        case = by_name[name]
        require(case["response"]["status"] == 200, f"{name}: status")
        require(rpc_body(case)["error"] == {"code": -32600, "message": "Invalid Request", "data": "batch request exceeds maximum (3) allowed number of requests"}, f"{name}: batch error")
        require("access-control-allow-origin" not in stable_headers(case), f"{name}: CORS bypass")
    require(len(rpc_body(by_name["custom-batch-at-limit"])) == 3, "batch-at-limit result count")
    require(len(rpc_body(by_name["zero-batch-four"])) == 4, "zero limit accepts four")
    require(len(rpc_body(by_name["zero-batch-fifty"])) == 50, "zero limit accepts fifty")
    scalar = by_name["custom-over-limit-with-scalar"]
    require(scalar["response"]["status"] == 500 and rpc_body(scalar)["error"]["code"] == -32700, "invalid batch element parse precedence")

    for name in ("single-notification", "notification-only-batch", "empty-batch"):
        require(by_name[name]["response"]["status"] == 200, f"{name}: status")
        require(by_name[name]["response"]["body_text"] == "", f"{name}: empty response")
    require(rpc_body(by_name["mixed-notification-batch"]) == {"jsonrpc": "2.0", "id": 71, "result": {}}, "mixed notification response")

    for transfer in ("content-length", "chunked"):
        for size in (511, 512):
            case = by_name[f"body-{transfer}-{size}"]
            require(case["response"]["status"] == 200, f"{case['name']}: accepted")
        case = by_name[f"body-{transfer}-513"]
        require(case["response"]["status"] == 400, f"{case['name']}: rejected")
        require(rpc_body(case)["error"]["data"] == "error reading request body: http: request body too large", f"{case['name']}: body error")
        require("access-control-allow-origin" not in stable_headers(case), f"{case['name']}: CORS bypass")

    cors = by_name["preflight-normal-allowed"]
    require(cors["response"]["status"] == 204, "allowed preflight status")
    require(stable_headers(cors) == {"access-control-allow-methods": ["POST"], "access-control-allow-origin": ["https://allowed.example"], "vary": ["Origin, Access-Control-Request-Method, Access-Control-Request-Headers"]}, "allowed preflight headers")
    for name in ("preflight-normal-denied-origin", "preflight-disallowed-request-header"):
        require(by_name[name]["response"]["status"] == 204, f"{name}: status")
        require(stable_headers(by_name[name]) == {"vary": ["Origin, Access-Control-Request-Method, Access-Control-Request-Headers"]}, f"{name}: stable headers")
    require(by_name["preflight-over-batch"]["response"]["status"] == 200, "batch precheck precedes CORS")
    for name in ("preflight-over-body", "post-over-body-cors"):
        case = by_name[name]
        require(case["response"]["status"] == 400, f"{name}: body precheck status")
        require(
            rpc_body(case)["error"]
            == {
                "code": -32600,
                "message": "Invalid Request",
                "data": "error reading request body: http: request body too large",
            },
            f"{name}: body precheck error",
        )
        require(
            "access-control-allow-origin" not in stable_headers(case),
            f"{name}: body precheck bypasses CORS",
        )
    zero_preflight = by_name["zero-preflight-over-body-unread"]
    require(zero_preflight["response"]["status"] == 204 and zero_preflight["request"]["decoded_body_size"] == 4097, "zero batch preflight leaves oversized body unread")
    require(zero_preflight["response"]["body_text"] == "", "zero batch preflight has empty response")
    require(
        stable_headers(zero_preflight)
        == {
            "access-control-allow-methods": ["POST"],
            "access-control-allow-origin": ["https://allowed.example"],
            "vary": ["Origin, Access-Control-Request-Method, Access-Control-Request-Headers"],
        },
        "zero batch preflight reaches CORS",
    )
    require("access-control-allow-origin" in stable_headers(by_name["post-normal-cors"]), "normal POST reaches CORS")
    require(by_name["malformed-json-under-body"]["response"]["status"] == 500 and rpc_body(by_name["malformed-json-under-body"])["error"]["code"] == -32700, "malformed JSON handler error")

    require(capture["header_search"] == {"last_accepted_pad": 4259, "first_rejected_pad": 4260}, "header search result")
    header_cases = [by_name[name] for name in CASE_INVENTORY["header-boundary-search"]]
    pads = [case["metadata"]["pad_bytes"] for case in header_cases]
    require(pads == sorted(pads), "header probes sorted")
    require(all(case["metadata"]["configured_limit"] == 256 for case in header_cases), "header probe configured limit")
    accepted = by_name["header-search-pad-4259"]
    rejected = by_name["header-search-pad-4260"]
    require(accepted["request"]["raw_size"] == 4352 and accepted["response"]["status"] == 200, "last accepted header request")
    require(rejected["request"]["raw_size"] == 4353 and rejected["response"]["status"] == 431, "first rejected header request")
    require(all(case["response"]["status"] == (200 if case["metadata"]["pad_bytes"] <= 4259 else 431) for case in header_cases), "header probe monotonic boundary")
    require(rejected["response"]["body_text"] == "431 Request Header Fields Too Large", "header rejection body")

    expected_keepalive = {
        "keepalive-success": ("second-response", [200, 200]),
        "keepalive-after-batch-error": ("second-response", [200, 200]),
        "keepalive-after-body-error": ("closed-before-second-response:EOFError", [400]),
    }
    for name, (outcome, statuses) in expected_keepalive.items():
        case = by_name[name]
        require(case["outcome"] == outcome, f"{name}: outcome")
        require([exchange["response"]["status"] for exchange in case["exchanges"]] == statuses, f"{name}: statuses")
        first_headers = header_lookup(split_http(base64.b64decode(case["exchanges"][0]["request"]["raw_base64"]), name)[1])
        require(first_headers["connection"] == ["keep-alive"], f"{name}: first request keep-alive")
    require("connection" not in header_lookup(by_name["keepalive-after-batch-error"]["exchanges"][0]["response"]["headers"]), "batch error keeps connection reusable")
    require(header_lookup(by_name["keepalive-after-body-error"]["exchanges"][0]["response"]["headers"])["connection"] == ["close"], "body error closes connection")


def verify_bundle(root: pathlib.Path = HERE) -> None:
    provenance = load_json(root / "provenance.json")
    require(provenance["schema_version"] == 1, "provenance schema")
    require(provenance["commands"] == {
        "zero_oracle": ["./start-source.sh", "zero", "6320", "6321", "0", "4096", "256"],
        "custom_oracle": ["./start-source.sh", "custom", "6330", "6331", "3", "512", "256"],
        "capture": ["python3", "capture-source.py"],
        "local_identity": ["python3", "observe-local-source.py"],
    }, "capture commands")
    require(provenance["dynamic_response_headers_excluded_from_stable_projection"] == DYNAMIC_HEADERS, "provenance dynamic headers")
    verify_file_manifest(root, provenance)
    capture = load_json(root / "comet-http-boundaries.json")
    sources = capture["sources_sha256"]
    require(sources["/workspace/scratch/rpc-181/comet-http/capture.py"] == provenance["files"]["capture-source.py"]["sha256"], "capture source binding")
    require(sources["/workspace/scratch/rpc-181/comet-http/start-oracle.sh"] == provenance["files"]["start-source.sh"]["sha256"], "start source binding")
    require(sources["/workspace/scratch/rpc-181/comet-http/mainnet-observation.json"] == provenance["files"]["mainnet-observation.json"]["sha256"], "mainnet source binding")
    verify_release(root, provenance, capture)
    verify_local_oracles(root, provenance, capture)
    verify_cases(capture)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", type=pathlib.Path, default=HERE)
    args = parser.parse_args()
    verify_bundle(args.root)
    print("verified 43 tagged-Go Comet HTTP boundary cases")


if __name__ == "__main__":
    main()
