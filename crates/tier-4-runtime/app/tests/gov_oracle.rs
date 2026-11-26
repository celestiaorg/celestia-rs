//! The governance chain against the Go oracle (`conformance/v1-oracle
//! -synthetic -chain gov-v1-v3`, fixture `conformance/fixtures/synthetic/gov-v1-v3`).
//!
//! celestia-app v3.13.0 runs the synthetic chain's genesis with 40 second
//! deposit and voting periods (about 3 blocks). At app version 1 (blocks
//! 2 to 13) and again at app version 3 (blocks 19 to 26, after the v1 to
//! v2 upgrade at block 13 and the signalled upgrade at block 18):
//! community pool spends that pass (with a delegator's authz vote
//! overriding its validator's), expire in the deposit period, are vetoed
//! or rejected; parameter changes through the param filter (slashing
//! `SignedBlocksWindow`, baseapp `BlockParams`, which updates the
//! consensus params in blocks 7 and 23, blob `GovMaxSquareSize`, icahost
//! `AllowMessages`) and blocked ones, whose submission fails; v1beta1 and
//! v1 messages, `MsgExecLegacyContent`, a second `MsgDeposit` that
//! starts the voting period, and weighted votes.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("gov-v1-v3")
}

/// The last block that celestia-rs executes as Go does.
const MATCHED_THROUGH: i64 = 26;

/// The fixture files are the ones the oracle wrote.
#[test]
fn gov_fixture_files_match_their_record() {
    let f = fix();
    let r = f.roots();
    let sha = |b: &[u8]| hex::encode(hash::sha256(b));
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 14);
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
    assert_eq!(f.last_height(), 26);
}

#[test]
fn gov_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
