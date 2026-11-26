//! The staking chain against the Go oracle (`conformance/v1-oracle
//! -synthetic -chain staking-v1-v3`, fixture
//! `conformance/fixtures/synthetic/staking-v1-v3`).
//!
//! celestia-app v3.13.0 runs the synthetic chain's genesis with a 60
//! second unbonding time and a 10 block signing window (half signed, a
//! 60 second downtime jail). At app version 1 (blocks 2 to 19): a fee
//! allowance used and revoked; authz StakeAuthorization and
//! SendAuthorization through one `MsgExec`, then `MsgRevoke`; an
//! undelegation and a redelegation that mature in the EndBlocker (blocks
//! 7 and 8) and a cancelled unbonding; a validator edit, commission and
//! reward withdrawals to a withdraw address; a continuous vesting account
//! delegating locked coins and a delayed one; a multi-send; val3 jailed
//! for downtime (block 11) and unjailed (block 16). After the v1 to v2
//! upgrade (block 19) and the signalled upgrade (block 24), at app
//! version 3: a periodic fee allowance, an authz undelegation, a
//! cancelled unbonding, a redelegation and a vesting account's
//! undelegation, maturing in blocks 30 and 31.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("staking-v1-v3")
}

/// The last block that celestia-rs executes as Go does.
const MATCHED_THROUGH: i64 = 32;

/// The fixture files are the ones the oracle wrote.
#[test]
fn staking_fixture_files_match_their_record() {
    let f = fix();
    let r = f.roots();
    let sha = |b: &[u8]| hex::encode(hash::sha256(b));
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 20);
    assert_eq!(r["genesis_sha256"], sha(&f.read("genesis.json")));
    assert_eq!(r["blocks_sha256"], sha(&f.gunzip("blocks.jsonl.gz")));
    assert_eq!(
        r["tx_results_sha256"],
        sha(&f.gunzip("tx-results.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-height-0.jsonl"],
        sha(&f.gunzip("kv-height-0.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-diffs.jsonl"],
        sha(&f.gunzip("kv-diffs.jsonl.gz"))
    );
    assert_eq!(f.last_height(), 32);
}

#[test]
fn staking_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
