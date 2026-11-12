#!/usr/bin/env python3
"""Audit one finite follower ledger's identity, height coverage and result hashes.

This validates retained records, not their authenticity or network signatures.
Bind the measured executable to clean source with separate build evidence.
"""

import argparse
import base64
from datetime import datetime
import gzip
import hashlib
import json
from pathlib import Path
import re
import sys

SCOPE = "local execution compared with authenticated network header commitments; not an independent signature proof"
START = {"candidate_commit", "binary_sha256", "software_version", "chain_id", "requested_stop_height", "process_proposal_checked", "witness_interval", "reference_server_count", "scope"}
HEIGHT = {"executed_height", "local_block_hash", "local_app_hash", "reference_app_hash", "local_results_hash", "reference_results_hash", "reference_header_height", "reference_header_hash", "reference_server_index", "next_header_app_version", "tx_count", "tx_results"}


def integer(value, minimum, maximum, label):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(f"invalid {label}")
    return value


def fields(value, expected):
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError("missing or unexpected record fields")


def hex_hash(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("invalid SHA-256 hash")
    return value


def varint(value):
    data = bytearray()
    while value > 127:
        data.append((value & 127) | 128)
        value >>= 7
    data.append(value)
    return bytes(data)


def result_bytes(result):
    fields(result, {"code", "data", "gas_wanted", "gas_used"})
    code = integer(result["code"], 0, 2**32-1, "result code")
    if not isinstance(result["data"], str):
        raise ValueError("result data must be base64")
    data = base64.b64decode(result["data"], validate=True)
    if base64.b64encode(data).decode() != result["data"]:
        raise ValueError("result data is not canonical base64")
    wanted = integer(result["gas_wanted"], -2**63, 2**63-1, "gas wanted")
    used = integer(result["gas_used"], -2**63, 2**63-1, "gas used")
    # deterministicExecTxResult retains protobuf fields 1, 2, 5 and 6.
    encoded = (b"\x08"+varint(code)) if code else b""
    if data:
        encoded += b"\x12"+varint(len(data))+data
    for tag, value in ((b"\x28", wanted), (b"\x30", used)):
        if value:
            encoded += tag+varint(value & (2**64-1))
    return encoded


def results_hash(results):
    if not isinstance(results, list):
        raise ValueError("transaction results must be an array")
    leaves = [hashlib.sha256(b"\0"+result_bytes(result)).digest() for result in results]

    def root(items):
        if not items:
            return hashlib.sha256(b"").digest()
        if len(items) == 1:
            return items[0]
        split = 1 << ((len(items)-1).bit_length()-1)
        return hashlib.sha256(b"\1"+root(items[:split])+root(items[split:])).digest()
    return root(leaves).hex()


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON key")
        value[key] = item
    return value


def audit(path, chain, candidate, binary, first, last):
    if not isinstance(candidate, str) or not re.fullmatch(r"[0-9a-f]{40}", candidate):
        raise ValueError("candidate must be a full lowercase Git SHA")
    hex_hash(binary)
    integer(first, 1, 2**63-1, "first height")
    integer(last, first, 2**63-1, "last height")
    start = baseline = ended = anchored = False
    next_height, transactions, server_count, anchor = first, 0, None, None
    previous_header = None
    digest = hashlib.sha256()
    opener = gzip.open if Path(path).suffix == ".gz" else open
    with opener(path, "rb") as source:
        for number, raw in enumerate(source, 1):
            digest.update(raw)
            try:
                if not raw.endswith(b"\n") or ended:
                    raise ValueError("truncated or trailing record")
                row = json.loads(raw, object_pairs_hook=unique_object)
                fields(row, {"schema_version", "event", "time_utc", "data"})
                integer(row["schema_version"], 1, 1, "schema version")
                stamp = row["time_utc"]
                if not isinstance(stamp, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?Z", stamp):
                    raise ValueError("record timestamp must be UTC")
                datetime.fromisoformat(stamp.replace("Z", "+00:00"))
                event, data = row["event"], row["data"]
                if not start:
                    if event != "run_start":
                        raise ValueError("ledger must start with run_start")
                    fields(data, START)
                    if (data["candidate_commit"] != candidate or data["binary_sha256"] != binary
                            or data["chain_id"] != chain or data["requested_stop_height"] != last
                            or data["process_proposal_checked"] is not True or data["scope"] != SCOPE):
                        raise ValueError("run identity or verification scope differs")
                    integer(data["requested_stop_height"], last, last, "requested stop")
                    integer(data["witness_interval"], 0, 2**63-1, "witness interval")
                    server_count = integer(data["reference_server_count"], 1, 2**64-1, "server count")
                    if not isinstance(data["software_version"], str) or not data["software_version"]:
                        raise ValueError("missing software version")
                    start = True
                elif not baseline:
                    if event != "baseline":
                        raise ValueError("missing baseline")
                    fields(data, {"anchor_height", "anchor_block_hash", "anchor_app_hash", "first_execution_height"})
                    anchor = integer(data["anchor_height"], 0, first-1, "anchor height")
                    if anchor != first-1 or data["first_execution_height"] != first:
                        raise ValueError("baseline range differs")
                    integer(data["first_execution_height"], first, first, "baseline first height")
                    hex_hash(data["anchor_app_hash"])
                    if anchor:
                        hex_hash(data["anchor_block_hash"])
                    elif data["anchor_block_hash"] != "":
                        raise ValueError("genesis anchor has a block hash")
                    baseline = True
                elif event == "anchor_verified":
                    fields(data, {"height", "reference_header_height", "reference_header_hash"})
                    if not anchor or anchored or next_height != first:
                        raise ValueError("unexpected anchor verification")
                    integer(data["height"], anchor, anchor, "verified anchor")
                    integer(data["reference_header_height"], first, first, "anchor header height")
                    previous_header = hex_hash(data["reference_header_hash"])
                    anchored = True
                elif event == "height":
                    fields(data, HEIGHT)
                    if (anchor and not anchored) or next_height > last:
                        raise ValueError("missing anchor or unexpected height")
                    integer(data["executed_height"], next_height, next_height, "executed height")
                    integer(data["reference_header_height"], next_height+1, next_height+1, "reference header height")
                    integer(data["reference_server_index"], 0, server_count-1, "reference server index")
                    integer(data["next_header_app_version"], 0, 2**64-1, "header app version")
                    for key in ("local_block_hash", "local_app_hash", "reference_app_hash", "local_results_hash", "reference_results_hash", "reference_header_hash"):
                        hex_hash(data[key])
                    if previous_header is not None and data["local_block_hash"] != previous_header:
                        raise ValueError("header chain differs")
                    previous_header = data["reference_header_hash"]
                    if data["local_app_hash"] != data["reference_app_hash"]:
                        raise ValueError("app hash differs")
                    count = integer(data["tx_count"], 0, 2**64-1, "transaction count")
                    if (not isinstance(data["tx_results"], list) or count != len(data["tx_results"])
                            or results_hash(data["tx_results"]) != data["local_results_hash"]
                            or data["local_results_hash"] != data["reference_results_hash"]):
                        raise ValueError("transaction-result commitment differs")
                    transactions += count
                    next_height += 1
                elif event == "run_end":
                    fields(data, {"status", "execution_error", "first_execution_height", "last_executed_height", "last_verified_height", "executed_blocks", "executed_transactions"})
                    if data["status"] != "complete" or data["execution_error"] is not False:
                        raise ValueError("run_end is not complete")
                    if next_height != last+1:
                        raise ValueError("missing requested heights")
                    for key, expected in {"first_execution_height": first, "last_executed_height": last,
                                          "last_verified_height": last, "executed_blocks": last-first+1,
                                          "executed_transactions": transactions}.items():
                        integer(data[key], expected, expected, key)
                    ended = True
                else:
                    raise ValueError("unexpected event")
            except (ValueError, TypeError, KeyError) as error:
                raise ValueError(f"line {number}: {error}") from error
    if not ended:
        raise ValueError("missing complete run_end")
    return {"schema_version": 1, "scope": "offline follower ledger self-consistency",
            "chain_id": chain, "candidate_commit": candidate, "binary_sha256": binary,
            "input_uncompressed_sha256": digest.hexdigest(), "covered_range": [first, last],
            "covered_heights": last-first+1, "transactions": transactions, "complete": True,
            "qualification_established": False, "network_signature_proof_verified": False,
            "limitations": ["The candidate is an operator claim; bind the measured binary to clean source separately.",
                            "Header authentication is reported by the producer; this audit has no independent signature proof.",
                            "Receipt integrity is not authenticity: fully fabricated records can be self-consistent.",
                            "This bounded ledger does not establish other historical eras, upgrades or continuous uptime."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ledger")
    for name in ("chain", "candidate", "binary-sha256"):
        parser.add_argument("--"+name, required=True)
    for name in ("first", "last"):
        parser.add_argument("--"+name, type=int, required=True)
    args = parser.parse_args()
    try:
        result = audit(args.ledger, args.chain, args.candidate, args.binary_sha256, args.first, args.last)
    except (OSError, EOFError, ValueError, TypeError) as error:
        parser.exit(2, str(error)+"\n")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
