//! Probe blocks per release, as the Go oracles ran them (`-heights 19
//! -probes` in `conformance/v{4,5,6,8,9}-oracle`, `probes.go`,
//! `warpprobes.go`, `pfmprobes.go` and `icaprobes.go`; fixtures
//! `conformance/fixtures/synthetic/v*-probes`): after each release's usual
//! five blocks, fourteen blocks of transactions whose outcome differs
//! between releases. Blocks +5 to +7 probe module rules; blocks +8 to +13 set up a
//! hyperlane route and send warp transactions signed in
//! SIGN_MODE_LEGACY_AMINO_JSON; from v6, where the chain has the solo
//! machine's transfer channel, blocks +14 and +15 receive packets that
//! packet-forward-middleware forwards, and blocks +16 to +18 open an
//! interchain-account channel and execute a transaction through it. These tests pin Go's outcome per
//! release, the target of the per-release rules.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic"
);

/// The probe transactions, in block order.
const PROBES: [&str; 15] = [
    "icahost MsgModuleQuerySafe",
    "warp MsgCreateSyntheticToken (unknown mailbox)",
    "MsgWithdrawDelegatorReward without a delegation",
    "gov proposal: icahost MsgUpdateParams",
    "gov proposal: consensus params, evidence 242,640",
    "gov proposal: consensus params, evidence 404,400",
    "gov proposal: consensus params, evidence 559,940",
    "core MsgCreateNoopIsm",
    "core MsgCreateNoopHook",
    "core MsgCreateMailbox",
    "warp MsgCreateCollateralToken (amino)",
    "warp MsgEnrollRemoteRouter, domain 2 (amino)",
    "warp MsgEnrollRemoteRouter, domain 3 (direct)",
    "warp MsgRemoteTransfer to domain 3 (amino)",
    "warp MsgRemoteTransfer to domain 2 (amino)",
];

/// The transactions of the probe blocks of `release`: those after the
/// usual five blocks that follow its hand-off height.
fn probe_txs(release: &str) -> Vec<serde_json::Value> {
    let dir = format!("{FIXTURES}/{release}-probes");
    let roots: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/store-roots.json")).unwrap()).unwrap();
    let first = roots["handoff_height"].as_i64().unwrap() + 6;
    common::gz_lines(&format!("{dir}/tx-results.jsonl.gz"))
        .filter(|d| d["height"].as_i64().unwrap() >= first)
        .flat_map(|d| d["txs"].as_array().unwrap().clone())
        .collect()
}

/// The first fifteen probe transactions of `release` (all but the IBC
/// ones from v6): code, codespace, log and signers.
fn probes(release: &str) -> Vec<(u64, String, String, Option<Vec<String>>)> {
    probe_txs(release)[..PROBES.len()]
        .iter()
        .map(|t| {
            (
                t["code"].as_u64().unwrap(),
                t["codespace"].as_str().unwrap().to_owned(),
                t["log"].as_str().unwrap().to_owned(),
                t.get("signers").map(|s| {
                    s.as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_owned())
                        .collect()
                }),
            )
        })
        .collect()
}

/// The value of attribute `key` of each event of type `kind` in `tx`.
fn attrs(tx: &serde_json::Value, kind: &str, key: &str) -> Vec<String> {
    tx["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == kind)
        .flat_map(|e| e["attributes"].as_array().unwrap().clone())
        .filter(|a| a["key"] == key)
        .map(|a| a["value"].as_str().unwrap().to_owned())
        .collect()
}

fn check(release: &str, want: &[(u64, &str, &str); 15]) {
    let got = probes(release);
    for (i, ((code, space, log, _), (wcode, wspace, wlog))) in
        got.iter().zip(want.iter().copied()).enumerate()
    {
        assert_eq!(
            (*code, space.as_str()),
            (wcode, wspace),
            "{release} {}: {log}",
            PROBES[i]
        );
        assert!(log.ends_with(wlog), "{release} {}: {log}", PROBES[i]);
    }
}

const NO_HANDLER: (u64, &str, &str) = (
    6,
    "sdk",
    "no message handler found for *types.MsgModuleQuerySafe: unknown request",
);
const SYNTHETIC_DISABLED: (u64, &str, &str) = (1, "undefined", "module disabled synthetic tokens");
const UNKNOWN_MAILBOX: (u64, &str, &str) = (
    1,
    "undefined",
    "failed to find mailbox with id: 0x0000000000000000000000000000000000000000000000000000000000000009",
);
const NO_DELEGATION: (u64, &str, &str) = (
    19,
    "staking",
    "no delegation for (address, validator) tuple",
);
const NO_DIST_INFO: (u64, &str, &str) = (5, "distribution", "no delegation distribution info");
const UNROUTABLE: (u64, &str, &str) = (
    10,
    "gov",
    "/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams: proposal message not recognized by router",
);
const OK: (u64, &str, &str) = (0, "", "");
/// Go's sign-bytes handler fails on a RemoteRouter in amino JSON
/// (hyperlane-cosmos v1.0.1), and the SigVerificationDecorator reports
/// that as a bad signature of alice (account 4) at her sequence in that
/// release's chain.
const BAD_SIGNATURE_V4: (u64, &str, &str) = (
    4,
    "sdk",
    "signature verification failed; please verify account number (4), sequence (15) and chain-id (test): unauthorized",
);
const BAD_SIGNATURE_V5: (u64, &str, &str) = (
    4,
    "sdk",
    "signature verification failed; please verify account number (4), sequence (18) and chain-id (test): unauthorized",
);
const BAD_SIGNATURE_V6: (u64, &str, &str) = (
    4,
    "sdk",
    "signature verification failed; please verify account number (4), sequence (23) and chain-id (test): unauthorized",
);
const NO_ROUTER: (u64, &str, &str) = (
    1,
    "undefined",
    "no enrolled router found for destination domain 2",
);
const BAD_EVIDENCE: (u64, &str, &str) = (4, "sdk", "invalid evidence parameters: unauthorized");

/// v4.1.0 and v5.0.12: no ICA host Msg service, synthetic warp tokens
/// disabled, the sdk's ErrNoDelegation, and a consensus param filter that
/// checks only the validator params (any evidence params pass). Amino JSON
/// cannot sign a RemoteRouter, so the amino enrollment fails and the
/// transfer to its domain finds no router.
#[test]
fn v4_and_v5_probes() {
    for (release, bad_signature) in [("v4", BAD_SIGNATURE_V4), ("v5", BAD_SIGNATURE_V5)] {
        check(
            release,
            &[
                NO_HANDLER,
                SYNTHETIC_DISABLED,
                NO_DELEGATION,
                UNROUTABLE,
                OK,
                OK,
                OK,
                OK,
                OK,
                OK,
                OK,
                bad_signature,
                OK,
                OK,
                NO_ROUTER,
            ],
        );
    }
}

/// v6.4.4 and v8.0.8: the ICA host Msg service, synthetic tokens enabled,
/// distribution's no-delegation error, and the filter requires the v6
/// evidence params (MaxAgeNumBlocks 242,640). v6.4.4 still has
/// hyperlane-cosmos v1.0.1 (no amino RemoteRouter); v8.0.8 has v1.1.0, and
/// every warp probe succeeds.
#[test]
fn v6_and_v8_probes() {
    for release in ["v6", "v8"] {
        let (enroll, transfer) = if release == "v6" {
            (BAD_SIGNATURE_V6, NO_ROUTER)
        } else {
            (OK, OK)
        };
        check(
            release,
            &[
                OK,
                UNKNOWN_MAILBOX,
                NO_DIST_INFO,
                OK,
                OK,
                BAD_EVIDENCE,
                BAD_EVIDENCE,
                OK,
                OK,
                OK,
                OK,
                enroll,
                OK,
                OK,
                transfer,
            ],
        );
    }
}

/// v9.0.8: as v8, but the filter requires MaxAgeNumBlocks 559,940.
#[test]
fn v9_probes() {
    check(
        "v9",
        &[
            OK,
            UNKNOWN_MAILBOX,
            NO_DIST_INFO,
            OK,
            BAD_EVIDENCE,
            BAD_EVIDENCE,
            OK,
            OK,
            OK,
            OK,
            OK,
            OK,
            OK,
            OK,
            OK,
        ],
    );
}

/// `ExecTxResult.Signers`: celestia-core v1.57.2 (v4.1.0) has no such
/// field, so the v4 fixture omits it; from v5.0.12 each transaction
/// records its signer.
#[test]
fn signers_are_recorded_from_v5() {
    for (_, _, _, signers) in probes("v4") {
        assert_eq!(signers, None);
    }
    for release in ["v5", "v6", "v8", "v9"] {
        for (i, (_, _, _, signers)) in probes(release).into_iter().enumerate() {
            assert_eq!(signers.unwrap().len(), 1, "{release} {}", PROBES[i]);
        }
    }
}

/// The IBC probe transactions from v6: two packet forwards, then the
/// interchain-account channel try, confirm, funding and execution.
const IBC_PROBES: usize = 6;

/// Packet forwarding (packet-forward-middleware v8.2.0) from v6.4.4, on
/// the solo machine's channel-0. Block +14: the solo machine's packet 3
/// asks to forward 300 uatom back over channel-0; the voucher is received
/// and sent on as packet 2 of channel-0 (`transfer/channel-0/uatom`), and
/// the acknowledgement waits for the forward's. Block +15: packet 4 asks
/// to forward to channel-9, which does not exist, and the receive writes
/// an error acknowledgement at once. v4 and v5 have no channel there.
#[test]
fn packet_forward_from_v6() {
    for release in ["v4", "v5"] {
        assert_eq!(probe_txs(release).len(), PROBES.len(), "{release}");
    }
    for release in ["v6", "v8", "v9"] {
        let txs = probe_txs(release);
        assert_eq!(txs.len(), PROBES.len() + IBC_PROBES, "{release}");
        let (forward, bad) = (&txs[PROBES.len()], &txs[PROBES.len() + 1]);
        for tx in [forward, bad] {
            assert_eq!(tx["code"], 0, "{release}: {}", tx["log"]);
        }
        assert_eq!(
            attrs(forward, "recv_packet", "packet_sequence"),
            ["3"],
            "{release}"
        );
        assert_eq!(
            attrs(forward, "send_packet", "packet_sequence"),
            ["2"],
            "{release}"
        );
        assert_eq!(
            attrs(forward, "send_packet", "packet_src_channel"),
            ["channel-0"],
            "{release}"
        );
        let data: serde_json::Value =
            serde_json::from_str(&attrs(forward, "send_packet", "packet_data")[0]).unwrap();
        assert_eq!(data["denom"], "transfer/channel-0/uatom", "{release}");
        assert_eq!(data["receiver"], "solo-forward-receiver", "{release}");
        assert_eq!(data["amount"], "300", "{release}");
        assert!(
            attrs(forward, "write_acknowledgement", "packet_ack").is_empty(),
            "{release}"
        );

        assert_eq!(
            attrs(bad, "recv_packet", "packet_sequence"),
            ["4"],
            "{release}"
        );
        assert!(
            attrs(bad, "send_packet", "packet_sequence").is_empty(),
            "{release}"
        );
        let ack = &attrs(bad, "write_acknowledgement", "packet_ack")[0];
        assert!(
            ack.starts_with(
                "{\"error\":\"packet-forward-middleware error: port ID (transfer) channel ID (channel-9): channel not found"
            ),
            "{release}: {ack}"
        );
    }
}

/// Interchain-account host from v6.4.4, with the solo machine as
/// controller on its connection. Block +16: `MsgChannelOpenTry` on port
/// icahost opens channel-1 in TRYOPEN and the host registers an account,
/// whose address the returned version names. Block +17:
/// `MsgChannelOpenConfirm` opens it, and alice funds the account. Block
/// +18: an EXECUTE_TX packet runs a `MsgSend` of 1,000 utia from the
/// account to bob and writes a result acknowledgement.
#[test]
fn interchain_account_host_from_v6() {
    for release in ["v6", "v8", "v9"] {
        let txs = probe_txs(release);
        let ica = &txs[PROBES.len() + 2..];
        assert_eq!(ica.len(), 4, "{release}");
        for tx in ica {
            assert_eq!(tx["code"], 0, "{release}: {}", tx["log"]);
        }
        let (try_, confirm, fund, exec) = (&ica[0], &ica[1], &ica[2], &ica[3]);
        assert_eq!(
            attrs(try_, "channel_open_try", "channel_id"),
            ["channel-1"],
            "{release}"
        );
        let version: serde_json::Value =
            serde_json::from_str(&attrs(try_, "channel_open_try", "version")[0]).unwrap();
        assert_eq!(version["version"], "ics27-1", "{release}");
        assert_eq!(
            version["controller_connection_id"], "connection-on-solomachine",
            "{release}"
        );
        assert_eq!(version["host_connection_id"], "connection-0", "{release}");
        // The interchain account's public bech32 address.
        let ica_addr = version["address"].as_str().unwrap();
        assert!(
            ica_addr.starts_with("celestia1"),
            "{release}: the version names no celestia address"
        );
        assert_eq!(
            attrs(confirm, "channel_open_confirm", "channel_id"),
            ["channel-1"],
            "{release}"
        );
        assert!(
            attrs(fund, "transfer", "recipient")
                .iter()
                .any(|r| r == ica_addr),
            "{release}"
        );
        assert_eq!(
            attrs(exec, "ics27_packet", "success"),
            ["true"],
            "{release}"
        );
        assert!(
            attrs(exec, "transfer", "sender")
                .iter()
                .any(|r| r == ica_addr),
            "{release}"
        );
        let ack = &attrs(exec, "write_acknowledgement", "packet_ack")[0];
        let ack: serde_json::Value = serde_json::from_str(ack).unwrap();
        assert!(ack["result"].is_string(), "{release}: {ack}");
    }
}
