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
//! vesting, crisis, evidence, slashing, ibc transfer and relayer messages
//! (which have no amino sign bytes: Go's ante handler panics), 2-of-3
//! multisig accounts with amino, direct and mixed sub-signatures, and
//! transactions signed over the wrapped form of a value the module codec
//! writes bare, which Go rejects. celestia-rs must accept and reject
//! exactly the signatures Go does, with Go's results.
//!
//! Among them: top-level periodic vesting (bare amino JSON),
//! `MsgPayForBlobs` and the signal messages (proto3 JSON: their
//! `ModuleCdc` is a `ProtoCodec`), and blobstream `MsgRegisterEVMAddress`
//! and `MsgExec` of a client update, whose sign bytes panic in Go.
//! `conformance/amino-sign-bytes-v046.md` lists the codec of every
//! message's sign bytes.

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
}

#[test]
fn amino_blocks_match_the_oracle() {
    Fixture::synthetic("amino-v1-v3").run_blocks(17);
}

/// The SHA-256 of the app version 4 and later (cosmos-sdk v0.50, x/tx)
/// amino JSON sign bytes, or error, of every transaction of the full
/// chain, captured before the v0.46 sign bytes changes: those changes
/// stay behind the v0.46 gate.
const V4_SIGN_BYTES_SHA256: &str =
    "ac5bf06dc329a4676e748d078dc97845612b459221df8cfb58c9b4c22e6ab9ae";

#[test]
fn amino_sign_bytes_from_app_version_4_are_unchanged() {
    use base64::Engine as _;
    use prost::Message as _;
    use std::fmt::Write as _;
    let f = Fixture::synthetic("amino-v1-v3");
    let sd = x_ante::signing::SignerData {
        address: "celestia1qqqsyqcyq5rqwzqfpg9scrgwpugpzysn7hzdtn".into(),
        chain_id: "test".into(),
        account_number: 7,
        sequence: 3,
        pub_key: celestia_proto::Any::default(),
    };
    let mut all = String::new();
    for (h, block) in f.blocks() {
        let txs = block["block"]["data"]["txs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (i, t) in txs.iter().enumerate() {
            let mut raw = base64::engine::general_purpose::STANDARD
                .decode(t.as_str().unwrap())
                .unwrap();
            if let Ok(b) = celestia_proto::celestia_core_proto::tendermint::types::BlobTx::decode(
                raw.as_slice(),
            ) && b.type_id == "BLOB"
            {
                raw = b.tx;
            }
            let out = match txdecode::decode(&raw) {
                Ok(tx) => match x_ante::aminojson::sign_bytes(&sd, &tx) {
                    Ok(b) => hex::encode(hash::sha256(&b)),
                    Err(e) => format!("sign bytes: {e}"),
                },
                Err(e) => format!("decode: {}", e.log),
            };
            writeln!(all, "{h}:{i}:{out}").unwrap();
        }
    }
    assert!(!sdk::version::is_sdk_v046(0));
    assert_eq!(
        hex::encode(hash::sha256(all.as_bytes())),
        V4_SIGN_BYTES_SHA256,
        "{all}"
    );
}
