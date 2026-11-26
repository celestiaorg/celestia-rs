//! The IBC synthetic chain across the v3 to v4 upgrade, as the Go oracles
//! ran it: `conformance/v1-oracle -synthetic -chain v3-ibc` (fixture
//! `conformance/fixtures/synthetic/v3-ibc`, app version 3) and
//! `conformance/v4-oracle` (fixture `conformance/fixtures/synthetic/v3-ibc-v4`).
//!
//! At app version 3 a solo machine client, connection and transfer channel
//! are opened, and a transfer and a receive go over the channel. At app
//! version 4 (celestia-app v4.1.0) the capability of that channel is not
//! found: v4.1.0 has no capability module in its module manager, so its
//! in-memory capability index is never initialized. A transfer fails with
//! `channel` code 9 and a receive or acknowledgement with `capability`
//! code 7, as every IBC packet message did on mainnet from 6,680,340 to
//! 6,748,821 (`conformance/mainnet-usage-v4-v9.md`). These tests pin that
//! target for the Rust v4 rules.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

const V3_IBC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v3-ibc"
);
const V3_IBC_V4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v3-ibc-v4"
);
/// The last block of celestia-app v3.13.0.
const HANDOFF: i64 = 14;

fn results(dir: &str) -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(&format!("{dir}/tx-results.jsonl.gz"))
        .map(|d| (d["height"].as_i64().unwrap(), d))
        .collect()
}

fn json(path: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The v3 side opens the channel and uses it: every transaction succeeds,
/// and the v4 side starts from its last block.
#[test]
fn v3_ibc_chain_opens_and_uses_a_channel() {
    let v3 = json(&format!("{V3_IBC}/store-roots.json"));
    let v4 = json(&format!("{V3_IBC_V4}/store-roots.json"));
    assert_eq!(v4["handoff_height"], HANDOFF);
    assert_eq!(v4["celestia_app"], "v4.1.0");
    let v3_last = v3["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["height"] == HANDOFF)
        .unwrap();
    assert_eq!(v4["heights"][0]["app_hash"], v3_last["app_hash"]);
    let mut actions = Vec::new();
    for (h, r) in results(V3_IBC) {
        for tx in r["txs"].as_array().unwrap() {
            assert_eq!(tx["code"].as_u64().unwrap_or(0), 0, "{h}: {}", tx["log"]);
            let types: Vec<&str> = tx["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["type"].as_str().unwrap())
                .collect();
            for a in [
                "create_client",
                "connection_open_init",
                "connection_open_ack",
                "channel_open_init",
                "channel_open_ack",
                "send_packet",
                "write_acknowledgement",
            ] {
                if types.contains(&a) {
                    actions.push((h, a));
                }
            }
        }
    }
    assert_eq!(
        actions,
        [
            (2, "create_client"),
            (3, "connection_open_init"),
            (5, "connection_open_ack"),
            (6, "channel_open_init"),
            (7, "channel_open_ack"),
            (8, "send_packet"),
            (9, "write_acknowledgement"),
        ]
    );
}

/// At app version 4 the channel's capability is not found: the transfer
/// of block 15 fails with `channel` code 9, the receive (16) and the
/// acknowledgement (17) with `capability` code 7; plain sends succeed.
#[test]
fn v4_ibc_packets_fail_with_capability_not_found() {
    let r = results(V3_IBC_V4);
    let outcome = |h: i64| {
        let tx = &r[&h]["txs"][0];
        (
            tx["code"].as_u64().unwrap(),
            tx["codespace"].as_str().unwrap().to_owned(),
            tx["log"].as_str().unwrap().to_owned(),
        )
    };
    let (code, space, log) = outcome(15);
    assert_eq!((code, space.as_str()), (9, "channel"));
    assert!(
        log.ends_with("module does not own channel capability: channel capability not found"),
        "{log}"
    );
    for h in [16, 17] {
        let (code, space, log) = outcome(h);
        assert_eq!((code, space.as_str()), (7, "capability"), "{h}");
        assert!(
            log.ends_with("capabilities/ports/transfer/channels/channel-0: capability not found"),
            "{h}: {log}"
        );
    }
    for h in [18, 19] {
        assert_eq!(outcome(h).0, 0, "{h}");
    }
}
