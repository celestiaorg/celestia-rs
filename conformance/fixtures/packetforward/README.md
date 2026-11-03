# PFM rejected-metadata acknowledgements

The 15 validation cases and 55 parse-error cases per app version retain
acknowledgement bytes and their IBC commitments from pinned Go v9.0.8 /
v10.5.0, built with Go 1.26.9. Validation cases cover reserved Unicode
identifiers at channel/port length boundaries, escaped and literal JSON,
and ordinary Unicode/ASCII controls.

`json_pfm_ack` first classifies and decodes the memo. When official PFM
typed metadata decoding or `ForwardMetadata.Validate` rejects it, the Go adapter
calls the real
`IBCMiddleware.OnRecvPacket` and `channeltypes.CommitAcknowledgement`.
Parsing/validation rejection returns before keeper state or transfer access;
a logger/context and
otherwise empty keeper suffice for this bounded path. Valid metadata returns
`valid`, without attempting a transfer. The ordinary `json_pfm` operation
still only decodes and classifies metadata. Non-forward, nil-forward and valid
metadata outputs remain classification markers, without invoking transfer code.
For this operation, the generic fuzzer's Go-accept counter includes error ACKs
and these markers; it is not a count of successful packet deliveries.

The Rust callback test runs the actual packet-forward middleware on all 69
invalid cases and compares both byte strings. It also requires the 55 new cases
to take the parse-error branch, before field validation. The parse cases cover
wrong field types, uint8 overflow/number syntax, duration parsing and overflow,
Unicode/control-character quoting, duplicate/folded fields and the precedence of
custom unmarshal errors over saved type errors. The 140 recorded oracle outcomes
run without Go in `fuzz-diff/tests/regressions.jsonl`; the nightly JSON
rotation already includes this operation. These tests do not verify IBC proofs,
execute a full MsgRecvPacket lifecycle or qualify production readiness.

Regenerate into a fresh directory, then review/copy the outputs:

```sh
for v in v9 v10; do
  (cd conformance/fuzzoracle/$v && \
    GOTOOLCHAIN=go1.26.9 GOFLAGS=-mod=readonly \
    go build -buildvcs=false -o /tmp/pfm-oracle-$v .)
done
GOTOOLCHAIN=go1.26.9 python3 conformance/fuzzoracle/pfm-validation-vectors.py \
  --v9 /tmp/pfm-oracle-v9 --v10 /tmp/pfm-oracle-v10 --out /tmp/pfm-vectors-new
cargo test -p x-packetforward -p fuzz-diff
```

`provenance.json` records oracle checksums/compiler/module build information,
source-file hashes, exact app/PFM/IBC source commits and the callback fixture
hash. The generator checks Go 1.26.9 and the pinned app/dependency versions,
checks agreement between the two reference versions, and verifies each emitted acknowledgement commitment.

For parse-error vectors, pass
`--cases conformance/fixtures/packetforward/memo-parse-error-acks.json` to the
same generator with a separate fresh output directory. The output fixture
keeps the input basename; its separate `provenance.json` is retained in the
repository as `memo-parse-error-provenance.json`. Validation provenance retains
its original generation hashes. Neither receipt transfers qualification to
later source revisions.

The expanded operation matched 20,006 mutation inputs at each pinned app
version (seed 173), with no output disagreement. These are dirty-source
component diagnostics, not qualification volume, IBC proof tests or a complete
error-path inventory.
