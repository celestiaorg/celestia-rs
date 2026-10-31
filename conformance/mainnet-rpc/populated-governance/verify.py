#!/usr/bin/env python3
"""Offline integrity and semantic verifier for the populated governance slice."""
import base64
from datetime import datetime, timedelta, timezone
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
from urllib.parse import parse_qs, urlsplit


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "conformance/fixtures/devnet/v105-populated-governance"
CAPTURE_COMMIT = "7627127a36b87eb7ce3770c88eda7de866ecff56"
EXPECTED_CASES = (
    "proposal-voting", "proposal-deposit", "proposal-expedited",
    "proposal-historical", "proposals-first", "proposals-next-key",
    "proposals-offset", "proposals-reverse", "proposals-empty-page",
    "proposals-voting", "proposals-deposit", "proposals-voter",
    "proposals-depositor", "proposals-historical", "vote-weighted",
    "votes-first", "votes-next-key", "votes-reverse", "deposit-secondary",
    "deposits-first", "deposits-next-key", "deposits-reverse",
    "tally-voting", "params-voting", "constitution",
)
EXPECTED_OMISSIONS = [
    "proof validity is not asserted by this transport capture",
    "governance proposal completion, cancellation and expiry remain separate captures",
    "failed historical governance queries remain a separate capture",
    "historical pagination key and offset traversal remain a separate capture",
    "invalid pagination cursors and malformed/error requests remain separate captures",
    "governance v1beta1 compatibility remains a separate capture",
    "non-governance module workloads remain separate captures",
    "invalid UTF-8 remains a separate capture",
]
VAL0_ADDRESS = "celestia19rl4cm2hmr8afy4kldpxz3fka4jguq0ad2ud9c"
USER0_ADDRESS = "celestia1avgyh77ycn997ja45q5q8ss8y9mr424j3snr0v"
GOV_AUTHORITY = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
METHODS = {
    **{name: "/cosmos.gov.v1.Query/Proposal" for name in (
        "proposal-voting", "proposal-deposit", "proposal-expedited",
        "proposal-historical",
    )},
    **{name: "/cosmos.gov.v1.Query/Proposals" for name in (
        "proposals-first", "proposals-next-key", "proposals-offset",
        "proposals-reverse", "proposals-empty-page", "proposals-voting",
        "proposals-deposit", "proposals-voter", "proposals-depositor",
        "proposals-historical",
    )},
    "vote-weighted": "/cosmos.gov.v1.Query/Vote",
    **{name: "/cosmos.gov.v1.Query/Votes" for name in (
        "votes-first", "votes-next-key", "votes-reverse",
    )},
    "deposit-secondary": "/cosmos.gov.v1.Query/Deposit",
    **{name: "/cosmos.gov.v1.Query/Deposits" for name in (
        "deposits-first", "deposits-next-key", "deposits-reverse",
    )},
    "tally-voting": "/cosmos.gov.v1.Query/TallyResult",
    "params-voting": "/cosmos.gov.v1.Query/Params",
    "constitution": "/cosmos.gov.v1.Query/Constitution",
}
REST_PATHS = {
    "proposal-voting": "/cosmos/gov/v1/proposals/1",
    "proposal-deposit": "/cosmos/gov/v1/proposals/2",
    "proposal-expedited": "/cosmos/gov/v1/proposals/3",
    "proposal-historical": "/cosmos/gov/v1/proposals/1",
    "proposals-first": "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.count_total=true",
    "proposals-offset": "/cosmos/gov/v1/proposals?pagination.offset=1&pagination.limit=1&pagination.count_total=true",
    "proposals-reverse": "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.reverse=true",
    "proposals-empty-page": "/cosmos/gov/v1/proposals?pagination.offset=999&pagination.limit=1",
    "proposals-voting": "/cosmos/gov/v1/proposals?proposal_status=2",
    "proposals-deposit": "/cosmos/gov/v1/proposals?proposal_status=1",
    "proposals-voter": f"/cosmos/gov/v1/proposals?voter={VAL0_ADDRESS}",
    "proposals-depositor": f"/cosmos/gov/v1/proposals?depositor={USER0_ADDRESS}",
    "proposals-historical": "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.count_total=true",
    "vote-weighted": f"/cosmos/gov/v1/proposals/1/votes/{USER0_ADDRESS}",
    "votes-first": "/cosmos/gov/v1/proposals/1/votes?pagination.limit=1&pagination.count_total=true",
    "votes-reverse": "/cosmos/gov/v1/proposals/1/votes?pagination.limit=1&pagination.reverse=true",
    "deposit-secondary": f"/cosmos/gov/v1/proposals/2/deposits/{USER0_ADDRESS}",
    "deposits-first": "/cosmos/gov/v1/proposals/2/deposits?pagination.limit=1&pagination.count_total=true",
    "deposits-reverse": "/cosmos/gov/v1/proposals/2/deposits?pagination.limit=1&pagination.reverse=true",
    "tally-voting": "/cosmos/gov/v1/proposals/1/tally",
    "params-voting": "/cosmos/gov/v1/params/voting",
    "constitution": "/cosmos/gov/v1/constitution",
}
EXPECTED_SOURCES = {
    "conformance/devnet/devnet.sh",
    "conformance/devnet/build-bins.sh",
    "conformance/devnet/workloads/lib.sh",
    "conformance/devnet/workloads/wait_tx.py",
    "conformance/devnet/workloads/populated-governance.sh",
    "conformance/devnet/workloads/populated-governance.genesis.py",
    "conformance/mainnet-rpc/populated-governance/README.md",
    "conformance/mainnet-rpc/populated-governance/capture.sh",
    "conformance/mainnet-rpc/populated-governance/record.sh",
    "conformance/mainnet-rpc/populated-governance/verify.py",
    "conformance/mainnet-rpc/observe-mainnet.py",
    "conformance/mainnet-rpc/populated-governance/devnet-keys/node_key.json",
    "conformance/mainnet-rpc/populated-governance/devnet-keys/priv_validator_key.json",
    "conformance/rpccorpus/recorder/cmd/populated-governance/main.go",
    "conformance/rpccorpus/recorder/cmd/populated-governance/main_test.go",
    "conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go",
    "conformance/rpccorpus/recorder/main.go",
    "conformance/rpccorpus/recorder/record.go",
    "conformance/rpccorpus/recorder/validation.go",
    "conformance/rpccorpus/recorder/go.mod",
    "conformance/rpccorpus/recorder/go.sum",
    "crates/devnet-conformance/tests/populated_governance_transports.rs",
    "Cargo.toml",
    "Cargo.lock",
    "crates/devnet-conformance/Cargo.toml",
    "crates/tier-4-runtime/api-server/Cargo.toml",
    "crates/tier-4-runtime/api-server/src/rest.rs",
    "conformance/devnet-oracle/main.go",
    "conformance/devnet-oracle/go.mod",
    "conformance/devnet-oracle/go.sum",
}
EXPECTED_ARTIFACTS = {
    "genesis.json", "abci.rec", "blocks.bin.gz", "meta.json",
    "capture/transports.json", "capture/mainnet-observation.json",
    "capture/app-build.json", "capture/core-build.json",
    "capture/app-build-info.txt", "capture/core-build-info.txt",
    "go-build-info.txt", "capture/abci-recorder-build-info.txt",
    "capture/populated-governance-build-info.txt",
    "capture/rpc-recorder-build-info.txt",
    "capture/devnet-oracle-build-info.txt", "state/1.json", "state/35.json",
}
EXPECTED_GENERATORS = {
    "abci-recorder": {
        "module": "github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder",
        "build_info": "capture/abci-recorder-build-info.txt",
        "sources": {
            "conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go",
            "conformance/rpccorpus/recorder/go.mod",
            "conformance/rpccorpus/recorder/go.sum",
        },
    },
    "populated-governance": {
        "module": "github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder",
        "build_info": "capture/populated-governance-build-info.txt",
        "sources": {
            "conformance/rpccorpus/recorder/cmd/populated-governance/main.go",
            "conformance/rpccorpus/recorder/go.mod",
            "conformance/rpccorpus/recorder/go.sum",
        },
    },
    "rpc-recorder": {
        "module": "github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder",
        "build_info": "capture/rpc-recorder-build-info.txt",
        "sources": {
            "conformance/rpccorpus/recorder/main.go",
            "conformance/rpccorpus/recorder/record.go",
            "conformance/rpccorpus/recorder/validation.go",
            "conformance/rpccorpus/recorder/go.mod",
            "conformance/rpccorpus/recorder/go.sum",
        },
    },
    "devnet-oracle": {
        "module": "github.com/celestiaorg/celestia-rs/conformance/devnet-oracle",
        "build_info": "capture/devnet-oracle-build-info.txt",
        "sources": {
            "conformance/devnet-oracle/main.go",
            "conformance/devnet-oracle/go.mod",
            "conformance/devnet-oracle/go.sum",
        },
    },
}


def check(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def captured_source_digest(name):
    """Hash a replay source from the captured tree, not the moving worktree."""
    try:
        completed = subprocess.run(
            ["git", "show", f"{CAPTURE_COMMIT}:{name}"],
            cwd=ROOT,
            check=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise ValueError(f"captured source unavailable: {name}") from error
    return hashlib.sha256(completed.stdout).hexdigest()


def read_varint(data, offset):
    value = shift = 0
    while offset < len(data) and shift < 70:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if byte < 0x80:
            return value, offset
        shift += 7
    raise ValueError("invalid protobuf varint")


def proto_fields(data):
    fields = []
    offset = 0
    while offset < len(data):
        tag, offset = read_varint(data, offset)
        field, wire = tag >> 3, tag & 7
        check(field != 0, "protobuf field zero")
        if wire == 0:
            value, offset = read_varint(data, offset)
        elif wire == 2:
            size, offset = read_varint(data, offset)
            check(size <= len(data) - offset, "truncated protobuf field")
            value = data[offset:offset + size]
            offset += size
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        fields.append((field, wire, value))
    return fields


def exact_message(data, allowed):
    result = {field: [] for field in allowed}
    for field, wire, value in proto_fields(data):
        check(field in allowed, f"unexpected protobuf field {field}")
        check(wire == allowed[field], f"field {field} has wire type {wire}")
        result[field].append(value)
    return result


def one(values, label):
    check(len(values) == 1, f"{label} occurs {len(values)} times")
    return values[0]


def optional(values, default):
    check(len(values) <= 1, "optional protobuf field is repeated")
    return values[0] if values else default


def page_shape(data):
    fields = exact_message(data, {1: 2, 2: 0, 3: 0, 4: 0, 5: 0})
    return {
        "key": optional(fields[1], b""),
        "offset": optional(fields[2], 0),
        "limit": optional(fields[3], 0),
        "total": optional(fields[4], 0),
        "reverse": optional(fields[5], 0),
    }


def verify_next_key_path(case, path, prefix, key):
    parsed = urlsplit(path)
    check(parsed.path == prefix, f"next-key REST path mismatch: {case}")
    query = parse_qs(parsed.query, keep_blank_values=True)
    check(set(query) == {"pagination.key", "pagination.limit"} and
          query["pagination.limit"] == ["1"] and
          len(query["pagination.key"]) == 1,
          f"next-key REST query mismatch: {case}")
    try:
        rest_key = base64.b64decode(query["pagination.key"][0], validate=True)
    except ValueError as error:
        raise ValueError(f"invalid next-key REST base64: {case}") from error
    check(rest_key == key, f"REST/protobuf next key mismatch: {case}")


def verify_request(case):
    name = case["name"]
    data = bytes.fromhex(case["request_hex"])
    check(case["method"] == METHODS[name], f"query method mismatch: {name}")
    if name in REST_PATHS:
        check(case["rest_path"] == REST_PATHS[name],
              f"REST path mismatch: {name}")

    if name.startswith("proposal-"):
        expected = {"proposal-voting": 1, "proposal-deposit": 2,
                    "proposal-expedited": 3, "proposal-historical": 1}[name]
        request = exact_message(data, {1: 0})
        check(one(request[1], f"{name}.proposal_id") == expected,
              f"proposal request mismatch: {name}")
        return
    if name.startswith("proposals-"):
        request = exact_message(data, {1: 0, 2: 2, 3: 2, 4: 2})
        status = optional(request[1], 0)
        voter = optional(request[2], b"").decode()
        depositor = optional(request[3], b"").decode()
        pagination = page_shape(optional(request[4], b""))
        expected = {
            "proposals-first": (0, "", "", {"key": b"", "offset": 0, "limit": 1, "total": 1, "reverse": 0}),
            "proposals-offset": (0, "", "", {"key": b"", "offset": 1, "limit": 1, "total": 1, "reverse": 0}),
            "proposals-reverse": (0, "", "", {"key": b"", "offset": 0, "limit": 1, "total": 0, "reverse": 1}),
            "proposals-empty-page": (0, "", "", {"key": b"", "offset": 999, "limit": 1, "total": 0, "reverse": 0}),
            "proposals-voting": (2, "", "", {"key": b"", "offset": 0, "limit": 0, "total": 0, "reverse": 0}),
            "proposals-deposit": (1, "", "", {"key": b"", "offset": 0, "limit": 0, "total": 0, "reverse": 0}),
            "proposals-voter": (0, VAL0_ADDRESS, "", {"key": b"", "offset": 0, "limit": 0, "total": 0, "reverse": 0}),
            "proposals-depositor": (0, "", USER0_ADDRESS, {"key": b"", "offset": 0, "limit": 0, "total": 0, "reverse": 0}),
            "proposals-historical": (0, "", "", {"key": b"", "offset": 0, "limit": 1, "total": 1, "reverse": 0}),
        }
        if name == "proposals-next-key":
            check(status == 0 and not voter and not depositor and
                  pagination["key"] and pagination["offset"] == 0 and
                  pagination["limit"] == 1 and pagination["total"] == 0 and
                  pagination["reverse"] == 0,
                  "proposal next-key request mismatch")
            verify_next_key_path(name, case["rest_path"],
                                 "/cosmos/gov/v1/proposals", pagination["key"])
        else:
            check((status, voter, depositor, pagination) == expected[name],
                  f"proposal list request mismatch: {name}")
        return
    if name == "vote-weighted":
        request = exact_message(data, {1: 0, 2: 2})
        check(one(request[1], "vote.proposal_id") == 1 and
              one(request[2], "vote.voter").decode() == USER0_ADDRESS,
              "vote request mismatch")
        return
    if name.startswith("votes-") or name.startswith("deposits-"):
        request = exact_message(data, {1: 0, 2: 2})
        expected_id = 1 if name.startswith("votes-") else 2
        check(one(request[1], f"{name}.proposal_id") == expected_id,
              f"page proposal mismatch: {name}")
        pagination = page_shape(one(request[2], f"{name}.pagination"))
        if name.endswith("next-key"):
            check(pagination["key"] and pagination["offset"] == 0 and
                  pagination["limit"] == 1 and pagination["total"] == 0 and
                  pagination["reverse"] == 0,
                  f"next-key request mismatch: {name}")
            resource = "votes" if name.startswith("votes-") else "deposits"
            verify_next_key_path(
                name, case["rest_path"],
                f"/cosmos/gov/v1/proposals/{expected_id}/{resource}",
                pagination["key"],
            )
        else:
            expected_page = ({"key": b"", "offset": 0, "limit": 1,
                              "total": 1, "reverse": 0}
                             if name.endswith("first") else
                             {"key": b"", "offset": 0, "limit": 1,
                              "total": 0, "reverse": 1})
            check(pagination == expected_page, f"page request mismatch: {name}")
        return
    if name == "deposit-secondary":
        request = exact_message(data, {1: 0, 2: 2})
        check(one(request[1], "deposit.proposal_id") == 2 and
              one(request[2], "deposit.depositor").decode() == USER0_ADDRESS,
              "deposit request mismatch")
        return
    if name == "tally-voting":
        request = exact_message(data, {1: 0})
        check(one(request[1], "tally.proposal_id") == 1,
              "tally request mismatch")
        return
    if name == "params-voting":
        request = exact_message(data, {1: 2})
        check(one(request[1], "params.type") == b"voting",
              "params request mismatch")
        return
    check(name == "constitution" and data == b"", "constitution request mismatch")


def archive_frames(path):
    data = gzip.decompress(path.read_bytes())
    frames = []
    offset = 0
    while offset < len(data):
        size, offset = read_varint(data, offset)
        check(size != 0 and size <= len(data) - offset,
              "invalid block archive frame")
        frames.append(data[offset:offset + size])
        offset += size
    return frames


def block_header(data):
    block = exact_message(data, {1: 2, 2: 2, 3: 2, 4: 2})
    header = exact_message(one(block[1], "Block.header"), {
        1: 2, 2: 2, 3: 0, 4: 2, 5: 2, 6: 2, 7: 2,
        8: 2, 9: 2, 10: 2, 11: 2, 12: 2, 13: 2, 14: 2,
    })
    return {
        "chain_id": one(header[2], "Header.chain_id").decode(),
        "height": one(header[3], "Header.height"),
        "app_hash": one(header[11], "Header.app_hash").hex() if header[11] else "",
    }


def commit_height(data):
    commit = exact_message(data, {1: 0, 2: 0, 3: 2, 4: 2})
    return one(commit[1], "Commit.height")


def response_bytes(case, transport):
    value = case["transports"][transport].get("response_hex", "")
    return bytes.fromhex(value)


def rest_body(cases, name):
    return cases[name]["transports"]["rest"]["body"]


def proposal_wire(case):
    """Decode fields needed to bind grpc-gateway Any JSON to protobuf bytes."""
    response = exact_message(response_bytes(case, "cosmos_grpc"), {1: 2})
    proposal = exact_message(one(response[1], "QueryProposalResponse.proposal"), {
        1: 0, 2: 2, 3: 0, 4: 2, 5: 2, 6: 2, 7: 2, 8: 2,
        9: 2, 10: 2, 11: 2, 12: 2, 13: 2, 14: 0, 15: 2,
    })
    any_message = exact_message(one(proposal[2], "Proposal.messages"), {1: 2, 2: 2})
    blob_update = exact_message(one(any_message[2], "Any.value"), {1: 2, 2: 2})
    params = exact_message(one(blob_update[2], "MsgUpdateBlobParams.params"),
                           {1: 0, 2: 0})
    return {
        "id": one(proposal[1], "Proposal.id"),
        "status": one(proposal[3], "Proposal.status"),
        "metadata": one(proposal[10], "Proposal.metadata").decode(),
        "title": one(proposal[11], "Proposal.title").decode(),
        "summary": one(proposal[12], "Proposal.summary").decode(),
        "proposer": one(proposal[13], "Proposal.proposer").decode(),
        "expedited": one(proposal[14], "Proposal.expedited")
        if proposal[14] else 0,
        "type_url": one(any_message[1], "Any.type_url").decode(),
        "authority": one(blob_update[1], "MsgUpdateBlobParams.authority").decode(),
        "gas_per_blob_byte": one(params[1], "Params.gas_per_blob_byte"),
        "gov_max_square_size": one(params[2], "Params.gov_max_square_size"),
    }


def verify_hashes(metadata):
    check(set(metadata) == {
        "schema_version", "captured_at", "method", "state_identity",
        "source_sha256", "artifact_sha256", "generator_binaries",
        "omissions", "regenerate",
    }, "unexpected metadata fields")
    check(metadata["schema_version"] == 1, "unexpected metadata schema")
    check(metadata["method"] ==
          "fresh tagged v10.5.0 governance workload and live four-transport capture on one isolated devnet",
          "unexpected capture method")
    check(metadata["regenerate"] ==
          "MAINNET_OBSERVATION=<fresh observation.json> conformance/mainnet-rpc/populated-governance/record.sh",
          "unexpected regeneration command")
    check(set(metadata["source_sha256"]) == EXPECTED_SOURCES,
          "unexpected source inventory")
    check(set(metadata["artifact_sha256"]) == EXPECTED_ARTIFACTS,
          "unexpected artifact inventory")
    published = set()
    for path in FIXTURE.rglob("*"):
        check(not path.is_symlink(),
              f"fixture contains symlink: {path.relative_to(FIXTURE)}")
        if path.is_file():
            published.add(path.relative_to(FIXTURE).as_posix())
    check(published == EXPECTED_ARTIFACTS | {"capture/metadata.json"},
          "fixture directory contains an unexpected or missing file")
    for name, expected in metadata["source_sha256"].items():
        check(captured_source_digest(name) == expected, f"source hash mismatch: {name}")
    for name, expected in metadata["artifact_sha256"].items():
        check(digest(FIXTURE / name) == expected, f"artifact hash mismatch: {name}")

    app = json.loads((FIXTURE / "capture/app-build.json").read_text())
    core = json.loads((FIXTURE / "capture/core-build.json").read_text())
    check(app["module"] == "github.com/celestiaorg/celestia-app/v10@v10.5.0",
          "unexpected app module")
    check(app["source_commit"] == "6013e758d5dceb75827ad191c8d739d7c43cfa04",
          "unexpected app source commit")
    check(app["module_sum"] == "h1:CEKeiFSFRclu85ozJo5Gdk71H9am3wyEJJ8pMWqBNkc=",
          "unexpected app module sum")
    check(app["build_tags"] == ["ledger"], "unexpected app build tags")
    check(core["module"] == "github.com/celestiaorg/celestia-core@v0.42.3",
          "unexpected core module")
    check(core["source_commit"] == "a5537d84a008d566abc503d2ea41e30f2301403d",
          "unexpected core source commit")
    check(core["module_sum"] == "h1:kdXKlSiEqEWl87EyxNB9ryx1aQJq/8RyBdgfYy2rdYM=",
          "unexpected core module sum")
    check(app["buildvcs"] is False and core["buildvcs"] is False,
          "unexpected VCS build stamping")
    check(app["compiler"] == core["compiler"] ==
          "go version go1.26.9 linux/amd64", "unexpected Go compiler")
    check(app["target"]["GOOS"] == core["target"]["GOOS"] == "linux" and
          app["target"]["GOARCH"] == core["target"]["GOARCH"] == "amd64",
          "unexpected build target")

    check(set(metadata["generator_binaries"]) == set(EXPECTED_GENERATORS),
          "unexpected generator inventory")
    for name, expected in EXPECTED_GENERATORS.items():
        generator = metadata["generator_binaries"][name]
        check(set(generator) == {
            "sha256", "module", "sources", "build_info", "build_info_sha256"
        }, f"unexpected {name} generator fields")
        check(len(generator["sha256"]) == 64, f"bad {name} binary hash")
        check(generator["module"] == expected["module"],
              f"unexpected {name} module")
        check(generator["build_info"] == expected["build_info"],
              f"unexpected {name} build receipt path")
        check(set(generator["sources"]) == expected["sources"],
              f"unexpected {name} generator sources")
        for path in generator["sources"]:
            check(digest(ROOT / path) == metadata["source_sha256"][path],
                  f"unbound generator source: {path}")
        build_info = FIXTURE / generator["build_info"]
        check(digest(build_info) == generator["build_info_sha256"],
              f"bad {name} build receipt")
        contents = build_info.read_text()
        check("go1.26.9" in contents, f"unexpected {name} Go toolchain")
        check(generator["module"] in contents, f"missing {name} module receipt")


def verify_observation(metadata):
    observation = json.loads(
        (FIXTURE / "capture/mainnet-observation.json").read_text())
    check((observation["chain_id"], observation["version"]) ==
          ("celestia", "10.5.0"), "unexpected observed release")
    endpoint = urlsplit(observation["endpoint"])
    check(endpoint.scheme == "https" and endpoint.hostname and
          endpoint.username is None and endpoint.password is None and
          not endpoint.query and not endpoint.fragment,
          "mainnet observation endpoint is not public HTTPS")
    parsed = {}
    for name in ("status", "abci_info"):
        raw = observation["responses"][name]["body"].encode()
        check(hashlib.sha256(raw).hexdigest() ==
              observation["responses"][name]["sha256"],
              f"observation raw hash mismatch: {name}")
        parsed[name] = json.loads(raw)
    status = parsed["status"]["result"]
    info = parsed["abci_info"]["result"]["response"]
    check(status["node_info"]["network"] == observation["chain_id"],
          "observed chain disagreement")
    check(info["version"] == observation["version"],
          "observed version disagreement")
    check(status["node_info"]["protocol_version"]["app"] == info["app_version"],
          "observed app protocol disagreement")
    check(info["app_version"] == "10", "unexpected observed app protocol")
    check(status["sync_info"]["catching_up"] is False,
          "mainnet observation node is catching up")
    check(status["sync_info"]["latest_block_height"] == info["last_block_height"],
          "observed height disagreement")
    observed = datetime.fromisoformat(observation["observed_at"])
    captured = datetime.fromisoformat(metadata["captured_at"])
    now = datetime.now(timezone.utc)
    check(observed.utcoffset() is not None and captured.utcoffset() is not None,
          "naive evidence timestamp")
    check(observed <= captured, "observation does not precede capture")
    check(captured - observed <= timedelta(hours=24), "stale mainnet observation")
    check(captured <= now + timedelta(minutes=5), "future capture timestamp")


def verify_transports(capture, metadata, cases):
    check(set(capture) == {
        "schema_version", "chain_id", "tip_height", "query_height",
        "app_hash", "state", "cases", "normalizations", "omissions",
    }, "unexpected transport capture fields")
    check(capture["schema_version"] == 1, "unexpected transport schema")
    check(capture["chain_id"] == "rs-devnet-1", "unexpected devnet chain")
    check(capture["tip_height"] == 35 and capture["query_height"] == 34,
          "unexpected capture heights")
    check(set(cases) == set(EXPECTED_CASES),
          "unexpected governance case inventory")
    check(capture["normalizations"] == [], "unexpected normalization")
    check(capture["omissions"] == EXPECTED_OMISSIONS, "unexpected capture omissions")
    check(metadata["omissions"] == EXPECTED_OMISSIONS, "unexpected metadata omissions")
    check(metadata["state_identity"] == {
        name: capture[name] for name in
        ("chain_id", "tip_height", "query_height", "app_hash")
    }, "metadata state identity mismatch")
    block = capture["state"]["committing_block"]["result"]["block"]["header"]
    check(block["height"] == "35" and block["chain_id"] == capture["chain_id"],
          "committing block identity mismatch")
    check(block["app_hash"] == capture["app_hash"], "committing root mismatch")

    for name, case in cases.items():
        check(set(case) == {
            "name", "method", "request_hex", "height", "rest_path",
            "transports",
        }, f"unexpected case fields: {name}")
        check(len(case["request_hex"]) % 2 == 0,
              f"odd request hex length: {name}")
        bytes.fromhex(case["request_hex"])
        verify_request(case)
        transports = case["transports"]
        check(set(transports) == {"abci", "comet_rpc", "cosmos_grpc", "rest"},
              f"missing transport: {name}")
        grpc = response_bytes(case, "cosmos_grpc")
        abci = response_bytes(case, "abci")
        abci_body = transports["abci"]["body"]
        comet_transport = transports["comet_rpc"]
        comet_body = comet_transport["body"]
        check(comet_transport.get("status") == 200,
              f"Comet HTTP status mismatch: {name}")
        check(set(comet_body) == {"jsonrpc", "id", "result"} and
              comet_body["jsonrpc"] == "2.0" and comet_body["id"] == 1 and
              set(comet_body["result"]) == {"response"},
              f"Comet response envelope mismatch: {name}")
        comet_response = comet_body["result"]["response"]
        success_fields = {
            "code": 0, "log": "", "info": "", "index": "0",
            "key": None, "proofOps": None, "codespace": "",
        }
        check(all(abci_body.get(field) == value
                  for field, value in success_fields.items()),
              f"ABCI response envelope mismatch: {name}")
        check(all(comet_response.get(field) == value
                  for field, value in success_fields.items()),
              f"Comet ABCI response error: {name}")
        comet = base64.b64decode(comet_response.get("value") or "")
        check(transports["cosmos_grpc"].get("code", 0) == 0,
              f"gRPC error: {name}")
        check(abci == grpc == comet, f"protobuf disagreement: {name}")
        check(abci_body["height"] == str(case["height"]),
              f"ABCI height mismatch: {name}")
        check(comet_response["height"] == str(case["height"]),
              f"Comet height mismatch: {name}")
        check(transports["cosmos_grpc"].get("headers", {}).get(
            "x-cosmos-block-height") == [str(case["height"])],
            f"gRPC height mismatch: {name}")
        rest = transports["rest"]
        check(rest["status"] == 200, f"REST status mismatch: {name}")
        check(rest["headers"].get("Grpc-Metadata-X-Cosmos-Block-Height") ==
              [str(case["height"])], f"REST height mismatch: {name}")


def verify_governance(cases):
    proposals = {
        name: rest_body(cases, name)["proposal"]
        for name in ("proposal-voting", "proposal-deposit", "proposal-expedited")
    }
    check(proposals["proposal-voting"]["id"] == "1", "proposal 1 id mismatch")
    check(proposals["proposal-deposit"]["id"] == "2", "proposal 2 id mismatch")
    check(proposals["proposal-expedited"]["id"] == "3", "proposal 3 id mismatch")
    check(proposals["proposal-voting"]["status"] == "PROPOSAL_STATUS_VOTING_PERIOD",
          "proposal 1 is not voting")
    check(proposals["proposal-deposit"]["status"] == "PROPOSAL_STATUS_DEPOSIT_PERIOD",
          "proposal 2 is not in deposit period")
    check(proposals["proposal-expedited"]["status"] == "PROPOSAL_STATUS_VOTING_PERIOD",
          "proposal 3 is not voting")
    check(proposals["proposal-expedited"]["expedited"] is True,
          "proposal 3 is not expedited")
    check(json.loads(proposals["proposal-voting"]["metadata"]) == {
        "title": "populated voting", "summary": "populated voting summary",
        "authors": ["alice", "bob"],
    }, "structured metadata mismatch")
    expected_wire = {
        "proposal-voting": (1, 2, 0, 9, VAL0_ADDRESS),
        "proposal-deposit": (2, 1, 0, 10, VAL0_ADDRESS),
        "proposal-expedited": (3, 2, 1, 11, USER0_ADDRESS),
    }
    expected_metadata = {
        "proposal-voting": '{"title":"populated voting","summary":"populated voting summary","authors":["alice","bob"]}',
        "proposal-deposit": "ipfs://deposit",
        "proposal-expedited": "ipfs://expedited",
    }
    expected_deposits = {
        "proposal-voting": "10000000",
        "proposal-deposit": "7000000",
        "proposal-expedited": "50000000",
    }
    for name, proposal in proposals.items():
        check(len(proposal["messages"]) == 1, f"missing proposal Any: {name}")
        check(proposal["messages"][0]["@type"] ==
              "/celestia.blob.v1.MsgUpdateBlobParams",
              f"unexpected proposal Any: {name}")
        wire = proposal_wire(cases[name])
        (expected_id, expected_status, expected_expedited, expected_gas,
         expected_proposer) = expected_wire[name]
        check((wire["id"], wire["status"], wire["expedited"],
               wire["gas_per_blob_byte"], wire["gov_max_square_size"],
               wire["proposer"]) ==
              (expected_id, expected_status, expected_expedited, expected_gas,
               64, expected_proposer),
              f"proposal protobuf fields mismatch: {name}")
        check(wire["type_url"] == "/celestia.blob.v1.MsgUpdateBlobParams",
              f"proposal protobuf Any type mismatch: {name}")
        check(proposal["messages"][0]["authority"] == wire["authority"] ==
              GOV_AUTHORITY,
              f"Any authority JSON mismatch: {name}")
        check(proposal["proposer"] == wire["proposer"] == expected_proposer,
              f"proposal proposer mismatch: {name}")
        check(proposal["title"] == wire["title"] and
              proposal["summary"] == wire["summary"] ==
              proposal["title"] + " summary",
              f"proposal title or summary mismatch: {name}")
        json_params = proposal["messages"][0]["params"]
        check(int(json_params["gas_per_blob_byte"]) == wire["gas_per_blob_byte"] and
              int(json_params["gov_max_square_size"]) == wire["gov_max_square_size"],
              f"Any params JSON mismatch: {name}")
        check(proposal["metadata"] == wire["metadata"],
              f"metadata JSON mismatch: {name}")
        check(proposal["metadata"] == expected_metadata[name],
              f"unexpected metadata: {name}")
        check(proposal["total_deposit"] == [{
            "denom": "utia", "amount": expected_deposits[name]
        }], f"proposal deposit total mismatch: {name}")

    first = rest_body(cases, "proposals-first")
    next_page = rest_body(cases, "proposals-next-key")
    offset = rest_body(cases, "proposals-offset")
    reverse = rest_body(cases, "proposals-reverse")
    check(len(first["proposals"]) == 1 and first["pagination"]["total"] == "3",
          "proposal first page mismatch")
    check(first["pagination"]["next_key"], "proposal next key missing")
    check(next_page["proposals"] == offset["proposals"],
          "proposal key and offset pages differ")
    check(first["proposals"] != next_page["proposals"], "proposal page did not advance")
    check(reverse["proposals"][0]["id"] == "3", "proposal reverse page mismatch")
    check(rest_body(cases, "proposals-empty-page")["proposals"] == [],
          "proposal empty page is populated")
    check({p["id"] for p in rest_body(cases, "proposals-voting")["proposals"]} ==
          {"1", "3"}, "voting filter mismatch")
    check([p["id"] for p in rest_body(cases, "proposals-deposit")["proposals"]] ==
          ["2"], "deposit filter mismatch")
    check({p["id"] for p in rest_body(cases, "proposals-voter")["proposals"]} ==
          {"1", "3"},
          "voter filter mismatch")
    check({p["id"] for p in
           rest_body(cases, "proposals-depositor")["proposals"]} == {"2", "3"},
          "depositor filter mismatch")
    check(rest_body(cases, "proposal-historical")["proposal"]["id"] == "1",
          "historical proposal missing")
    check(rest_body(cases, "proposals-historical")["proposals"],
          "historical proposal page empty")

    vote = rest_body(cases, "vote-weighted")["vote"]
    check(vote["voter"] == USER0_ADDRESS, "weighted voter mismatch")
    weights = [(option["option"], option["weight"]) for option in vote["options"]]
    check(weights == [("VOTE_OPTION_YES", "0.600000000000000000"),
                      ("VOTE_OPTION_NO", "0.300000000000000000"),
                      ("VOTE_OPTION_ABSTAIN", "0.100000000000000000")],
          "weighted vote mismatch")
    votes_first = rest_body(cases, "votes-first")
    check(len(votes_first["votes"]) == 1 and votes_first["pagination"]["total"] == "2",
          "votes first page mismatch")
    check(rest_body(cases, "votes-next-key")["votes"] != votes_first["votes"],
          "votes page did not advance")
    check(rest_body(cases, "votes-reverse")["votes"] ==
          rest_body(cases, "votes-next-key")["votes"], "votes reverse mismatch")

    deposit = rest_body(cases, "deposit-secondary")["deposit"]
    check(deposit["proposal_id"] == "2" and
          deposit["depositor"] == USER0_ADDRESS and
          deposit["amount"] == [{"denom": "utia", "amount": "2000000"}],
          "secondary deposit mismatch")
    deposits_first = rest_body(cases, "deposits-first")
    check(len(deposits_first["deposits"]) == 1 and
          deposits_first["pagination"]["total"] == "2",
          "deposits first page mismatch")
    check(rest_body(cases, "deposits-next-key")["deposits"] !=
          deposits_first["deposits"], "deposits page did not advance")
    check(rest_body(cases, "deposits-reverse")["deposits"] ==
          rest_body(cases, "deposits-next-key")["deposits"],
          "deposits reverse mismatch")
    check(rest_body(cases, "tally-voting")["tally"], "tally missing")
    check(rest_body(cases, "params-voting")["params"], "governance params missing")
    check("constitution" in rest_body(cases, "constitution"), "constitution missing")


def verify_archive(capture):
    meta = json.loads((FIXTURE / "meta.json").read_text())
    expected_meta = {
        "scenario": "v105-populated-governance", "chain_id": "rs-devnet-1",
        "celestia_app": "v10.5.0", "celestia_app_next": None,
        "celestia_core": "v0.42.3", "committed_heights": 35,
        "workload": "populated-governance.sh",
    }
    check(set(meta) == set(expected_meta) | {"generated_at"},
          "unexpected devnet metadata fields")
    check(all(meta[name] == value for name, value in expected_meta.items()),
          "devnet metadata mismatch")
    frames = archive_frames(FIXTURE / "blocks.bin.gz")
    check(len(frames) == 70, "unexpected block archive frame count")
    headers = [block_header(frames[index]) for index in range(0, len(frames), 2)]
    check([header["height"] for header in headers] == list(range(1, 36)),
          "block archive height mismatch")
    check([commit_height(frames[index]) for index in range(1, len(frames), 2)] ==
          list(range(1, 36)), "commit archive height mismatch")
    check(all(header["chain_id"] == capture["chain_id"] for header in headers),
          "block archive chain mismatch")
    check(headers[-1]["app_hash"].upper() == capture["app_hash"],
          "block archive root mismatch")
    snapshots = {}
    for height in (1, 35):
        state = json.loads((FIXTURE / f"state/{height}.json").read_text())
        check(state["height"] == height, f"state height mismatch: {height}")
        names = [store["name"] for store in state["stores"]]
        check(names == sorted(names) and len(names) == len(set(names)),
              f"state store inventory mismatch: {height}")
        for store in state["stores"]:
            check(len(store["hash"]) == 64, f"state store hash mismatch: {height}")
            kv = store.get("kv", [])
            check(kv == sorted(kv, key=lambda item: item[0]),
                  f"state keys unsorted: {height}")
            check(len({item[0] for item in kv}) == len(kv),
                  f"duplicate state key: {height}")
            for key, value in kv:
                bytes.fromhex(key); bytes.fromhex(value)
        snapshots[height] = state
    check(snapshots[1]["app_hash"] == headers[1]["app_hash"],
          "initial snapshot root mismatch")
    info = capture["state"]["abci_info"]["result"]["response"]
    check(info["last_block_height"] == "35", "ABCI info height mismatch")
    check(snapshots[35]["app_hash"] ==
          base64.b64decode(info["last_block_app_hash"]).hex(),
          "final snapshot root mismatch")


def main():
    check(FIXTURE.is_dir(), f"missing fixture: {FIXTURE}")
    capture = json.loads((FIXTURE / "capture/transports.json").read_text())
    metadata = json.loads((FIXTURE / "capture/metadata.json").read_text())
    check([case["name"] for case in capture["cases"]] == list(EXPECTED_CASES),
          "governance case order changed")
    cases = {case["name"]: case for case in capture["cases"]}
    check(len(cases) == len(capture["cases"]), "duplicate capture case")
    historical = {"proposal-historical", "proposals-historical"}
    for name, case in cases.items():
        check(case["height"] == (24 if name in historical else 34),
              f"unexpected frozen query height: {name}")
    verify_hashes(metadata)
    verify_observation(metadata)
    verify_transports(capture, metadata, cases)
    verify_governance(cases)
    verify_archive(capture)
    print(f"verified {len(cases)} populated governance cases at "
          f"{capture['chain_id']} height {capture['query_height']}")


if __name__ == "__main__":
    main()
