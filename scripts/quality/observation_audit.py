#!/usr/bin/env python3
"""Audit a caller-specified validator observation window without RPC access.

This checks retained samples and height receipts, not continuous operation or
all qualification requirements. Missing evidence stays explicit.
"""

import argparse
from datetime import timedelta
import hashlib
import json
import math
from pathlib import Path
import re
import sys

import receipt_coverage


def read_rows(path):
    artifact = {"path": path.name, "rows": 0}
    def rows():
        digest = hashlib.sha256()
        with path.open("rb") as source:
            for number, line in enumerate(source, 1):
                digest.update(line)
                try:
                    row = json.loads(line)
                    if not isinstance(row, dict):
                        raise ValueError("receipt must be an object")
                    receipt_coverage.timestamp(row.get("time_utc"))
                    artifact["rows"] += 1
                    yield row
                except (ValueError, TypeError) as error:
                    raise ValueError(f"{path.name}:{number}: {error}") from error
        artifact["sha256"] = digest.hexdigest()
    return rows(), artifact


def height(value):
    if type(value) not in (str, int) or not re.fullmatch(r"[1-9][0-9]*", str(value)):
        raise ValueError("invalid sample height")
    return int(value)


def sample_error(row, identity, chain, binary, address, max_lag, max_age):
    try:
        if row.get("outcome") != "observed" or row.get("error") is not None:
            raise ValueError(row.get("error") or "unavailable sample")
        if row.get("identity") != identity or row.get("binary_sha256") != binary:
            raise ValueError("sample executable identity differs")
        checked = receipt_coverage.timestamp(row["time_utc"])
        measured = receipt_coverage.timestamp(row["binary_measured_at_utc"])
        local_time = receipt_coverage.timestamp(row["local_status_time_utc"])
        ref_time = receipt_coverage.timestamp(row["reference_status_time_utc"])
        if not measured <= local_time <= ref_time <= checked or checked-measured > timedelta(seconds=20):
            raise ValueError("invalid or stale sample measurement times")
        local, reference = row["local_status"], row["reference_status"]
        if local["validator_info"]["address"] != address:
            raise ValueError("sample validator differs")
        for status, observed in ((local, local_time), (reference, ref_time)):
            if status["node_info"]["network"] != chain:
                raise ValueError("sample chain differs")
            sync = status["sync_info"]
            if sync["catching_up"] is not False:
                raise ValueError("sample node is catching up")
            age = (observed-receipt_coverage.timestamp(sync["latest_block_time"])).total_seconds()
            if not -5 <= age <= max_age:
                raise ValueError("sample block time is stale or in the future")
        lag = height(reference["sync_info"]["latest_block_height"])-height(local["sync_info"]["latest_block_height"])
        if type(row.get("reference_height_minus_local")) is not int or row["reference_height_minus_local"] != lag:
            raise ValueError("sample lag disagrees with statuses")
        if abs(lag) > max_lag:
            raise ValueError("sample exceeds allowed reference lag")
        return None
    except (KeyError, TypeError, ValueError) as error:
        return str(error)


def audit(directory, chain, candidate, binary, first, last, start, end,
          max_gap=20, max_lag=3, max_age=60):
    start, end = receipt_coverage.timestamp(start), receipt_coverage.timestamp(end)
    if end <= start or not math.isfinite(max_gap) or not math.isfinite(max_age) or max_gap <= 0 or max_lag < 0 or max_age <= 0:
        raise ValueError("invalid observation window or sample thresholds")
    if not re.fullmatch(r"[0-9a-f]{64}", binary):
        raise ValueError("binary must be a full lowercase SHA-256")
    directory = Path(directory)
    events, event_artifact = read_rows(directory/"events.jsonl")
    events = list(events)
    samples, sample_artifact = read_rows(directory/"samples.jsonl")
    heights, height_artifact = read_rows(directory/"heights.jsonl")
    starts = [row for row in events if row.get("event") == "start"]
    baselines = [row for row in events if row.get("event") == "baseline"]
    if len(starts) != 1 or len(baselines) != 1:
        raise ValueError("require exactly one collector start and baseline")
    initial, baseline = starts[0], baselines[0]
    address = initial["config"]["validator_address"]
    if not re.fullmatch(r"[0-9A-F]{40}", address) or baseline["local_status"]["validator_info"]["address"] != address:
        raise ValueError("invalid or contradictory validator address")
    provenance = initial["identity"]
    if provenance["binary_sha256"] != binary or provenance["candidate_commit"] != candidate:
        raise ValueError("initial executable provenance differs")
    identity = {key: provenance[key] for key in ("pid", "inode", "device", "size", "mtime_ns")}
    if first < baseline["first_required_height"]:
        raise ValueError("requested heights precede collector baseline")
    if receipt_coverage.timestamp(baseline["time_utc"]) > start:
        raise ValueError("requested time precedes collector baseline")
    for row in (initial, baseline):
        if row.get("chain_id") != chain or row.get("candidate_commit") != candidate:
            raise ValueError("collector baseline identity differs")
    failures, valid_times = [], []
    previous = None
    for row in samples:
        checked = receipt_coverage.timestamp(row["time_utc"])
        if not start <= checked <= end:
            continue
        error = None
        if row.get("chain_id") != chain or row.get("candidate_commit") != candidate:
            error = "sample candidate/chain differs"
        elif previous is not None and checked <= previous:
            error = "sample times duplicate or run backwards"
        else:
            error = sample_error(row, identity, chain, binary, address, max_lag, max_age)
        previous = checked
        if error:
            failures.append({"time_utc": row["time_utc"], "error": error})
        else:
            valid_times.append(checked)
    # Include both requested edges; no first/last-success inference.
    points = [start, *sorted(set(valid_times)), end]
    gaps = [{"start_utc": a.isoformat(), "end_utc": b.isoformat(),
             "seconds": (b-a).total_seconds()}
            for a, b in zip(points, points[1:]) if (b-a).total_seconds() > max_gap]
    interruptions = [row for row in events if row.get("event") in ("stopped", "unavailable", "deadline")
                     and start <= receipt_coverage.timestamp(row["time_utc"]) < end]
    participation = {}
    participation_errors = []
    for row in heights:
        h = row.get("executed_height")
        if type(h) is not int or not first <= h <= last or row.get("outcome") != "match":
            continue
        try:
            execution = row["execution"]
            match = re.search(r"^(\S+).*committed block height=(\d+) hash=([A-Fa-f0-9]{64}) app_hash=([A-Fa-f0-9]{64})", execution["log_line"])
            if (row.get("chain_id") != chain or row.get("candidate_commit") != candidate
                    or row.get("identity") != identity or not match or int(match[2]) != h
                    or not start <= receipt_coverage.timestamp(match[1]) <= end
                    or match[3].upper() != execution["block_hash"].upper()
                    or match[4].upper() != execution["app_hash"].upper()
                    or execution["app_hash"].upper() != row["local_app_hash"].upper()
                    or execution["block_hash"].upper() != row["reference_block_hash"].upper()):
                raise ValueError("execution identity, time or hash differs")
        except (KeyError, ValueError, TypeError, AttributeError) as error:
            participation_errors.append({"height": h, "error": str(error)})
            continue
        outcome = row.get("participation", {}).get("outcome")
        if outcome not in ("signed", "nil", "missed", "not_in_set"):
            participation_errors.append({"height": h, "error": "missing participation outcome"})
        elif h in participation and participation[h] != outcome:
            participation_errors.append({"height": h, "error": "contradictory participation outcomes"})
        else:
            participation[h] = outcome
    missing_participation = receipt_coverage.gaps(receipt_coverage.ranges(participation), first, last)
    audits = {kind: receipt_coverage.audit([directory/"heights.jsonl"], chain, candidate, first, last, kind)
              for kind in ("apphash", "tx-results")}
    for result in audits.values():
        if result["artifacts"][0]["sha256"] != height_artifact["sha256"]:
            raise ValueError("height receipts changed during audit")
        result["artifacts"][0]["path"] = "heights.jsonl"
    artifacts = [event_artifact, sample_artifact, height_artifact]
    for artifact in artifacts:
        with (directory/artifact["path"]).open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != artifact["sha256"]:
                raise ValueError(artifact["path"]+" changed during audit; use a frozen copy")
    return {
        "schema_version": 1, "scope": "offline retained observation samples and explicit height coverage",
        "chain_id": chain, "candidate_commit": candidate, "binary_sha256": binary, "validator_address": address,
        "requested_time_range_utc": [start.isoformat(), end.isoformat()],
        "requested_seconds": (end-start).total_seconds(), "requested_height_range": [first, last],
        "thresholds": {"max_sample_gap_seconds": max_gap, "max_absolute_reference_lag": max_lag,
                       "max_block_age_seconds": max_age},
        "valid_sample_count": len(valid_times), "sample_failures": failures, "sample_gaps": gaps,
        "collector_interruptions": interruptions,
        "sample_requirements_met": bool(valid_times) and not (failures or gaps or interruptions),
        "participation_counts": {outcome: list(participation.values()).count(outcome)
                                 for outcome in ("signed", "nil", "missed", "not_in_set")},
        "missing_participation_ranges": missing_participation, "participation_errors": participation_errors,
        "height_audits": audits, "artifacts": artifacts,
        "qualification_established": False,
        "limitations": [
            "Samples are discrete observations; time between them remains unobserved.",
            "Requested heights must come from independent execution logs, including interval edges.",
            "Height comparisons may be retrospective; check times do not establish execution at tip.",
            "Reference commit participation is an RPC assertion, not cryptographic verification.",
            "Input digests identify supplied files; they do not authenticate receipts or logs.",
            "Separate monitoring, guard/panic logs, duration and remaining qualification criteria require review.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory")
    for name in ("chain", "candidate", "binary-sha256", "start", "end"):
        parser.add_argument("--"+name, required=True)
    for name in ("first", "last"):
        parser.add_argument("--"+name, required=True, type=int)
    parser.add_argument("--max-gap", type=float, default=20)
    parser.add_argument("--max-lag", type=int, default=3)
    parser.add_argument("--max-age", type=float, default=60)
    args = parser.parse_args()
    try:
        report = audit(args.directory, args.chain, args.candidate, args.binary_sha256,
                       args.first, args.last, args.start, args.end, args.max_gap, args.max_lag, args.max_age)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, str(error)+"\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if (report["sample_requirements_met"] and not report["missing_participation_ranges"]
                 and not report["participation_errors"]
                 and all(item["complete"] for item in report["height_audits"].values())) else 1


if __name__ == "__main__":
    sys.exit(main())
