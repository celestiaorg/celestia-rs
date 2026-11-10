# Auditing retained parity receipt coverage

`scripts/quality/receipt_coverage.py` reads existing JSONL receipts without
contacting a node or accessing a validator home. Supply one receipt kind,
one exact candidate, one chain and an inclusive required height range:

```sh
python3 scripts/quality/receipt_coverage.py \
  --chain celestia \
  --candidate b459a6e041b2675582019cde5c75ecf2969318ba \
  --first 15530001 --last 15530101 --kind tx-results \
  conformance/mainnet-rpc/tx-result-receipts/2026-11-06-isolated-replay-15530001-15530101.jsonl
```

This retained historical artifact produces 101 covered heights, no missing
heights, and input SHA-256
`f6acdade007a288bf6c8de4308f624e3e7589abd6cc9f4001220a162e66eb6f8`.
The candidate is the historical run's identity; it is not the current main
branch's identity. Extending `--first` to 15530000 reports the missing
15,530,000 receipt and exits 1.

For app-hash receipts from `check-apphash.sh --json`, use `--kind apphash`.
Supply additional JSONL paths as positional arguments for rotated files.
The audit records each input digest, distinct covered height ranges, missing
and rejected ranges, receipt counts, duplicate counts and receipt time bounds.
It does not infer a required range from the first and last receipt: callers
must supply the intended range so missing leading/trailing heights stay visible.

A successful row requires both chain IDs and the full candidate commit to
match, a UTC timestamp, a match outcome without an error, and agreeing
payloads. App-hash checks also require reference header H+1. Transaction
checks require equal counts and digests for `code`, `data`, `gas_wanted` and
`gas_used`. Repeated identical receipts count once per height. An unavailable,
invalid or contradictory receipt at a known height excludes that height even
if another receipt reports success. Identity rejections and failures without
a known height prevent an overall complete result. Out-of-range rows are
counted separately and do not contribute coverage. Malformed JSON, heights
or timestamps fail the audit with exit 2; reported gaps/failures exit 1.

`complete: true` means coverage of the requested receipt kind and height
range only. It does not authenticate receipt contents or the executable
identity asserted by them. A pair of matching digests is evidence of a
recorded comparison, not a reconstruction of the original responses.
Receipt times are comparison times, not block times. The report does not
establish continuous at-tip operation, reference height lag, signing,
monitoring availability, snapshot provenance or alert delivery. App-hash
and transaction-result files must be audited separately; neither substitutes
for the other. Logs/events and other ABCI result fields are outside the
transaction comparison scope.

The [replay inventory](replay-coverage-inventory.md) retains historical
coverage and provenance limitations. Issues [#298](https://github.com/celestiaorg/celestia-rs/issues/298),
[#299](https://github.com/celestiaorg/celestia-rs/issues/299) and
[#302](https://github.com/celestiaorg/celestia-rs/issues/302) require the actual
missing receipts and operational evidence before closure. The receipt audit
does not turn the passage of three days into a completed qualification.
