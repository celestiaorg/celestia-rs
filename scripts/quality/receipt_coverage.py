#!/usr/bin/env python3
"""Audit finite, inclusive height coverage from retained parity JSONL receipts.

This is an offline evidence audit, not a live-node or replay qualification.
Exit 0 means the requested receipt kind covers the range; 1 means gaps;
2 means invalid input. No RPCs, validator homes or signing keys are accessed.
"""

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timedelta
from pathlib import Path

from follow_receipt_audit import results_hash


def ranges(heights):
    result = []
    for height in sorted(heights):
        if result and height == result[-1][1] + 1:
            result[-1][1] = height
        else:
            result.append([height, height])
    return result


def gaps(covered, first, last):
    result = []
    next_height = first
    for start, end in covered:
        if next_height < start:
            result.append([next_height, start - 1])
        next_height = end + 1
    if next_height <= last:
        result.append([next_height, last])
    return result


def timestamp(value):
    if not isinstance(value, str):
        raise ValueError("missing UTC receipt timestamp")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.utcoffset() != timedelta(0):
        raise ValueError("receipt timestamp must be UTC")
    return parsed


COMMITMENT_FIELDS = {"local_results_hash", "reference_results_hash", "header_results_hash",
                     "local_tx_results", "reference_tx_results"}


def committed_payload(row):
    """Recompute optional stronger receipts without authenticating the header."""
    try:
        expected = row["header_results_hash"]
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            return False
        if type(row["reference_header_height"]) is not int or row["reference_header_height"] != row["executed_height"]+1:
            return False
        for prefix in ("local", "reference"):
            rows = row[prefix+"_tx_results"]
            if not isinstance(rows, list) or len(rows) != row[prefix+"_tx_count"]:
                return False
            digest = hashlib.sha256(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            if digest != row[prefix+"_results_sha256"]:
                return False
            normalized = []
            for item in rows:
                if not isinstance(item, dict) or set(item) != {"code", "data", "gas_wanted", "gas_used"}:
                    return False
                converted = dict(item)
                for field in ("code", "gas_wanted", "gas_used"):
                    number = item[field]
                    if not isinstance(number, str) or not re.fullmatch(r"0|-?[1-9][0-9]*", number):
                        return False
                    converted[field] = int(number)
                normalized.append(converted)
            if results_hash(normalized) != row[prefix+"_results_hash"] or row[prefix+"_results_hash"] != expected:
                return False
        return row["local_tx_results"] == row["reference_tx_results"]
    except (KeyError, TypeError, ValueError):
        return False


def matching_payload(row, kind, require_commitment=False):
    if row.get("outcome") != "match" or row.get("error") is not None:
        return None
    if kind == "apphash":
        local, reference = row.get("local_app_hash"), row.get("reference_app_hash")
        header = row.get("reference_header_height")
        if type(header) is not int or header != row["executed_height"] + 1:
            return None
        if not isinstance(local, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", local):
            return None
        if not isinstance(reference, str) or local.upper() != reference.upper():
            return None
        return local.upper()
    if row.get("compared_fields") != ["code", "data", "gas_wanted", "gas_used"]:
        return None
    local, reference = row.get("local_results_sha256"), row.get("reference_results_sha256")
    count = row.get("local_tx_count")
    if not isinstance(local, str) or not re.fullmatch(r"[0-9a-f]{64}", local):
        return None
    if local != reference or type(count) is not int or count < 0:
        return None
    ref_count = row.get("reference_tx_count")
    if type(ref_count) is not int or count != ref_count:
        return None
    if row.get("first_difference_index") is not None:
        return None
    has_commitment = bool(COMMITMENT_FIELDS.intersection(row))
    if (has_commitment or require_commitment) and not committed_payload(row):
        return None
    return (local, count)


def audit(paths, chain, candidate, first, last, kind, require_commitment=False):
    if require_commitment and kind != "tx-results":
        raise ValueError("result commitments require tx-results receipts")
    if first < 1 or last < first:
        raise ValueError("require 1 <= first <= last")
    if not re.fullmatch(r"[0-9a-f]{40}", candidate):
        raise ValueError("candidate must be a full lowercase Git commit SHA")
    matches, rejected, committed = {}, set(), set()
    counts = {"receipts": 0, "duplicates": 0, "outside_range": 0,
              "identity_rejected": 0, "failed_or_invalid": 0}
    artifacts, times = [], []
    for path in paths:
        digest = hashlib.sha256()
        with Path(path).open("rb") as source:
            for line_number, line in enumerate(source, 1):
                digest.update(line)
                try:
                    row = json.loads(line)
                    if not isinstance(row, dict):
                        raise ValueError("receipt must be an object")
                    height = row.get("executed_height")
                    if height is not None and (type(height) is not int or height < 1):
                        raise ValueError("invalid executed height")
                    time = timestamp(row.get("time_utc"))
                except (ValueError, TypeError) as error:
                    raise ValueError(f"{path}:{line_number}: invalid receipt: {error}") from error
                counts["receipts"] += 1
                if height is not None and not first <= height <= last:
                    counts["outside_range"] += 1
                    continue
                if (row.get("chain_id") != chain or row.get("reference_chain_id") != chain
                        or row.get("candidate_commit") != candidate):
                    counts["identity_rejected"] += 1
                    # An unavailable reference commonly has no chain ID.
                    # Preserve that height as unknown even if a retry matches;
                    # rejecting identity must not bypass failure accounting.
                    if height is not None:
                        rejected.add(height)
                    continue
                payload = matching_payload(row, kind, require_commitment) if height is not None else None
                if payload is None:
                    counts["failed_or_invalid"] += 1
                    if height is not None:
                        rejected.add(height)
                    continue
                if kind == "tx-results" and COMMITMENT_FIELDS.intersection(row):
                    committed.add(height)
                times.append(time)
                if height in matches:
                    counts["duplicates"] += 1
                    if matches[height] != payload:
                        rejected.add(height)
                else:
                    matches[height] = payload
        artifacts.append({"path": str(path), "sha256": digest.hexdigest()})
    # A later success cannot erase a retained failure at the same height.
    covered = ranges(matches.keys() - rejected)
    missing = gaps(covered, first, last)
    return {
        "schema_version": 1, "scope": "offline retained receipt height coverage",
        "kind": kind, "chain_id": chain, "candidate_commit": candidate,
        "requested_range": [first, last], "covered_ranges": covered,
        "missing_ranges": missing, "rejected_ranges": ranges(rejected),
        "covered_heights": sum(end - start + 1 for start, end in covered),
        "result_commitment_required": require_commitment,
        "result_commitment_checked_heights": len(committed - rejected),
        "receipt_time_range_utc": [min(times).isoformat(), max(times).isoformat()] if times else None,
        "counts": counts, "artifacts": artifacts,
        "complete": not missing and not any(counts[key] for key in
                                               ("identity_rejected", "failed_or_invalid")),
        "limitations": [
            "Receipt timestamps are check times, not block times or proof of continuous uptime.",
            "Coverage does not prove snapshot provenance, executable identity, signing or monitoring health.",
            "Transaction results cover only code, data, gas_wanted and gas_used.",
            "Input digests identify supplied files; receipts are not authenticated by this audit.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chain", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--first", required=True, type=int)
    parser.add_argument("--last", required=True, type=int)
    parser.add_argument("--kind", required=True, choices=("apphash", "tx-results"))
    parser.add_argument("--require-result-commitment", action="store_true",
                        help="require retained result lists and H+1 commitments for tx-results")
    parser.add_argument("receipts", nargs="+")
    args = parser.parse_args()
    if args.require_result_commitment and args.kind != "tx-results":
        parser.error("--require-result-commitment requires --kind tx-results")
    try:
        report = audit(args.receipts, args.chain, args.candidate, args.first, args.last, args.kind, args.require_result_commitment)
    except (ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    sys.exit(main())
