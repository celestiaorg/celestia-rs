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

`upgrade_test.go` (`TestGenerateIBCUpgradeVectors`, fixture `ibc-upgrades`,
CheckTx of every tx recorded too) covers what the first fixture leaves out:

- 06-solomachine clients (single key and 3-of-3 multisig): headers that
  rotate the key, tampered / stale / wrong-key / multisig-short headers,
  connection and channel handshakes with the solo machine as counterparty
  (membership proofs), packets both ways, a tampered commitment, empty and
  stale proofs, a timeout (non-membership), misbehaviour (identical
  signatures rejected by ValidateBasic, then a freezing one), and recovery
  of the frozen client from a substitute through gov.
- ICS-27 host channel upgrades (the only upgradable route: PFM wraps
  transfer and is not an `UpgradableModule`, so transfer channel upgrades
  fail with `upgrade route not found`): ORDERED proto3 -> UNORDERED
  proto3json with an in-flight packet (Try, Confirm to FLUSHCOMPLETE,
  receive while flushing, Open), host-callback rejections (connection
  hops, account address), cancel by error receipt and by the gov
  authority, a timeout, a Confirm returning `FAILURE` after the
  counterparty timeout elapsed, and a gov `ChanUpgradeInit` refused by the
  host. The controller end on A is driven with A's channel keeper.
- proto3json ICA txs (valid, camelCase / spaced JSON, unknown type,
  unknown field, truncated JSON, `{}`, a registered non-`sdk.Msg` type,
  protobuf bytes on a JSON channel) and `MsgModuleQuerySafe` through ICA
  (protobuf and JSON) and as a plain tx (allowed via genesis
  `allow_messages`): every module-query-safe bank / auth / staking /
  circuit query, unsafe paths, bad request data, missing accounts.
- `MsgIBCSoftwareUpgrade` executed by gov: stopped by celestia's circuit
  breaker (and by the circuit ante for a plain tx), then
  `MsgResetCircuitBreaker` and a scheduled plan, up to the upgraded
  consensus state stored one block before the plan height.

```bash
IBCVEC_UPGRADE_OUT=$PWD/../celestia-rs/conformance/fixtures/devnet/ibc-upgrades \
  go test -count=1 -run TestGenerateIBCUpgradeVectors ./test/ibcvectors
```

`crates/devnet-conformance/tests/replay_scenarios.rs` (`ibc_edge_parity`,
`ibc_upgrades_parity`)
replays the fixture; `cargo run -p devnet-conformance --example acks --
ibc-edge` lists the acknowledgements Go wrote.
