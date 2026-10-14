//! Every devnet scenario must replay with full parity: app hash and the
//! consensus fields of every tx result (code, codespace, data, gas) at every
//! height, plus validator updates.

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
    Ok(())
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
