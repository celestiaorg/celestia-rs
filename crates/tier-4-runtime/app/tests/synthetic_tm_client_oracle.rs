//! 07-tendermint client blocks per release, as the Go oracles ran them
//! (`-heights 10 -tmclient` in `conformance/v{4,5,6,8,9}-oracle`,
//! `tmclient.go`; fixtures `conformance/fixtures/synthetic/v*-tm`): after
//! each release's usual five blocks, five blocks of MsgCreateClient,
//! MsgUpdateClient and MsgSubmitMisbehaviour for a synthetic tendermint
//! counterparty whose headers each break one rule of header verification.
//! These tests pin Go's outcome per release (code, codespace, log) and the
//! client store changes, the target of the light-client rules (and of the
//! order in which the parallel commit checks report errors).

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic"
);

const RELEASES: [&str; 5] = ["v4", "v5", "v6", "v8", "v9"];

/// The cases, in block order: (block offset, tx index, what it does).
const CASES: [(i64, usize, &str); 32] = [
    (5, 0, "create A"),
    (5, 1, "create E"),
    (5, 2, "create P"),
    (5, 3, "update P to h11"),
    (5, 4, "create M"),
    (5, 5, "create R (revision 0)"),
    (6, 0, "A adjacent to h11"),
    (6, 1, "A h11 again"),
    (6, 2, "E expired"),
    (6, 3, "P to h12, pruning h10"),
    (6, 4, "A non-adjacent to h20, 1/2 trust"),
    (6, 5, "A to h30, 1/4 trust"),
    (6, 6, "R non-adjacent with a nil vote"),
    (7, 0, "A to h31 within drift"),
    (7, 1, "A to h32 beyond drift"),
    (7, 2, "A chain ID cp-9"),
    (7, 3, "non-adjacent: missing slot and bad first signature"),
    (7, 4, "non-adjacent: bad first signature"),
    (7, 5, "non-adjacent: two of four signed"),
    (7, 6, "non-adjacent: validator 0 in two slots"),
    (8, 0, "non-adjacent: bad, good, absent, absent"),
    (8, 1, "header validators hash is not the set's"),
    (8, 2, "trusted validators are not the next validators"),
    (8, 3, "adjacent: missing slot and bad first signature"),
    (8, 4, "adjacent: bad first signature"),
    (8, 5, "adjacent: two of four signed"),
    (8, 6, "adjacent: slot 1 names validator 0"),
    (9, 0, "A conflicting h20: frozen"),
    (9, 1, "A after freeze"),
    (9, 2, "submit misbehaviour on M"),
    (9, 3, "R adjacent with a nil vote"),
    (9, 4, "R adjacent with an absent signature"),
];

/// The tx results of the client blocks of `release`, by (offset, index).
fn results(release: &str) -> Vec<((i64, usize), serde_json::Value)> {
    let dir = format!("{FIXTURES}/{release}-tm");
    let roots: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/store-roots.json")).unwrap()).unwrap();
    let first = roots["handoff_height"].as_i64().unwrap() + 6;
    common::gz_lines(&format!("{dir}/tx-results.jsonl.gz"))
        .filter(|d| d["height"].as_i64().unwrap() >= first)
        .flat_map(|d| {
            let offset = d["height"].as_i64().unwrap() - first + 5;
            d["txs"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, t)| ((offset, i), t.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The first client sequence of the blocks: v6 and later chains already
/// hold the solo machine client `06-solomachine-0`.
fn base(release: &str) -> u64 {
    u64::from(!matches!(release, "v4" | "v5"))
}

/// What Go returns for case `(block offset, tx index)` of `release`: code,
/// codespace and a part of the log.
fn want(release: &str, case: (i64, usize)) -> (u64, &'static str, String) {
    let id = |n: u64| format!("07-tendermint-{}", base(release) + n);
    let verify = "failed to execute message; message index: 0: failed to verify header: ";
    let wrong_sig =
        |idx: usize| format!("invalid header: wrong signature (#{idx}): 436F6D6D69745369677B");
    let ok = (0, "", String::new());
    match case {
        (5, _) | (6, 0 | 1 | 3 | 4 | 6) | (7, 0) | (9, 0 | 2 | 3 | 4) => ok,
        (6, 2) => (
            29,
            "client",
            format!("cannot update client ({}) with status Expired: client state is not active", id(1)),
        ),
        (6, 5) => (
            1,
            "undefined",
            format!("{verify}cant trust new val set: invalid commit -- insufficient voting power: got 10, needed more than 13"),
        ),
        (7, 1) => (1, "undefined", "invalid header: new header has a time from the future".into()),
        (7, 2) => (
            5,
            "07-tendermint",
            "header height revision 9 does not match trusted header revision 1: invalid header height".into(),
        ),
        // The trusting check finds signatures by address: the missing slot
        // is no error there, the bad first signature is.
        (7, 3 | 4) | (8, 0) => (1, "undefined", format!("{verify}wrong signature (#0): 436F6D6D69745369677B")),
        (7 | 8, 5) => (
            1,
            "undefined",
            format!("{verify}invalid header: invalid commit -- insufficient voting power: got 20, needed more than 26"),
        ),
        (7, 6) => (
            1,
            "undefined",
            format!("{verify}double vote from Validator{{6698BE609CDB48337A0F5A72C0FFBC1484E9D7A4 PubKeyEd25519{{"),
        ),
        (8, 1) => (12, "client", "validator set does not match hash: invalid client header".into()),
        (8, 2) => (14, "07-tendermint", ": invalid validator set".into()),
        // By index, the set size is checked before any signature.
        (8, 3) => (1, "undefined", format!("{verify}invalid header: Invalid commit -- wrong set size: 4 vs 3")),
        (8, 4) => (1, "undefined", format!("{verify}{}", wrong_sig(0))),
        // celestia-core 0.39.30 (v8.0.8) and later check the address of a
        // signature found by index; earlier releases verify it against the
        // validator at that index.
        (8, 6) if matches!(release, "v8" | "v9") => (
            1,
            "undefined",
            format!(
                "{verify}invalid header: validator address mismatch at index 1: expected 8BCE656D9CDA26A03C583B5AF64647AEE9F99F5A, got 6698BE609CDB48337A0F5A72C0FFBC1484E9D7A4"
            ),
        ),
        (8, 6) => (1, "undefined", format!("{verify}{}", wrong_sig(1))),
        (9, 1) => (
            29,
            "client",
            format!("cannot update client ({}) with status Frozen: client state is not active", id(0)),
        ),
        _ => unreachable!(),
    }
}

#[test]
fn tm_client_tx_results_per_release() {
    for release in RELEASES {
        let got = results(release);
        assert_eq!(got.len(), CASES.len(), "{release}");
        for (((offset, idx), tx), case) in got.iter().zip(CASES) {
            assert_eq!((*offset, *idx), (case.0, case.1), "{release} {}", case.2);
            let (code, space, log) = want(release, (case.0, case.1));
            let got_log = tx["log"].as_str().unwrap();
            assert_eq!(
                (
                    tx["code"].as_u64().unwrap(),
                    tx["codespace"].as_str().unwrap()
                ),
                (code, space),
                "{release} {}: {got_log}",
                case.2
            );
            assert!(got_log.contains(&log), "{release} {}: {got_log}", case.2);
        }
    }
}

/// The ibc store changes of the client blocks: the update of P in block +6
/// prunes P's expired h10 consensus state (and its processed height, time
/// and iteration key); block +9 writes new client states for A and M (now
/// frozen) and R (updated to h0-17), and none for E, which never updated.
#[test]
fn tm_client_store_changes_per_release() {
    for release in RELEASES {
        let dir = format!("{FIXTURES}/{release}-tm");
        let roots: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{dir}/store-roots.json")).unwrap())
                .unwrap();
        let first = roots["handoff_height"].as_i64().unwrap() + 6;
        let id = |n: u64| format!("clients/07-tendermint-{}", base(release) + n);
        let diffs = common::diffs_in(&format!("{dir}/kv-diffs.jsonl.gz"));
        // A block whose txs all fail (+8) changes no ibc key.
        let ibc = |offset: i64| {
            diffs
                .get(&(first + offset - 5))
                .and_then(|stores| stores.get("ibc"))
                .cloned()
                .unwrap_or_default()
        };

        let p = id(2);
        let deleted: Vec<String> = ibc(6)
            .delete
            .iter()
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect();
        for want in [
            format!("{p}/consensusStates/1-10"),
            format!("{p}/consensusStates/1-10/processedHeight"),
            format!("{p}/consensusStates/1-10/processedTime"),
        ] {
            assert!(
                deleted.contains(&want),
                "{release}: {want} not pruned: {deleted:?}"
            );
        }
        let set9: Vec<String> = ibc(9)
            .set
            .keys()
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .filter(|k| k.ends_with("/clientState"))
            .collect();
        assert_eq!(
            set9,
            [
                format!("{}/clientState", id(0)),
                format!("{}/clientState", id(3)),
                format!("{}/clientState", id(4)),
            ],
            "{release}"
        );
        for offset in 6..=9 {
            assert!(
                !ibc(offset)
                    .set
                    .keys()
                    .any(|k| k.starts_with(id(1).as_bytes())),
                "{release}: E changed at +{offset}"
            );
        }
    }
}
