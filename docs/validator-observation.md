# Exact-process validator observations on macOS

`scripts/quality/observe_validator.py` observes one already-running validator
through public RPCs, its execution log, and read-only process inspection. It
never opens validator keys, submits transactions, restarts nodes or deploys
binaries. A deployment hold is a separate operator action described in
[operator guide §8](operator-guide.md).

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
development checkout so branch changes cannot alter collection. A finite
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
counts and canonical JSON digests. The receipt also records whether the
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
