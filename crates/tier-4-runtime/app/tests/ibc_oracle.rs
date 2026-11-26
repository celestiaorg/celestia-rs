//! The IBC chain against the Go oracle (`conformance/v1-oracle`
//! `TestIBCFixture`, fixture `conformance/fixtures/synthetic/ibc-v2`).
//!
//! celestia-app v3.13.0 is chain A of an ibc-go v6 ibctesting
//! coordinator with two ibc-go simapp counterparties. It crosses the v1
//! to v2 upgrade at block 2 (`--v2-upgrade-height 3`), then opens
//! transfer channels to B and C, sends utia to B, and receives it back
//! three times with a packet-forward-middleware `forward` memo to C:
//!
//! - forwarded (block 37) and acknowledged by C (block 42, which writes
//!   the acknowledgement of B's packet);
//! - forwarded (block 45) and rejected by C (block 50: the utia moves
//!   back to the A-B escrow and B's packet gets an error
//!   acknowledgement);
//! - forwarded with one retry (block 53), timed out and sent again
//!   (block 56), timed out again (block 58: A gives up, as above).
//!
//! Then it opens an interchain account channel for a controller on B
//! (blocks 69 and 72, which create the account), executes the account's
//! delegate, send and withdraw-rewards messages (block 75), and receives
//! two transactions that fail with error acknowledgements: one signed by
//! another account (block 78) and one whose send exceeds the balance
//! (block 81, which rolls back its delegation too).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("ibc-v2")
}

/// The last block that celestia-rs executes as Go does: every block.
const MATCHED_THROUGH: i64 = 83;

/// The fixture files are the ones the oracle wrote.
#[test]
fn ibc_fixture_files_match_their_record() {
    let f = fix();
    let r = f.roots();
    let sha = |b: &[u8]| hex::encode(hash::sha256(b));
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 3);
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
    assert_eq!(f.last_height(), 83);
}

#[test]
fn ibc_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
