#!/usr/bin/env python3
"""Offline verifier for the v10.5.0 proxy/TLS/malformed-framing evidence."""

from __future__ import annotations

import base64
import datetime as dt
import gzip
import hashlib
import json
import pathlib
import re
from typing import Any


HERE = pathlib.Path(__file__).parent
DYNAMIC_HEADERS = ["connection", "content-length", "date", "transfer-encoding", "x-server-time"]
CASE_DIGESTS = {
    'tls-health-http1': '27bce7b3b3dff41189761f68e6776cb125179574dbed2534d3d8a67003d2dc65',
    'tls-health-http2': '44e66ba71dd241a5a6e1780ca83bbe3ee20ac4089c355568695617b420bc6fd7',
    'tls-jsonrpc-health-http1': 'cf49cbce720c6103aaf086a3d3c05f6a5bc8e73f286c1d29c3214df90ce0f5df',
    'tls-cors-preflight-http1': '86d5299a59a3454eff23a881ec20b5f57c1fcc13f61d3f68200dd0a7e2bed6cc',
    'tls-wss-jsonrpc-clean-close': '310a40f2aa600a930cde05a5106548a7ca803d165023de9154d2dbbe4c4541b8',
    'tls-node-rest-get-latest-block': 'b470681e88489295fc4a34e35ee50242ad90e2653ed5347c38800d1b7ed45737',
    'tls-certificate-name-mismatch': '0fb67af39f0220a620b35b041efdb57df9c10222cc7d83af67ea29e760b27cd2',
    'plaintext-on-tls-port': 'c2c01bc9ee8c32cea790adc7cdd0e39aeee8485a55493fe83f860d183b0bf022',
    'tls-on-plaintext-port': '0e4fbeb7f8691552957b85adfa409cfea8c48fe4ca3380232a164eb799472094',
    'proxy-root-internal-host': 'a7c5041a63407eb1e18b10b20b891ab77d5f3550ccf67ed20fd197943ee52424',
    'proxy-root-public-host': 'c7878ed05ece36070b012ee765a82b237deca42a8fea3a7c7ddd3e3ea54aa632',
    'proxy-forwarded-headers-inert': '3058e4efab24478b57be84f3a4d15f7f3cd206edd6c4b9e137663574f899171f',
    'proxy-cors-forwarded-headers-inert': 'fa515a8366491a4867a69a0381d7392e4e55b7a11894d9aabb781cd088f41cd1',
    'proxy-health-forwarded-headers-inert': '3c3a7e5c747b1d8caf3db05bea64217b89ff290bea76ccf7c338c035b2c8887f',
    'proxy-absolute-form-target': '6f70ee971a62146a5a3604bff38b7c063335e9f574d2039a688b7091bbe24298',
    'framing-content-length-duplicate-same': '2f2b621b4e51f02f6986236b6052629e5c93ead1fd5f703c0da5a638503d6d2a',
    'framing-content-length-comma-same': '814073a6a144270f8ca4e3273ec0985ce8bb605700c8932a603b759b159c5b13',
    'framing-transfer-encoding-with-content-length': '00239fe692be8c8ff7e491a26e2f3d6198f375cab878073e3e2c8febf942c088',
    'framing-chunk-extension': '8aacc56dff3abf6136c259fc908ce38baeb213cf5b2386fbb82676371324fb4b',
    'framing-declared-trailer': '4e32ffc812e36ad799842aedd5ed6166605ff85afb9d46de9477cd3616f6e34d',
    'framing-missing-host': 'e2ace89cafe8018e7199101777b0f7bfd63c48a943b58967c782e54c659a184e',
    'framing-duplicate-host': 'd8bed89022121086e25e002040998d046e32d595a1dc271efa8d7e0f59428ba2',
    'framing-header-name-space': '9cfff0cdc6cd2562af50ab923c0bda32a4ea4c8f1152190509e1ff889bfcbc01',
    'framing-header-without-colon': '8669e275e3b50c4358c2e3900d9167f17a9f32eb4a070cd1c32524696d0d0ffb',
    'framing-obs-fold': 'dd74e35a5061fc675a28634cf1c70eb1b0c1ebd0f8f3a5f65231abf309e26151',
    'framing-nul-header-value': 'a264c720cb4302132436de45fe98f6f324423e4046168792c9c54a586930fe46',
    'framing-invalid-method-token': '4acb9e038f08954501ce0dacc853be02be561291247deb3ba9975eacf8322720',
    'framing-invalid-http-version': '41e0777689713562245c838eba447afc976beffbe44f9d83b9616677662ddb76',
    'framing-invalid-target-control': '2081c9e925af98200b4f9212780a36d7bdd9f17bdd16ceeb430b49fc9284b146',
    'framing-content-length-different': '1132e0ab11fdef1d364c1812932c06622b51dd74f56e96f68355b6c5784aab8d',
    'framing-content-length-negative': 'f1353d5311baa4035656223feef96a91042bcddb77c21de303ac406c83f80c8b',
    'framing-content-length-plus': 'b44a321fffc23d84a99653e2b6c565e90e5f77d98be8375609be42456486f480',
    'framing-content-length-nondigit': 'ec61d7791e23d64f7dbbc258ca1c8d08db0f97aacba83fddd385baaae7ee734a',
    'framing-transfer-encoding-gzip': '9f93c81714e20abeacb6840a2f3d55ad306545cdd92f8b9fce1548e0050d0d62',
    'framing-transfer-encoding-nonfinal-chunked': '9136feb7b311bc879787753843ca42062989b019bc90ad530b9059d15474f3f9',
    'framing-transfer-encoding-duplicate-chunked': '00f12154b086a2cb36e1df4d2bc2745c6e1b8284c4c6e6d7a079a92c43255cee',
    'framing-invalid-chunk-size': '758e5e05a5b04268f92e2077c17107b3c85c80f223e846f107cf0ade5c104f84',
    'framing-invalid-chunk-terminator': '0fba9f0a023ca68517d030c57777a1e5718adcd651221d75cd713a2961bb1899',
    'framing-forbidden-trailer': '38b64bdd9f24c1fdbbe8860cc0af5102b8a90bc553d579ef94edd4a7fe985d83',
    'framing-truncated-content-length': '30e7291ba0d2e8c3964faa216ad565bc408059b18b9db27dcbc2dbd6ae28a7d3',
    'framing-truncated-chunk': '323a36d7f1ccac4f42fbba56221e8fc5c8559729f9df7bdf70312d5b7e3f68a9',
    'framing-transfer-length-keepalive': 'b81006121619b27d3791ddee2751bcd3ed58ab41683be87fbf14bb9620aac02f',
    'framing-conflicting-length-closes': 'a73ae3d4ca3b9413cb84f5644b2306deef4ddd3a2eacb8a5460d5236fed7fb9a',
    'framing-invalid-chunk-closes': 'cdd5eb1fbd17f2097bd65cc54360f7cd2e7d7abd4a7a96d6d748d7145ca19f29',
    'config-cert-only-plaintext': '9f9a10f9102c2a477695f4d8eee19eca58a02edfcd873b0cfc32e7196e32e339',
    'config-key-only-plaintext': '06a43d235e12cd6a36074f055308526cae94828a736f17cfea333f8f9b5a37b5',
    'config-relative-path-tls': 'ed08553de7b5aa4ad6bd8c76f0ab35fa174072f819792790b0bcbd5070b75a7c',
    'config-absolute-path-tls': '92144a14d9b690bad5b8104f133b719a8a45038eb3145cd5da9e9123eefabfea',
    'config-missing-pair-no-plaintext-fallback': '582d458d8882f38e5cee1c8d3c2019b2f8229a9be82ff6baf40900068e9d686c',
    'config-mismatched-pair-no-plaintext-fallback': '3c3b47ab2792328e76cf397d2c9a2346ea0cecf23c7849c426f0bc1de1a56c1b',
}
SOURCE_HASHES = {
    "go-source/core-config.go.gz": "85319ec1e7326d4fa67798e349d4617b03193cb79437645a1974961ae604e1e9",
    "go-source/core-http-json-handler.go.gz": "c103ae3b0cd030bee2fbe4820a4fbcbcee8e1b827d48e08f03db37de9c485bc2",
    "go-source/core-http-server.go.gz": "1375b043324e46dfd63af104bd3ae6321fe593f3d089664d56a30c5bfa80edd4",
    "go-source/core-node.go.gz": "e6c5d518c38074e25fe38334eb488e7b46df60a1fa5f5a25ea2034133b0d7b7a",
    "go-source/core-toml.go.gz": "673bf35dc6cf83b8317bef2e62aecc10b930c23fe0ccc2be7c0e6e1a2ed0e2bb",
    "go-source/core-ws-handler.go.gz": "7eadfa17765b6e330e0a058a8cd1850f24ee8dd4965b26a0fa3837bae0dff032",
    "go-source/cors.go.gz": "3a9bedebda398289143f73844e778dc6f0204264007e488a753099e6ef0ad918",
    "go-source/stdlib-crypto-tls-handshake-server.go.gz": "c89f0250b27fb1e8d6006b1cd1cdcd8ce568e4afd70558e98f54cf35161d9fbf",
    "go-source/stdlib-crypto-tls.go.gz": "234b2a093e4cee3c89b58bfcafcf3c88e4cae3ed2a79a8e618c9a1fd4c840977",
    "go-source/stdlib-net-http-chunked.go.gz": "edc08912a0fc3642079947b1db1fc480b7ce8b2c24712bf2545c888ea16b23ad",
    "go-source/stdlib-net-http-h2-bundle.go.gz": "4f50b6a8524cc1fc6aa70e55693af07bc1559d2a7bb94d5bb354b4e9a4f5779f",
    "go-source/stdlib-net-http-request.go.gz": "3521c55db8e25cbd085c75e0f85fc63b51ac6512fd41a299be6a431669c85227",
    "go-source/stdlib-net-http-server.go.gz": "6ec2a3107bc673fc0ab1f154e20df925f64698886be226f6e5b28b977756332f",
    "go-source/stdlib-net-http-transfer.go.gz": "4ab74cdb9a3310c0aa2800345b9a106923038262498c58d9501b05fea6de843c",
}


class VerificationError(ValueError):
    pass


def require(value: bool, message: str) -> None:
    if not value:
        raise VerificationError(message)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load(path: pathlib.Path) -> Any:
    return json.loads(path.read_text())


def parse_time(value: str) -> dt.datetime:
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(parsed.tzinfo is not None, f"timestamp lacks timezone: {value}")
    return parsed


def canonical_headers(headers: list[list[str]]) -> list[list[Any]]:
    grouped: dict[str, list[str]] = {}
    for name, value in headers:
        grouped.setdefault(name.lower(), []).append(value)
    return [[name, sorted(values)] for name, values in sorted(grouped.items())]


def verify_raw_records(value: Any, label: str) -> None:
    if isinstance(value, dict):
        if {"raw_base64", "raw_sha256", "raw_size"} <= value.keys():
            raw = base64.b64decode(value["raw_base64"], validate=True)
            require(len(raw) == value["raw_size"], f"{label}: raw size")
            require(sha(raw) == value["raw_sha256"], f"{label}: raw hash")
        if {"body_base64", "body_sha256"} <= value.keys():
            body = base64.b64decode(value["body_base64"], validate=True)
            require(sha(body) == value["body_sha256"], f"{label}: body hash")
            if "body_text" in value:
                require(body.decode() == value["body_text"], f"{label}: body text")
        if "headers" in value and "stable_headers" in value:
            stable = [pair for pair in value["headers"] if pair[0].lower() not in DYNAMIC_HEADERS]
            require(canonical_headers(stable) == canonical_headers(value["stable_headers"]), f"{label}: stable headers")
        for key, child in value.items():
            verify_raw_records(child, f"{label}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            verify_raw_records(child, f"{label}[{index}]")


def verify_manifest(root: pathlib.Path, provenance: dict[str, Any]) -> None:
    require(set(provenance["files"]) == set(provenance["published_files"]), "manifest inventory")
    paths = list(root.rglob("*"))
    require(not any(path.is_symlink() for path in paths), "published tree contains symlink")
    actual = {
        str(path.relative_to(root))
        for path in paths
        if path.is_file()
    }
    expected_actual = set(provenance["published_files"]) | {"provenance.json"}
    require(actual == expected_actual, "exact on-disk file inventory")
    for name, expected in provenance["files"].items():
        data = (root / name).read_bytes()
        require(len(data) == expected["size"], f"{name}: file size")
        require(sha(data) == expected["sha256"], f"{name}: file hash")
        if name.endswith(".gz"):
            raw = gzip.decompress(data)
            require(len(raw) == expected["uncompressed_size"], f"{name}: uncompressed size")
            require(sha(raw) == expected["uncompressed_sha256"], f"{name}: uncompressed hash")
    for name, expected in SOURCE_HASHES.items():
        require(provenance["files"][name]["sha256"] == expected, f"{name}: frozen source hash")


def verify_release(root: pathlib.Path, provenance: dict[str, Any], capture: dict[str, Any]) -> None:
    build = load(root / "oracle-build.json")
    require(build["module"] == "github.com/celestiaorg/celestia-app/v10@v10.5.0", "app module")
    require(build["source_commit"] == "6013e758d5dceb75827ad191c8d739d7c43cfa04", "app source")
    require(build["binary_sha256"] == "eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38", "oracle binary")
    require((root / "go-compiler.txt").read_text() == "go version go1.26.9 linux/amd64\n", "compiler")
    require((root / "oracle-module.txt").read_text() == "github.com/celestiaorg/celestia-app/v10@v10.5.0\n", "module receipt")
    build_info = gzip.decompress((root / "oracle-build-info.txt.gz").read_bytes()).decode()
    require("=>\tgithub.com/celestiaorg/celestia-core\tv0.42.3" in build_info, "embedded core")
    require("dep\tgithub.com/rs/cors\tv1.11.1" in build_info, "embedded CORS")
    require((root / "curl-binary.sha256").read_text() == "a353e3e5ca53a2b856257e7cf6e33f46ec0e6ced903846d69864d354d557cb15  /usr/bin/curl\n", "curl binary")
    require("HTTP2" in (root / "curl-version.txt").read_text(), "curl HTTP/2 support")

    observation = load(root / "mainnet-observation.json")
    require(observation["chain_id"] == "celestia" and observation["app_version"] == "10.5.0", "mainnet release")
    parsed = {}
    for route in ("status", "abci_info"):
        response = observation["responses"][route]
        require(sha(response["body"].encode()) == response["sha256"], f"mainnet {route} hash")
        parsed[route] = json.loads(response["body"])["result"]
    status = parsed["status"]
    abci = parsed["abci_info"]["response"]
    require(status["node_info"]["network"] == "celestia", "mainnet response chain")
    require(abci["version"] == "10.5.0", "mainnet response release")
    require(status["node_info"]["protocol_version"]["app"] == abci["app_version"] == "10", "mainnet protocol")
    require(status["sync_info"]["latest_block_height"] == abci["last_block_height"] == observation["height"], "mainnet height")
    delta = parse_time(capture["captured_at"]) - parse_time(observation["observed_at"])
    require(dt.timedelta(0) <= delta <= dt.timedelta(hours=24), "mainnet selection freshness/order")


def verify_identities(root: pathlib.Path, capture: dict[str, Any]) -> None:
    captured = parse_time(capture["captured_at"])
    for name, endpoint, network in (
        ("cert_only", "http://127.0.0.1:6340", "rs-proxy-cert-only-1"),
        ("relative", "https://localhost:6350", "rs-proxy-relative-1"),
    ):
        identity = load(root / f"{name}-identity.json")
        require(identity["endpoint"] == endpoint and identity["network"] == network, f"{name}: endpoint/network")
        require(identity["comet_version"] == "0.38.17" and identity["app_version"] == "10.5.0", f"{name}: release")
        require(identity["protocol_app"] == identity["abci_app_version"] == "10", f"{name}: protocol")
        require(identity["height"] == "1", f"{name}: height")
        for route in ("status", "abci_info"):
            response = identity["responses"][route]
            require(sha(response["body"].encode()) == response["sha256"], f"{name}: {route} hash")
        observed = parse_time(identity["observed_at"])
        require(dt.timedelta(0) <= observed - captured <= dt.timedelta(minutes=10), f"{name}: identity order")


def verify_configs(root: pathlib.Path, cases: dict[str, dict[str, Any]]) -> None:
    values = {
        "cert-only": ("ignored.crt", ""),
        "key-only": ("", "ignored.key"),
        "relative": ("localhost.crt", "localhost.key"),
        "absolute": (
            "/workspace/scratch/rpc-181/proxy-tls/tls/localhost.crt",
            "/workspace/scratch/rpc-181/proxy-tls/tls/localhost.key",
        ),
        "missing": (
            "/workspace/scratch/rpc-181/proxy-tls/tls/missing.crt",
            "/workspace/scratch/rpc-181/proxy-tls/tls/missing.key",
        ),
        "mismatch": (
            "/workspace/scratch/rpc-181/proxy-tls/tls/localhost.crt",
            "/workspace/scratch/rpc-181/proxy-tls/tls/mismatch.key",
        ),
    }
    case_names = {
        "cert-only": "config-cert-only-plaintext",
        "key-only": "config-key-only-plaintext",
        "relative": "config-relative-path-tls",
        "absolute": "config-absolute-path-tls",
        "missing": "config-missing-pair-no-plaintext-fallback",
        "mismatch": "config-mismatched-pair-no-plaintext-fallback",
    }
    for run, expected in values.items():
        raw = gzip.decompress((root / f"{run}-config.toml.gz").read_bytes())
        text = raw.decode()
        cert = re.search(r'(?m)^tls_cert_file\s*=\s*"([^"]*)"$', text)
        key = re.search(r'(?m)^tls_key_file\s*=\s*"([^"]*)"$', text)
        require(cert is not None and key is not None and (cert.group(1), key.group(1)) == expected, f"{run}: TLS config")
        require(sha(raw) == cases[case_names[run]]["config_sha256"], f"{run}: config binding")


def verify_semantics(root: pathlib.Path, capture: dict[str, Any]) -> None:
    campaign = load(root / "campaign.json")
    expected_names = [name for names in campaign["categories"].values() for name in names]
    require([case["name"] for case in capture["cases"]] == expected_names, "exact case inventory/order")
    require(len(expected_names) == len(set(expected_names)) == 50, "50 unique cases")
    require(capture["dynamic_response_headers_excluded_from_stable_projection"] == DYNAMIC_HEADERS, "dynamic exclusions")
    cases = {case["name"]: case for case in capture["cases"]}
    require(set(cases) == set(CASE_DIGESTS), "frozen case digest inventory")
    for name, case in cases.items():
        digest = sha(json.dumps(case, sort_keys=True, separators=(",", ":")).encode())
        require(digest == CASE_DIGESTS[name], f"{name}: exact frozen case")
        verify_raw_records(case, name)

    h1, h2 = cases["tls-health-http1"]["response"], cases["tls-health-http2"]["response"]
    require(h1["http_version"] == "HTTP/1.1" and h2["http_version"] == "HTTP/2", "real H1/H2")
    require(h1["status"] == h2["status"] == 200 and h1["body_text"] == h2["body_text"], "H1/H2 health")
    require(canonical_headers(h1["stable_headers"]) == canonical_headers(h2["stable_headers"]), "H1/H2 headers")
    wss = cases["tls-wss-jsonrpc-clean-close"]
    require(wss["handshake_response"]["status"] == 101, "WSS handshake")
    require(wss["response_frame"]["fin"] and wss["response_frame"]["opcode"] == 1, "WSS text frame")
    require(json.loads(wss["response_frame"]["payload_text"]) == {"jsonrpc": "2.0", "id": 1, "result": {}}, "WSS health")
    require(wss["close_response"]["fin"] and wss["close_response"]["opcode"] == 8 and wss["close_response"]["code"] == 1000, "WSS close")
    require(wss["outcome"] == "clean-eof", "WSS EOF")
    rest = cases["tls-node-rest-get-latest-block"]["response"]
    require(rest["status"] == 200 and json.loads(rest["body_text"])["block"]["header"]["height"] == "1", "TLS node SDK query")

    require(cases["proxy-root-internal-host"]["response"]["body_text"] == cases["proxy-forwarded-headers-inert"]["response"]["body_text"], "forwarded headers inert")
    require(cases["proxy-root-public-host"]["response"]["body_text"] == cases["proxy-absolute-form-target"]["response"]["body_text"], "absolute-form host")
    require("//backend.internal/" in cases["proxy-root-internal-host"]["response"]["body_text"], "backend Host")
    require("//rpc.provider.example/" in cases["proxy-root-public-host"]["response"]["body_text"], "public Host")
    require(cases["plaintext-on-tls-port"]["response"]["body_text"] == "Client sent an HTTP request to an HTTPS server.\n", "plaintext-to-TLS diagnostic")
    require(cases["tls-certificate-name-mismatch"] == {"name": "tls-certificate-name-mismatch", "category": "tls", "transport": "tls", "outcome": "certificate-name-mismatch", "verify_code": 62}, "certificate name rejection")

    require(len(cases["framing-transfer-length-keepalive"]["responses"]) == 2, "TE+CL keepalive")
    require(len(cases["framing-conflicting-length-closes"]["responses"]) == 1, "conflicting CL close")
    require(len(cases["framing-invalid-chunk-closes"]["responses"]) == 1, "invalid chunk close")
    expected_transports = ["plain", "plain", "tls", "tls", "unavailable", "unavailable"]
    for name, transport in zip(campaign["categories"]["tls-config"], expected_transports, strict=True):
        require(cases[name]["expected_transport"] == transport, f"{name}: config outcome")
    require(cases["config-missing-pair-no-plaintext-fallback"]["tls_start_error"].endswith("missing.crt: no such file or directory"), "missing pair error")
    require(cases["config-mismatched-pair-no-plaintext-fallback"]["tls_start_error"] == "tls: private key does not match public key", "mismatched pair error")
    verify_configs(root, cases)

    cert_text = (root / "tls/localhost-cert.txt").read_text()
    require("DNS:localhost" in cert_text and "IP Address:127.0.0.1" in cert_text, "certificate SANs")
    require("Not After : Oct  7 03:21:32 2126 GMT" in cert_text, "long-lived certificate")
    require("CA:FALSE" in cert_text, "certificate is an end-entity fixture")
    require("conformance-only" in (root / "README.md").read_text(), "fixture key label")


def verify(root: pathlib.Path = HERE) -> None:
    provenance = load(root / "provenance.json")
    capture = load(root / "proxy-tls-framing.json")
    verify_manifest(root, provenance)
    verify_release(root, provenance, capture)
    verify_identities(root, capture)
    verify_semantics(root, capture)


if __name__ == "__main__":
    verify()
    print("verified 50 proxy/TLS/framing cases and exact provenance")
