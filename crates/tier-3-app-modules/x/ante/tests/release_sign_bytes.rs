//! Sign bytes per release, against the Go sign mode handlers of celestia-app
//! v4.1.0, v5.0.12, v6.4.4, v8.0.8 and v9.0.8 (`go run . -signbytes` in
//! `conformance/v{4,5,6,8,9}-oracle`, fixtures
//! `conformance/fixtures/signbytes/v*.json.gz`).
//!
//! For a corpus of the messages mainnet signs (bank, staking,
//! distribution, slashing, gov v1 and v1beta1, authz with inner messages,
//! feegrant, vesting, blob, signal, IBC transfer and core, hyperlane warp and
//! core), each vector holds the encoded tx, the signer data, the sign mode
//! (`SIGN_MODE_LEGACY_AMINO_JSON`, `SIGN_MODE_DIRECT_AUX` with and without an
//! explicit fee payer, and `SIGN_MODE_TEXTUAL`) and Go's sign bytes, or that
//! Go returned an error. Rust must return the same bytes, or an error.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read as _;

use celestia_proto::Any;
use prost::Message as _;
use x_ante::signing::{
    SIGN_MODE_DIRECT_AUX, SIGN_MODE_LEGACY_AMINO_JSON, SIGN_MODE_TEXTUAL, SignerData,
    get_sign_bytes,
};

fn vectors(release: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../../../conformance/fixtures/signbytes/{release}.json.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut out = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
        .read_to_string(&mut out)
        .unwrap();
    serde_json::from_str(&out).unwrap()
}

fn mode(name: &str) -> i32 {
    match name {
        "SIGN_MODE_LEGACY_AMINO_JSON" => SIGN_MODE_LEGACY_AMINO_JSON,
        "SIGN_MODE_DIRECT_AUX" => SIGN_MODE_DIRECT_AUX,
        "SIGN_MODE_TEXTUAL" => SIGN_MODE_TEXTUAL,
        other => panic!("mode {other}"),
    }
}

/// Known divergences from Go, as `(release, vector name, sign mode)`:
/// Go's x/tx amino-JSON encoder fails on these and celestia-rs signs. They
/// are rule gaps (owned by the rules workstream), not oracle errors. The
/// test asserts this list exactly: a new divergence fails it, and so does a
/// fix that leaves its entry here (delete the entry with the fix).
///
/// - `MsgEnrollRemoteRouter` in v4.1.0 to v6.4.4 (hyperlane-cosmos
///   v1.0.1): `hyperlane.warp.v1.RemoteRouter: {TagNum: 1, WireType:"varint"}:
///   unknown protobuf field`.
/// - `MsgCreateVestingAccount` with a start time in v4.1.0:
///   `cosmos.vesting.v1beta1.MsgCreateVestingAccount: {TagNum: 6, ...}:
///   unknown protobuf field` (the fork's `start_time`).
const KNOWN_DIVERGENCES: &[(&str, &str, &str)] = &[
    (
        "v4",
        "warp/MsgEnrollRemoteRouter",
        "SIGN_MODE_LEGACY_AMINO_JSON",
    ),
    (
        "v4",
        "vesting/MsgCreateVestingAccount",
        "SIGN_MODE_LEGACY_AMINO_JSON",
    ),
    (
        "v5",
        "warp/MsgEnrollRemoteRouter",
        "SIGN_MODE_LEGACY_AMINO_JSON",
    ),
    (
        "v6",
        "warp/MsgEnrollRemoteRouter",
        "SIGN_MODE_LEGACY_AMINO_JSON",
    ),
];

/// Runs every vector of `release` at `app_version` and requires the vectors
/// whose outcome differs from Go to be exactly the release's
/// [`KNOWN_DIVERGENCES`].
fn check(release: &str, app_version: u64, celestia_app: &str) {
    sdk::version::set_app_version(app_version);
    let file = vectors(release);
    assert_eq!(file["celestia_app"], celestia_app);
    let chain_id = file["chain_id"].as_str().unwrap();
    let mut failures = Vec::new();
    let mut diverged = Vec::new();
    let mut checked = 0;
    let known: Vec<(&str, &str)> = KNOWN_DIVERGENCES
        .iter()
        .filter(|(r, _, _)| *r == release)
        .map(|(_, n, m)| (*n, *m))
        .collect();
    for v in file["vectors"].as_array().unwrap() {
        let key = (v["name"].as_str().unwrap(), v["mode"].as_str().unwrap());
        let name = format!("{} {}", key.0, key.1);
        let before = failures.len();
        let raw = hex::decode(v["tx"].as_str().unwrap()).unwrap();
        let tx = match txdecode::decode_for(app_version, &raw) {
            Ok(tx) => tx,
            Err(e) => {
                failures.push(format!("{name}: decode: {}", e.log));
                continue;
            }
        };
        let sd = SignerData {
            address: v["address"].as_str().unwrap().into(),
            chain_id: chain_id.into(),
            account_number: v["account_number"].as_u64().unwrap(),
            sequence: v["sequence"].as_u64().unwrap(),
            pub_key: Any::decode(
                hex::decode(v["pub_key"].as_str().unwrap())
                    .unwrap()
                    .as_slice(),
            )
            .unwrap(),
        };
        let got = match get_sign_bytes(mode(v["mode"].as_str().unwrap()), &sd, &tx) {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("{e:?}")),
        };
        let go_error = v["error"].as_str().unwrap();
        match (&got, go_error.is_empty()) {
            (Ok(bytes), true) => {
                let want = hex::decode(v["sign_bytes"].as_str().unwrap()).unwrap();
                if *bytes != want {
                    failures.push(format!(
                        "{name}: sign bytes\n  rs {}\n  go {}",
                        String::from_utf8_lossy(bytes),
                        String::from_utf8_lossy(&want)
                    ));
                }
            }
            (Err(_), false) => {}
            (Ok(_), false) => failures.push(format!("{name}: rs signs, go errors: {go_error}")),
            (Err(e), true) => failures.push(format!("{name}: rs errors ({e}), go signs")),
        }
        if failures.len() > before && known.contains(&key) {
            failures.pop();
            diverged.push(key);
        }
        checked += 1;
    }
    assert!(checked > 100, "{release}: {checked} vectors");
    assert!(failures.is_empty(), "{release}:\n{}", failures.join("\n"));
    let fixed: Vec<_> = known.iter().filter(|k| !diverged.contains(k)).collect();
    assert!(
        fixed.is_empty(),
        "{release}: {fixed:?} now match Go; delete them from KNOWN_DIVERGENCES"
    );
}

#[test]
fn v4_sign_bytes_match_go() {
    check("v4", 4, "v4.1.0");
}

#[test]
fn v5_sign_bytes_match_go() {
    check("v5", 5, "v5.0.12");
}

#[test]
fn v6_sign_bytes_match_go() {
    check("v6", 6, "v6.4.4");
}

#[test]
fn v8_sign_bytes_match_go() {
    check("v8", 8, "v8.0.8");
}

#[test]
fn v9_sign_bytes_match_go() {
    check("v9", 9, "v9.0.8");
}
