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
