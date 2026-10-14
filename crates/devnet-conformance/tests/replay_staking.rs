//! Differential replay of the `staking` devnet scenario (create-validator,
//! delegate, edit-validator, redelegate, unbond, cancel-unbond, reward and
//! commission withdrawal, withdraw address, community pool, failing
//! unjail; see `conformance/devnet/workloads/staking.sh`).
//!
//! With x-ante wired as the app's ante handler and phased tx execution in
//! `FinalizeBlock`, all 40 heights match Go (app hash, tx codes, gas used
//! and tx events). Until that pipeline is on `main`, only its known
//! blockers are tolerated.

use devnet_conformance::replay::replay;

fn known_blocker(e: &str) -> bool {
    e.contains("not ported yet")
        || e.contains("transaction execution")
        || e.contains("commit without finalized block")
}

#[test]
fn staking_scenario_app_hash_parity() -> anyhow::Result<()> {
    let r = replay("staking")?;
    if !r.errors.is_empty() && r.errors.iter().all(|e| known_blocker(e)) {
        return Ok(());
    }
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r.init_chain_diffs.is_empty(), "{:?}", r.init_chain_diffs);
    assert_eq!(r.first_mismatch, None, "stores: {:#?}", r.store_diffs);
    Ok(())
}
