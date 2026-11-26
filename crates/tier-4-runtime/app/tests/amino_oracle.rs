//! The amino chain against the Go oracle (`conformance/v1-oracle
//! -synthetic -chain amino-v1-v3`, fixture
//! `conformance/fixtures/synthetic/amino-v1-v3`).
//!
//! Every transaction is signed in `SIGN_MODE_LEGACY_AMINO_JSON` by the
//! cosmos-sdk v0.46 client, at app versions 1 (blocks 2 to 4), 2 (blocks
//! 6 to 8) and 3 (blocks 14 to 16): authz grants of every authorization
//! type and `MsgExec` of each inner message, fee allowances and
//! fee-granted transactions, gov proposals of every content type through
//! v1beta1 and v1, votes and deposits, staking, distribution, bank,
//! vesting, crisis, evidence, slashing, signal, blobstream,
//! `MsgPayForBlobs`, ibc transfer and relayer messages (which have no
//! amino sign bytes: Go's ante handler panics), 2-of-3 multisig accounts
//! with amino, direct and mixed sub-signatures, and transactions signed
//! over the wrapped form of a value the module codec writes bare, which
//! Go rejects. celestia-rs must accept and reject exactly the signatures
//! Go does, with Go's results.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("amino-v1-v3")
}

/// The last block that celestia-rs executes as Go does.
const MATCHED_THROUGH: i64 = 17;

/// The fixture files are the ones the oracle wrote.
#[test]
fn amino_fixture_files_match_their_record() {
    let f = fix();
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
fn amino_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}
