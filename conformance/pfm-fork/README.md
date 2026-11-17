# Packet-forward fork memo oracle

Run `go test -run TestFixture -update .` here to regenerate
`../fixtures/pfm-fork.json`; `go test ./...` fails when the committed fixture
is stale.

The module replaces `packet-forward-middleware/v8` with the Celestia fork
(celestiaorg/ibc-apps#3) at commit
`15b9622da053b82091a89be4c4e42677297858d8`, fetched through the Go module
proxy as pseudo-version `v8.2.1-0.20261117023558-15b9622da053`. From app
version 12, celestia-app runs this fork and celestia-rs mirrors it.

The test runs the fork's real `OnRecvPacket` and `OnTimeoutPacket` with the
fork's own mocked keepers (`test.NewTestSetup`), as its unit tests do. For each
receive memo it records whether the packet goes to the transfer app, gets an
error acknowledgement (its bytes), or is forwarded (the `MsgTransfer`
receiver, port, channel, timeout at block time 0, retries, and memo bytes).
For each retry memo it records the memo of the packet sent again. Long memos
are stored as runs, and forwarded memos by length and SHA-256.

The Rust test `x/packetforward/tests/fork_go.rs` checks the app version 12
decisions against this fixture. It does not cover gas, store writes, refunds,
or celestia-app's replacement of the error acknowledgement (Rust middleware
tests cover those).
