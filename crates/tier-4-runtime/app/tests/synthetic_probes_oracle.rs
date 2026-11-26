//! Probe blocks per release, as the Go oracles ran them (`-heights 14
//! -probes` in `conformance/v{4,5,6,8,9}-oracle`, `probes.go` and
//! `warpprobes.go`; fixtures `conformance/fixtures/synthetic/v*-probes`):
//! after each release's usual five blocks, nine blocks of transactions
//! whose outcome differs between releases. Blocks +5 to +7 probe module
//! rules; blocks +8 to +13 set up a hyperlane route and send warp
//! transactions signed in SIGN_MODE_LEGACY_AMINO_JSON. These tests pin Go's
//! outcome per release, the target of the per-release rules.

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

/// The probe transactions of `release` (the last fifteen of its fixture):
/// code, codespace, the end of the log and the signers.
fn probes(release: &str) -> Vec<(u64, String, String, Option<Vec<String>>)> {
    let txs: Vec<serde_json::Value> =
        common::gz_lines(&format!("{FIXTURES}/{release}-probes/tx-results.jsonl.gz"))
            .flat_map(|d| d["txs"].as_array().unwrap().clone())
            .collect();
    txs[txs.len() - PROBES.len()..]
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
