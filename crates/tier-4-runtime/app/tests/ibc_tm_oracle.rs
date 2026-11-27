//! 07-tendermint client updates at app versions 1 to 3 against the Go
//! oracle (`conformance/v1-oracle` `TestIBCTMFixture`, fixture
//! `conformance/fixtures/synthetic/ibc-v2-tm`).
//!
//! celestia-app v3.13.0 (ibc-go v6, celestia-core v1) is chain A of the
//! ibc-v2 harness. It crosses the v1 to v2 upgrade at block 2, then
//! (one tx per block, 5 seconds apart) creates clients of a synthetic
//! counterparty whose headers each break one rule, as the `-tmclient`
//! blocks of `v4-oracle` to `v9-oracle` do for later releases:
//! adjacent and non-adjacent trust, expiry, clock drift, a wrong chain
//! ID, pruning (block 13), a revision 0 counterparty with nil votes and
//! absent signatures, the order in which the commit checks report errors
//! on the trusting (by address) and adjacent (by index) paths, a
//! misbehaviour in `MsgUpdateClient` (a decode error in ibc-go v6), a
//! conflicting header that freezes the client (block 34), and
//! `MsgSubmitMisbehaviour` (block 36) and its repeat on the frozen client.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("ibc-v2-tm")
}

/// The fixture files are the ones the oracle wrote.
#[test]
fn ibc_tm_fixture_files_match_their_record() {
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
        r["kv"]["kv-diffs.jsonl"],
        sha(&f.gunzip("kv-diffs.jsonl.gz"))
    );
    assert_eq!(f.last_height(), 38);
}

/// What ibc-go v6 returns for the tx of each height from 4: code,
/// codespace and a part of the log.
fn want(height: i64) -> (u64, &'static str, &'static str) {
    let ok = (0, "", "");
    match height {
        4..=11 | 13 | 14 | 16 | 17 | 31 | 32 | 34 | 36 => ok,
        12 => (
            29,
            "client",
            "cannot update client (07-tendermint-1) with status Expired: client is not active",
        ),
        15 => (
            1,
            "undefined",
            "failed to verify header: cant trust new val set: invalid commit -- insufficient voting power: got 10, needed more than 13",
        ),
        18 => (
            1,
            "undefined",
            "invalid header: new header has a time from the future",
        ),
        19 => (
            5,
            "tendermint-client",
            "header height revision 9 does not match trusted header revision 1: invalid header height",
        ),
        // The trusting path finds signatures by address: the bad first
        // signature, not the missing slot. In ibc-go v6 the log shows the
        // signature bytes.
        20 | 21 | 24 => (
            1,
            "undefined",
            "failed to verify header: wrong signature (#0): ",
        ),
        22 | 29 => (
            1,
            "undefined",
            "failed to verify header: invalid header: invalid commit -- insufficient voting power: got 20, needed more than 26",
        ),
        23 => (
            1,
            "undefined",
            "failed to verify header: double vote from Validator{6698BE609CDB48337A0F5A72C0FFBC1484E9D7A4 PubKeyEd25519{",
        ),
        25 => (
            12,
            "client",
            "validator set does not match hash: invalid client header",
        ),
        26 => (14, "tendermint-client", ": invalid validator set"),
        // By index, the set size is checked before any signature.
        27 => (
            1,
            "undefined",
            "failed to verify header: invalid header: Invalid commit -- wrong set size: 4 vs 3",
        ),
        28 => (
            1,
            "undefined",
            "failed to verify header: invalid header: wrong signature (#0): ",
        ),
        // celestia-core v1 does not check the address of a signature
        // found by index.
        30 => (
            1,
            "undefined",
            "failed to verify header: invalid header: wrong signature (#1): A14C82AA",
        ),
        33 => (
            2,
            "sdk",
            "no concrete type registered for type URL /ibc.lightclients.tendermint.v1.Misbehaviour against interface *exported.Header: tx parse error",
        ),
        35 => (
            29,
            "client",
            "cannot update client (07-tendermint-0) with status Frozen: client is not active",
        ),
        37 => (
            29,
            "client",
            "cannot process misbehaviour for client (07-tendermint-3) with status Frozen: client is not active",
        ),
        38 => (
            29,
            "client",
            "cannot update client (07-tendermint-3) with status Frozen: client is not active",
        ),
        _ => unreachable!("height {height}"),
    }
}

/// Go's result for each client tx, and the store changes of pruning
/// (block 13 deletes client P's expired h10 consensus state) and freezing
/// (blocks 34 and 36 write the frozen client states of A and M).
#[test]
fn ibc_tm_tx_results_and_store_changes() {
    let f = fix();
    for (height, block) in f.results() {
        let txs = block["txs"].as_array().unwrap();
        if height < 4 {
            assert!(txs.is_empty(), "{height}");
            continue;
        }
        assert_eq!(txs.len(), 1, "{height}");
        let tx = &txs[0];
        let (code, space, log) = want(height);
        let got = tx["log"].as_str().unwrap();
        assert_eq!(
            (
                tx["code"].as_u64().unwrap(),
                tx["codespace"].as_str().unwrap()
            ),
            (code, space),
            "{height}: {got}"
        );
        assert!(got.contains(log), "{height}: {got}");
    }

    let diffs = f.diffs();
    let ibc = |h: i64| {
        diffs
            .get(&h)
            .and_then(|s| s.get("ibc"))
            .cloned()
            .unwrap_or_default()
    };
    let deleted: Vec<String> = ibc(13)
        .delete
        .iter()
        .map(|k| String::from_utf8_lossy(k).into_owned())
        .collect();
    assert!(
        deleted.contains(&"clients/07-tendermint-2/consensusStates/1-10".to_owned()),
        "{deleted:?}"
    );
    for (h, client) in [(34, "07-tendermint-0"), (36, "07-tendermint-3")] {
        assert!(
            ibc(h)
                .set
                .contains_key(format!("clients/{client}/clientState").as_bytes()),
            "{h}: {client} not frozen"
        );
    }
}

/// celestia-rs executes the blocks as Go does: every KV change, app hash
/// and transaction result. Two logs from the shared light client still
/// differ (code, codespace and gas match): the double-vote text at 23
/// (celestia-core v1 prints the `Validator`) and the wrong-signature
/// text at 30 (celestia-core v1 does not check the address of a
/// signature found by index).
#[test]
fn ibc_tm_blocks_match_the_oracle() {
    let f = fix();
    f.run_blocks_except_logs(f.last_height(), &[23, 30]);
}
