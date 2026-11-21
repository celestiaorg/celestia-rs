# Packet-forward fork memo oracle

Run `go test -run TestFixture -update .` here to regenerate
`../fixtures/pfm-fork.json`; `go test ./...` fails when the committed fixture
is stale.

The module replaces `packet-forward-middleware/v8` with the Celestia fork
(celestiaorg/ibc-apps#3) at its release tag
`middleware/packet-forward-middleware/v8.2.1-celestia` (module version
`v8.2.1-celestia`, commit `106efffd6a4be3460e33ccf63b0e034b9be709f9`). From
app version 12, celestia-app runs this fork and celestia-rs mirrors it.

The test runs the fork's real `OnRecvPacket` and `OnTimeoutPacket` with the
fork's own mocked keepers (`test.NewTestSetup`), as its unit tests do. For each
receive memo it records whether the packet goes to the transfer app, gets an
error acknowledgement (its bytes), or is forwarded (the `MsgTransfer`
receiver, port, channel, timeout at block time 0, retries, and memo bytes).
With each error acknowledgement the fork emits a `packet_forward_error` event
that carries the Go error text; the test records its attributes, and checks
that no other outcome emits one. For each retry memo it records the memo of
the packet sent again, and for a forward that times out with no retries left
the event of the error acknowledgement written for the source packet.

The error text can hold ibc-go source paths, so the module also replaces
ibc-go with the `celestiaorg/ibc-go` release that celestia-app builds with. Long memos
are stored as runs, and forwarded memos by length and SHA-256.

The Rust test `x/packetforward/tests/fork_go.rs` checks the app version 12
decisions and error events against this fixture. It does not cover gas, store
writes, refunds, or celestia-app's replacement of the error acknowledgement
(Rust middleware tests cover those).

`cmd/fuzzoracle` is the Go side of the differential fuzzer
(`crates/fuzz-diff`) for app version 12: it answers the `json_pfm`,
`json_pfm_ack` and `json_pfm_next` ops with this fork (see its doc comment),
for example `cargo run -p fuzz-diff -- --app-version 12 --ops
json_pfm,json_pfm_ack,json_pfm_next`.
