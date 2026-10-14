# IBC edge-case vectors

`gen_test.go` drives a celestia-app v10.4.0 chain with ibc-go's testing
package (`ibctesting`) against two simapp chains with packet forward
middleware, and records the celestia chain's ABCI traffic (InitChain,
FinalizeBlock, Commit) as a devnet fixture. It covers what a live relayer
does not produce: the handshake's Try/Confirm side, redundant and tampered
relays, timeouts by height and timestamp, timeout-on-close and close
confirm, PFM forwards, refunds, timeout retries and invalid forward
metadata, the ICS-27 host (account creation and executed / rejected txs),
misbehaviour freezing a client, a header signed by the wrong validator set
and an expired client.

The program imports celestia-app's `test/interop` simapp, so it runs inside
a celestia-app v10.4.0 checkout:

```bash
mkdir -p <celestia-app>/test/ibcvectors
cp conformance/ibcvectors/gen_test.go <celestia-app>/test/ibcvectors/
cd <celestia-app>
IBCVEC_OUT=$PWD/../celestia-rs/conformance/fixtures/devnet/ibc-edge \
  go test -count=1 -run TestGenerateIBCVectors ./test/ibcvectors
```

Notes:

- Txs go through a recording wrapper of the app (`SendMsgsOverride`); fees
  are paid in utia, so no state is changed out of band on celestia.
- The counterparty chains are not recorded; their state may be edited
  directly (closing A's channel end, the ICA controller's channel end and
  packet commitments, since no simapp here has the ICS-27 controller).
- Keys are random per run; regenerating replaces the fixture wholesale.

`crates/devnet-conformance/tests/replay_scenarios.rs` (`ibc_edge_parity`)
replays the fixture; `cargo run -p devnet-conformance --example acks --
ibc-edge` lists the acknowledgements Go wrote.
