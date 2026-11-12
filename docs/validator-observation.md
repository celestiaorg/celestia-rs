# Exact-process validator observations on macOS

`scripts/quality/observe_validator.py` observes one already-running validator
through public RPCs, its execution log, and read-only process inspection. It
never opens validator keys, submits transactions, restarts nodes or deploys
binaries. A deployment hold is a separate operator action described in
[operator guide §8](operator-guide.md).

The [pooled-reference observation record](pooled-validator-observation-2026-11-12.md)
retains a separately started durable window and bounded live transport probes.
It preserves the earlier window's gaps and states the later acceptance deadline.

Supply a JSON configuration with `network`, `chain_id`, `pid`, `binary`,
`home`, `log`, `local_rpc`, `reference_rpc`, `validator_address` (uppercase
public hexadecimal address), and the full `candidate_commit`. The command is:

```sh
python3 scripts/quality/observe_validator.py --config observer.json \
  --output /private/tmp/observation-new --seconds 262800 --interval 5
```

The output directory must not exist. Each run starts at the first height
after its local `/status` baseline; it does not claim earlier coverage.
Run one process per network. Install a fixed copy of the script outside a
development checkout so branch changes cannot alter collection. Keep the
sibling `follow_receipt_audit.py` module beside it: the observer uses its
Go-vector-tested deterministic transaction-result commitment implementation. A finite
launchd job with `RunAtLoad` and without `KeepAlive` can outlive a terminal.
Do not automatically restart it into the same observation after failure.

`events.jsonl` records the candidate's embedded version, binary SHA-256,
loaded/installed inode match, PID, baseline and terminal event. Every step
checks the process/home and inode again, measures the binary bytes again,
and records that measurement in `samples.jsonl`. Samples retain local and
independent reference statuses, separate observation times, height lag and
reference failures. They are discrete observations; endpoints, outages and
time between them remain unknown. Five seconds is a requested interval,
not a guarantee: RPC latency and bounded catch-up work extend it.

`heights.jsonl` joins each actual `committed block` execution-log line at H
to the independent header H+1 app hash, independent commit H block hash,
and local/reference transaction results. Ordered comparisons cover `code`,
`data`, `gas_wanted` and `gas_used`, with exact integer normalization,
counts and canonical JSON digests. New receipts additionally retain both
normalized result lists, their deterministic protobuf/Merkle commitments and
H+1's `last_results_hash`. Both computed commitments must match that header
field; equal local/reference responses with a different commitment stop the
run as a parity mismatch. Missing/malformed commitments remain unavailable.
This is a consistency check against the retained reference response; it does
not authenticate that header. Older receipts lack these additional fields
and cannot establish this stronger check retrospectively.

The receipt also records whether the
public validator address signed that reference commit, signed nil, missed,
or was outside the reference validator set. Absent signatures trigger an
independent validator-set lookup to distinguish the last two cases.

Failures retain the attempted height and error and leave the cursor in place.
Retries append additional receipts; they do not erase unknown observations.
Execution-log rotation/truncation, process or binary changes, missing
execution records, more than 10,000 queued execution heights, or an actual
parity mismatch stop the run. Logs, events and other ABCI fields are outside
the transaction comparison. Reference responses are trusted HTTPS RPC
observations: this script does **not** cryptographically verify their commit
signatures or independently establish reference executable versions. Node
logs and receipts are not authenticated. Separate monitoring and acceptance
review remain necessary for #299.

For offline height coverage, supply `heights.jsonl` to
[`receipt_coverage.py`](receipt-coverage.md), separately for `--kind apphash`
and `--kind tx-results`, with the explicit chain, candidate and required
inclusive range. Do not infer a required interval from successful receipts.
Retain every receipt file, including failures, and preserve process and
monitoring gaps. These files are evidence inputs, not a qualification verdict.

The observer creates private files. Bound its lifetime (`--seconds`, default
73 hours), monitor disk use, and retain a digest manifest before archiving
receipts. A successful short canary establishes only that canary's coverage;
72-hour qualification still needs the full continuous operational record.

## Auditing a requested observation window

`scripts/quality/observation_audit.py` additionally checks the observer's
`events.jsonl`, `samples.jsonl` and `heights.jsonl` together. Copy all three
files into a separate snapshot directory first: changing inputs are rejected.
Supply the expected chain, candidate and binary SHA-256 independently, plus
an explicit UTC interval and its inclusive executed height range:

```sh
python3 scripts/quality/observation_audit.py observation-snapshot \
  --chain celestia --candidate "$candidate" --binary-sha256 "$binary_sha256" \
  --start 2026-11-12T01:00:00Z --end 2026-11-12T01:10:00Z \
  --first 15810038 --last 15810248
```

Determine required heights from independent execution logs covering both
interval edges, rather than from successful comparison receipts. The auditor
checks each retained execution line against the claimed height, block hash,
app hash and interval. It reports missing height/participation ranges and
contradictory signing observations. Retries do not erase unavailable attempts.

Sample checks require the recorded binary bytes and process identity to
match, both chains to agree, block times to be fresh, and independently
recorded height lag to agree with both statuses. Defaults allow at most a
20-second sample gap, three heights of absolute reference lag and a 60-second
block age; the selected thresholds are retained in the report. Gaps include
both requested interval edges. Duplicate/backwards samples, stale binary
measurements, outages and collector interruptions remain explicit failures.

Exit 0 means these bounded sample, execution-link, participation-coverage and
receipt checks pass for the supplied interval. It does not require that the
interval lasts 72 hours, bound missed-block percentages, verify reference
signatures, authenticate input logs or inspect separate Prometheus/guard/panic
records. `qualification_established` remains false. Review those independent
requirements before claiming C1.1, F1.8 or #299 is complete. File digests bind
the supplied snapshots; they do not authenticate their contents.

## Result-commitment validation on live validators

Two independent 120-second read-only probes on 2026-11-12 checked the added
result commitment comparison using observer revision `01a3188`. Both signing
validators remained on candidate `46acbcb`; the original three-day collectors
continued running. Clean-source wrappers passed with unchanged process/binary
identity. Both probes reached their normal deadline with all 39 attempted
heights matching and 24 status samples each:

| Network | Inclusive executed heights | Compared transactions |
|---|---|---:|
| Mocha `mocha-5` | 2,542,577–2,542,615 | 38 |
| Mainnet `celestia` | 15,815,126–15,815,164 | 177 |

[Retained probe evidence](evidence/observer-commitment-2026-11-12/summary.json)
includes compressed raw events, samples and per-height normalized results,
wrapper receipts and file digests. Every retained matching result commitment
was recomputed offline from those result lists and matched H+1's recorded
commitment. This validates nonempty live transaction handling in the stronger
collector. It does not extend the earlier collectors' claims retrospectively,
verify reference signatures or establish continuous three-day qualification.
