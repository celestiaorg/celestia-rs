//! Conformance tests for ante against committed Go-generated fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use ante::{AnteConfig, AnteHandler, Error};
use prost::Message;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct StatelessFixture {
    chain_id: String,
    cases: Vec<StatelessCase>,
}

#[derive(Debug, Deserialize)]
struct StatelessCase {
    label: String,
    tx_raw_hex: String,
    expected_ok: bool,
    #[serde(default)]
    expected_error: String,
    #[serde(default)]
    signer_count: usize,
}

#[test]
fn stateless_fixtures_match_go_tx_shapes() {
    let fixture: StatelessFixture = load_fixture("ante/stateless.json");
    let handler = AnteHandler::new(AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id));

    for case in &fixture.cases {
        let tx_bytes = hex::decode(&case.tx_raw_hex)
            .unwrap_or_else(|error| panic!("decode tx hex for '{}': {error}", case.label));
        let result = handler.validate_tx_stateless(&tx_bytes);
        if case.expected_ok {
            let tx = result.unwrap_or_else(|error| {
                panic!(
                    "stateless fixture '{}' unexpectedly failed: {error}",
                    case.label
                )
            });
            let auth_info =
                cosmos_types::proto::tx::AuthInfo::decode(tx.auth_info_bytes.as_slice())
                    .unwrap_or_else(|error| {
                        panic!("decode auth info for '{}': {error}", case.label)
                    });
            assert_eq!(
                auth_info.signer_infos.len(),
                case.signer_count,
                "signer count mismatch for '{}'",
                case.label
            );
        } else {
            let Err(error) = result else {
                panic!("stateless fixture '{}' unexpectedly succeeded", case.label);
            };
            assert_eq!(
                error_code(&error),
                case.expected_error,
                "error mismatch for '{}'",
                case.label
            );
        }
    }
}

fn load_fixture<T>(relative_path: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let path = workspace_root()
        .join("conformance/fixtures")
        .join(relative_path);
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn workspace_root() -> PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..10 {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let body =
                std::fs::read_to_string(&manifest).expect("read candidate workspace Cargo.toml");
            if body.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    panic!("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR")
}

fn error_code(error: &Error) -> &'static str {
    match error {
        Error::MalformedTx => "malformed_tx",
        Error::UnsupportedSignMode => "unsupported_sign_mode",
        Error::MissingSigner => "missing_signer",
        Error::SignatureVerification => "signature_verification",
        Error::AccountNotFound => "account_not_found",
        Error::AccountNumberMismatch => "account_number_mismatch",
        Error::SequenceMismatch => "sequence_mismatch",
        Error::InsufficientFee => "insufficient_fee",
        Error::InsufficientFunds => "insufficient_funds",
        Error::OutOfGas => "out_of_gas",
        Error::GasOverflow => "gas_overflow",
        Error::InvalidMemo => "invalid_memo",
        Error::InvalidPublicKey => "invalid_public_key",
        Error::State(_) => "state",
    }
}
