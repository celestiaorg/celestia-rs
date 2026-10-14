//! Differential replay of the `empty` devnet scenario.
//!
//! `EXPECTED_MATCHING_STORES` ratchets: every store listed must match Go at
//! the first diverging height. Add stores as modules are ported; when all
//! heights match, `full_app_hash_parity` passes and replaces this.

use devnet_conformance::replay::replay;

const EXPECTED_MATCHING_STORES: &[&str] = &[
    "authz",
    "capability",
    "consensus",
    "evidence",
    "feegrant",
    "hyperlane",
    "gov",
    "ibc",
    "icahost",
    "fibre",
    "packetfowardmiddleware",
    "blob",
    "circuit",
    "minfee",
    "mint",
    "params",
    "signal",
    "transfer",
    "upgrade",
    "valaddr",
    "warp",
    "zkism",
];

#[test]
fn ratchet_matching_stores() -> anyhow::Result<()> {
    let r = replay("empty")?;
    if r.first_mismatch.is_none() && r.errors.is_empty() {
        return Ok(());
    }
    for s in EXPECTED_MATCHING_STORES {
        assert!(
            r.matching_stores.iter().any(|m| m == s),
            "store {s} regressed; matching={:?} errors={:?}",
            r.matching_stores,
            r.errors
        );
    }
    Ok(())
}

#[test]
fn full_app_hash_parity() -> anyhow::Result<()> {
    let r = replay("empty")?;
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.first_mismatch, None, "stores: {:#?}", r.store_diffs);
    Ok(())
}
