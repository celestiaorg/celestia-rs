#!/usr/bin/env python3
"""Build the exact published file manifest after capture and verifier freeze."""

import gzip
import hashlib
import json
import pathlib


HERE = pathlib.Path(__file__).parent
FILES = [
    "README.md", "campaign.json", "capture-source.py", "test_capture_source.py",
    "config-probe-source.py", "aggregate-source.py", "observe-local-source.py",
    "observe-mainnet.py", "observe-mainnet.sh", "run-source.sh", "start-source.sh",
    "make-cert.sh", "copy-source.sh", "build-provenance.py", "verify.py", "test_verify.py",
    "mainnet-observation.json", "proxy-tls-framing.json", "cert_only-identity.json",
    "relative-identity.json", "oracle-build.json", "oracle-build-info.txt.gz",
    "oracle-module.txt", "go-compiler.txt", "curl-version.txt", "curl-binary.sha256",
    "tls/openssl.cnf", "tls/localhost.crt", "tls/localhost.key", "tls/mismatch.key",
    "tls/localhost-cert.txt", "tls/openssl-version.txt",
    *[f"{name}-{kind}" for name in ("cert-only", "key-only", "relative", "absolute", "missing", "mismatch")
      for kind in ("config.toml.gz", "app.toml.gz", "genesis.json.gz")],
    *sorted(str(path.relative_to(HERE)) for path in (HERE / "go-source").glob("*.gz")),
]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


manifest = {}
for name in FILES:
    data = (HERE / name).read_bytes()
    entry = {"sha256": sha(data), "size": len(data)}
    if name.endswith(".gz"):
        raw = gzip.decompress(data)
        entry |= {"uncompressed_sha256": sha(raw), "uncompressed_size": len(raw)}
    manifest[name] = entry

result = {
    "schema_version": 1,
    "tracking_issues": [191, 181],
    "release_selection": {
        "network": "celestia",
        "app_release": "10.5.0",
        "observation": "mainnet-observation.json",
        "rule": "Read-only status and abci_info agreed on chain, height, protocol app and active app release before capture; this selects the oracle and does not claim provider parity."
    },
    "oracle": {
        "module": "github.com/celestiaorg/celestia-app/v10@v10.5.0",
        "source_commit": "6013e758d5dceb75827ad191c8d739d7c43cfa04",
        "core": "github.com/celestiaorg/celestia-core@v0.42.3",
        "compiler": "go version go1.26.9 linux/amd64",
        "binary_sha256": "eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38",
    },
    "dynamic_response_headers_excluded_from_stable_projection": [
        "connection", "content-length", "date", "transfer-encoding", "x-server-time"
    ],
    "semantic_header_comparison": "lowercase name to exact duplicate-preserving value multiset; raw order and casing remain hash-bound integrity evidence",
    "published_exclusions": [
        "node homes and databases", "validator and node private keys", "application keyrings",
        "ordinary node logs", "the 184 MiB oracle binary"
    ],
    "published_files": FILES,
    "files": manifest,
}
(HERE / "provenance.json").write_text(json.dumps(result, indent=2) + "\n")
