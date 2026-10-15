//! Every devnet scenario must replay with full parity: app hash and the
//! consensus fields of every tx result (code, codespace, data, gas) at every
//! height, validator updates, PrepareProposal responses (txs, square size,
//! data root), ProcessProposal status and CheckTx responses (code,
//! codespace, gas, priority, signer address and sequence).

use devnet_conformance::replay::replay;

fn assert_parity(scenario: &str) -> anyhow::Result<()> {
    let r = replay(scenario)?;
    assert!(r.errors.is_empty(), "{scenario}: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "{scenario}: stores {:#?}",
        r.store_diffs
    );
    assert!(
        !r.matched_heights.is_empty(),
        "{scenario}: nothing replayed"
    );
    assert!(
        r.compared.prepare > 0 && r.compared.process > 0,
        "{scenario}: proposals not replayed: {:?}",
        r.compared
    );
    Ok(())
}

/// `workloads/cli.sh`: txs built, signed and broadcast by the Rust CLI
/// (checked identical to the Go CLI's) on a Go node.
#[test]
fn cli_parity() -> anyhow::Result<()> {
    assert_parity("cli")
}

#[test]
fn staking_parity() -> anyhow::Result<()> {
    assert_parity("staking")
}

#[test]
fn staking_lifecycle_parity() -> anyhow::Result<()> {
    assert_parity("staking-lifecycle")
}

#[test]
fn hyperlane_parity() -> anyhow::Result<()> {
    assert_parity("hyperlane")
}

#[test]
fn gov_parity() -> anyhow::Result<()> {
    assert_parity("gov")
}

#[test]
fn authz_feegrant_parity() -> anyhow::Result<()> {
    assert_parity("authz-feegrant")
}

#[test]
fn vesting_parity() -> anyhow::Result<()> {
    assert_parity("vesting")
}

#[test]
fn consensus_params_parity() -> anyhow::Result<()> {
    assert_parity("consensus")
}

/// `staking` re-driven through Go with duplicate-vote evidence injected
/// (conformance/devnet/inject-evidence.sh staking evidence 20 30): slash,
/// jail, tombstone; unknown validator, unknown type and an already
/// tombstoned validator are ignored.
#[test]
fn evidence_parity() -> anyhow::Result<()> {
    assert_parity("evidence")
}

/// Gov-scheduled v10 upgrade (handler runs), schedule + cancel, a repeated
/// completed upgrade, then a signalled v11 upgrade: the app version bumps
/// and both Go and Rust refuse the next block (no v11 handler).
#[test]
fn upgrade_parity_and_halt() -> anyhow::Result<()> {
    assert_parity("upgrade")?;
    let r = devnet_conformance::replay::replay("upgrade")?;
    assert!(r.halted_at.is_some(), "Go halted but Rust did not");
    Ok(())
}

/// x/fibre + x/valaddr (`workloads/fibre.sh`): provider registration,
/// escrow deposits/withdrawals (paid out by the BeginBlocker),
/// MsgPayForFibre settlement (PFF system blobs in the square, validator
/// signatures checked in CheckTx and ProcessProposal), unsettleable
/// promises dropped by PrepareProposal, timeouts and a gov
/// MsgUpdateFibreParams.
#[test]
fn fibre_parity() -> anyhow::Result<()> {
    assert_parity("fibre")?;
    let c = devnet_conformance::replay::replay("fibre")?.compared;
    assert!(c.prepare_with_txs >= 15 && c.check_tx >= 40, "{c:?}");
    Ok(())
}

/// x/zkism + x/forwarding (`workloads/zkism-forwarding.sh`): SP1 Groth16
/// state-transition and membership proofs (celestia-app's test vectors)
/// verified on chain, failing proofs, the zk ISM on the hyperlane router,
/// and MsgForward through warp with IGP fee collection/refund.
#[test]
fn zkism_forwarding_parity() -> anyhow::Result<()> {
    assert_parity("zkism-forwarding")
}

#[test]
fn blob_parity() -> anyhow::Result<()> {
    assert_parity("blob")?;
    let c = devnet_conformance::replay::replay("blob")?.compared;
    assert!(c.prepare_with_txs >= 7 && c.check_tx >= 50, "{c:?}");
    Ok(())
}

/// Two Go chains linked by a relayer: client creation and updates,
/// connection and channel handshakes, ICS-20 escrow / mint / burn, error
/// acks, packet-forward-middleware forwards (success and failure) and a
/// packet timeout, all from the main chain's side.
#[test]
fn ibc_parity() -> anyhow::Result<()> {
    assert_parity("ibc")
}

/// IBC edge cases from ibc-go's testing package driving a recorded
/// celestia-app against simapp chains (conformance/ibcvectors): handshake
/// Try/Confirm, redundant and invalid relays, timeouts by height and time,
/// timeout-on-close and close confirm, PFM forwards / refunds / retries and
/// invalid forward metadata, the ICA host (account creation, executed and
/// rejected txs), misbehaviour freezing a client, a wrong-valset header and
/// an expired client.
/// ibctesting drives FinalizeBlock and Commit directly (no proposals or
/// CheckTx), so only the block-level parity applies.
/// `conformance/ibcvectors/upgrade_test.go`: 06-solomachine clients, ICS-27
/// host channel upgrades, proto3json ICA txs, `MsgModuleQuerySafe` and the
/// gov-executed `MsgIBCSoftwareUpgrade`, with CheckTx of every tx.
#[test]
fn ibc_upgrades_parity() -> anyhow::Result<()> {
    let r = replay("ibc-upgrades")?;
    assert!(r.errors.is_empty(), "ibc-upgrades: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "ibc-upgrades: stores {:#?}",
        r.store_diffs
    );
    assert!(r.matched_heights.len() > 180, "{}", r.matched_heights.len());
    assert!(r.compared.check_tx > 140, "{:?}", r.compared);
    Ok(())
}

#[test]
fn ibc_edge_parity() -> anyhow::Result<()> {
    let r = replay("ibc-edge")?;
    assert!(r.errors.is_empty(), "ibc-edge: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "ibc-edge: stores {:#?}",
        r.store_diffs
    );
    assert!(
        r.matched_heights.len() > 150,
        "ibc-edge: {:?}",
        r.matched_heights.len()
    );
    Ok(())
}

/// App version 9 (celestia-app v9.0.8, ADR 0004): an empty chain started at
/// app version 9.
#[test]
fn v9_empty_parity() -> anyhow::Result<()> {
    assert_parity("v9-empty")
}

/// The v10 workloads recorded from Go v9.0.8 (`APP_V9=1`).
#[test]
fn v9_workloads_parity() -> anyhow::Result<()> {
    for s in [
        "v9-staking",
        "v9-gov",
        "v9-authz-feegrant",
        "v9-vesting",
        "v9-blob",
        "v9-hyperlane",
        // Two Go v9 chains linked by cosmos/relayer.
        "v9-ibc",
        // celestia-app v9's SP1 v5 Groth16 vectors (x/zkism testdata/v9).
        "v9-zkism-forwarding",
    ] {
        assert_parity(s)?;
    }
    Ok(())
}

/// `workloads/v9-exec.sh`: v9-only behavior: sequential `deliverTx` (two
/// txs of one account in one block, the second cannot pay its fee),
/// multi-denom fees and their priority, `MsgExec` wrapping
/// `MsgSubmitProposal`.
#[test]
fn v9_exec_parity() -> anyhow::Result<()> {
    assert_parity("v9-exec")
}

/// App version 9 → 10 like mainnet / Mocha (`workloads/v9-upgrade.sh`):
/// txs under v9, MsgSignalVersion + MsgTryUpgrade, the v9 EndBlocker bumps
/// the app version, the recorder switches to the v10 binary after that
/// Commit (as the multiplexer does), the v10 store loader adds fibre and
/// valaddr, the v10 handler runs, then txs under v10.
#[test]
fn v9_upgrade_parity() -> anyhow::Result<()> {
    let r = replay("v9-upgrade")?;
    assert!(r.errors.is_empty(), "v9-upgrade: {:?}", r.errors);
    assert_eq!(r.first_mismatch, None, "v9-upgrade: {:#?}", r.store_diffs);
    assert!(r.matched_heights.len() >= 40, "{:?}", r.matched_heights);
    assert!(r.compared.check_tx > 0, "{:?}", r.compared);
    Ok(())
}
