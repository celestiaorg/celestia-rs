# Per-height RPC follower receipts

`follow --receipts <new-file> --receipt-candidate <full-commit>` retains a
JSONL ledger for one execution run. Use the source revision from the binary's
clean build evidence. The revision is an operator claim; the ledger also hashes
the executable file at startup. Separate qualification evidence must bind that
measured binary to the claimed source. Receipt mode requires execution and
cannot be combined with `--light-only`.

A height H is recorded after header H+1 has been compared with local state and
authenticated by the existing follower verifier. The receipt includes app hashes,
transaction-result hashes, the following header's height/hash and reference
server index. It retains every local transaction's consensus fields: code,
base64 data, gas wanted and gas used. Their protobuf/Merkle result commitment
is recomputed from retained FinalizeBlock results and must equal the state and
network header commitments. Logs, events and codespaces are outside that
consensus commitment. This does not claim full response parity.

Each file starts with `run_start` and `baseline`. A restored/resumed checkpoint
has a separate `anchor_verified` record and is not counted as execution by this
invocation. `height` records describe only newly executed blocks. A finite run
gets `run_end` status `complete` only after its final executed height has also
been checked against the authenticated next header. Shutdown reports
`interrupted`; execution failure reports `failed`. Startup or I/O failure can
leave no terminal record. Such files remain incomplete even if their last
retained height matched.

The file must be new, with an existing parent directory. An existing file is
never overwritten or appended to. Resume with a new filename and retain the
previous run. Writes flush each JSON line; terminal file synchronization errors
propagate. A partial/truncated file is not completion evidence. Size grows with
the number of heights and retained transaction-result bytes. RPC credentials
and URLs are not copied into the ledger; source server indices refer to the
RPC order supplied to the invocation.

For a fresh non-signing home containing a verified Mocha-5 genesis:

```sh
REPLAY_HOME=/path/to/isolated-replay
REPLAY_BINARY=/path/to/qualified/celestia-rs
REPLAY_COMMIT=... # full lowercase source revision from that binary's build receipt
"$REPLAY_BINARY" follow --home "$REPLAY_HOME" --chain-id mocha-5 \
  --rpc https://rpc.celestia-mocha.com,https://celestia-testnet-rpc.polkachu.com \
  --stop-at-height 1000 --trust-period-hours 10000 \
  --receipts "$REPLAY_HOME/heights-1-1000.jsonl" --receipt-candidate "$REPLAY_COMMIT"
```

The existing `follow(&FollowOptions)` library API remains available. Library
callers can opt into `follow_with_receipts` with `FollowReceiptOptions`.

## Offline audit

```sh
python3 scripts/quality/follow_receipt_audit.py /path/to/heights-1-1000.jsonl \
  --chain mocha-5 --candidate "$REPLAY_COMMIT" --binary-sha256 <binary-sha256> \
  --first 1 --last 1000
```

The audit accepts plain JSONL or gzip, requires the complete requested range
for that invocation and a matching terminal record, checks identity, record
order, header-hash links and counts, and recomputes each result commitment. It
rejects duplicate JSON keys, missing/reordered heights, changed consensus result
fields, mismatched roots and interrupted/failed/truncated runs. The wire/Merkle
implementation has retained vectors generated against both pinned Go core
versions, including signed gas limits and non-power-of-two result sets. The
vector generator is in `scripts/quality/fixtures/follow_results_vectors.go` and
runs from the matching v9/v10 fuzzoracle module with its pinned toolchain.

This is a self-consistency audit. Header authentication is a producer claim;
the ledger is not a portable signature proof, and a fully fabricated ledger can
be internally consistent. Bind retained files to trusted capture/build evidence.
The report always leaves `qualification_established=false` and
`network_signature_proof_verified=false`. A bounded ledger covers neither other
historical eras nor the separate upgrade, review or continuous-observation gates.

## Bounded check on 2026-11-12

A [clean-source build and smoke run](evidence/follow-receipts-2026-11-12/manifest.json)
of `1cb60aa` produced and audited all 100 height receipts for Mocha heights
1–100, authenticating header 101 before completion. The verified genesis hash
was checked before and after the run. This interval had zero transactions;
nonempty result commitments are exercised by the offline RPC integration test
and the pinned v9/v10 Go vectors. The retained audit leaves qualification and
independent network signature proof false. This new replay binary does not
replace the installed validator candidate `46acbcb` or close #302.
