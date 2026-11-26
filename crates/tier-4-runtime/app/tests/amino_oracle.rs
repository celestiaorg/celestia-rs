//! The amino chains against the Go oracle (`conformance/v1-oracle
//! -synthetic -chain amino-v1-v3` and `amino-core-v1-v3`, fixtures
//! `conformance/fixtures/synthetic/amino-v1-v3` and `amino-core-v1-v3`).
//!
//! Every transaction is signed in `SIGN_MODE_LEGACY_AMINO_JSON` by the
//! cosmos-sdk v0.46 client, at app versions 1 (blocks 2 to 4), 2 (blocks
//! 6 to 8) and 3 (blocks 14 to 16): authz grants of every authorization
//! type and `MsgExec` of each inner message, fee allowances and
//! fee-granted transactions, gov proposals of every content type through
//! v1beta1 and v1, votes and deposits, staking, distribution, bank,
//! vesting, crisis, evidence, slashing, ibc transfer and relayer messages
//! (which have no amino sign bytes: Go's ante handler panics), 2-of-3
//! multisig accounts with amino, direct and mixed sub-signatures, and
//! transactions signed over the wrapped form of a value the module codec
//! writes bare, which Go rejects. celestia-rs must accept and reject
//! exactly the signatures Go does, with Go's results.
//!
//! The core chain leaves out the cases celestia-rs does not sign as Go
//! yet and signs the version signals in DIRECT mode; the full chain adds
//! them (top-level periodic vesting, `MsgPayForBlobs` and signal
//! messages, whose sign bytes are bare amino or proto-JSON, blobstream
//! `MsgRegisterEVMAddress` and `MsgExec` of a client update, whose sign
//! bytes panic in Go). `conformance/amino-sign-bytes-v046.md` lists the
//! codec of every message's sign bytes.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::fixture::Fixture;

/// The fixture files are the ones the oracle wrote.
fn check_record(name: &str) {
    let f = Fixture::synthetic(name);
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
fn amino_fixture_files_match_their_record() {
    check_record("amino-v1-v3");
    check_record("amino-core-v1-v3");
}

#[test]
fn amino_core_blocks_match_the_oracle() {
    Fixture::synthetic("amino-core-v1-v3").run_blocks(17);
}

/// The last block of the full chain that celestia-rs executes as Go does:
/// block 3 holds the first of the cases the core chain leaves out.
const FULL_MATCHED_THROUGH: i64 = 2;

#[test]
fn amino_blocks_match_the_oracle() {
    Fixture::synthetic("amino-v1-v3").run_blocks(FULL_MATCHED_THROUGH);
}
