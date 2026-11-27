//! The failures chain against the Go oracle (`conformance/v1-oracle
//! -synthetic -chain failures-v1-v3`, fixture
//! `conformance/fixtures/synthetic/failures-v1-v3`).
//!
//! At app versions 1 (blocks 2 to 4), 2 (blocks 6 to 8) and 3 (blocks 14
//! to 16), transactions fail in the ante handler (wrong sequence, wrong
//! chain ID, out of gas in the ante handler and in a message, no fee, a
//! fee above the balance, a memo too large, a past timeout height) and in
//! message handlers (a multi-message rollback, authz without or with an
//! expired grant, fee grants missing, spent or expired, distribution and
//! staking errors, a send to a blocked module account, too many unbonding
//! entries). Each failure's code, codespace and gas_used go into
//! `LastResultsHash`, so celestia-rs must fail at the same point as Go.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::fixture::Fixture;

#[test]
fn failures_fixture_files_match_their_record() {
    let f = Fixture::synthetic("failures-v1-v3");
    let r = f.roots();
    let sha = |b: &[u8]| hex::encode(hash::sha256(b));
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 6);
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
    assert_eq!(f.last_height(), 17);
}

#[test]
fn failures_blocks_match_the_oracle() {
    Fixture::synthetic("failures-v1-v3").run_blocks(17);
}
