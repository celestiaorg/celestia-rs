//! The IBC chain against the Go oracle (`conformance/v1-oracle`
//! `TestIBCFixture`, fixture `conformance/fixtures/synthetic/ibc-v2`).
//!
//! celestia-app v3.13.0 is chain A of an ibc-go v6 ibctesting
//! coordinator with two ibc-go simapp counterparties. It crosses the v1
//! to v2 upgrade at block 2 (`--v2-upgrade-height 3`), then opens
//! transfer channels to B and C, sends utia to B, receives it back with
//! a packet-forward-middleware `forward` memo and forwards it to C
//! (block 37), acknowledges the forward (block 42, which writes the
//! acknowledgement of B's packet), opens an interchain account channel
//! for a controller on B (blocks 53 and 56) and executes the account's
//! delegate, send and withdraw-rewards messages (block 59).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("ibc-v2")
}

/// The last block that celestia-rs executes as Go does: block 37
/// receives the packet that packet-forward-middleware forwards, which is
/// not ported yet.
const MATCHED_THROUGH: i64 = 36;

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
    assert_eq!(f.last_height(), 61);
}

#[test]
fn ibc_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
