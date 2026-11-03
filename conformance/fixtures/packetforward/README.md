# PFM metadata validation acknowledgements

These 15 cases per app version retain acknowledgement bytes and their IBC
commitments from pinned Go v9.0.8 / v10.5.0, built with Go 1.26.9. They cover
reserved Unicode identifiers at channel/port length boundaries, escaped and
literal JSON, and ordinary Unicode/ASCII controls.

`json_pfm_ack` first classifies and decodes the memo. When official PFM
`ForwardMetadata.Validate` rejects it, the Go adapter calls the real
`IBCMiddleware.OnRecvPacket` and `channeltypes.CommitAcknowledgement`.
Validation returns before keeper or transfer access; a logger/context and
otherwise empty keeper suffice for this bounded path. Valid metadata returns
`valid`, without attempting a transfer. The ordinary `json_pfm` operation
still only decodes and classifies metadata.

The Rust callback test runs the actual packet-forward middleware on the 14
invalid cases and compares both byte strings. Thirty recorded oracle outcomes
also run without Go in `fuzz-diff/tests/regressions.jsonl`; the nightly JSON
rotation includes the new operation. These tests do not verify IBC proofs,
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
checks agreement between the
two reference versions, and verifies each emitted acknowledgement commitment.
