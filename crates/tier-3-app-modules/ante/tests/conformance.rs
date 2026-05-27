//! Conformance tests for ante against committed Go-generated fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use ante::{
    Account, AccountReader, AccountWriter, AnteConfig, AnteContext, AnteHandler, Error, FeeKeeper,
    GasMeter, RunTxMode,
};
use prost::Message;
use serde::Deserialize;
use std::collections::BTreeMap;
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

#[derive(Debug, Deserialize)]
struct SignaturesFixture {
    chain_id: String,
    cases: Vec<SignatureCase>,
}

#[derive(Debug, Deserialize)]
struct SignatureCase {
    label: String,
    tx_raw_hex: String,
    #[serde(default)]
    address_hex: String,
    #[serde(default)]
    public_key_any_hex: String,
    account_present: bool,
    #[serde(default)]
    account_number: u64,
    #[serde(default)]
    account_sequence: u64,
    #[serde(default)]
    account_public_key_any_hex: String,
    expected_ok: bool,
    #[serde(default)]
    expected_error: String,
    #[serde(default)]
    expected_sign_bytes_hex: String,
    #[serde(default)]
    expected_signature_hex: String,
    #[serde(default)]
    expected_final_sequence: u64,
    #[serde(default)]
    expected_final_public_key: bool,
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

#[test]
fn signature_fixtures_match_go_sign_bytes_and_account_checks() {
    let fixture: SignaturesFixture = load_fixture("ante/signatures.json");
    let handler = AnteHandler::new(AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id));

    for case in &fixture.cases {
        let tx_bytes = decode_hex(&case.tx_raw_hex, &case.label, "tx raw");
        let mut accounts = accounts_for_case(case);
        let mut fees = NoFees;

        if case.expected_ok {
            let tx = handler
                .validate_tx_stateless(&tx_bytes)
                .unwrap_or_else(|error| {
                    panic!(
                        "signature fixture '{}' unexpectedly failed stateless validation: {error}",
                        case.label
                    )
                });
            let signers = handler.signer_data(&tx, &accounts).unwrap_or_else(|error| {
                panic!(
                    "signature fixture '{}' unexpectedly failed signer extraction: {error}",
                    case.label
                )
            });
            assert_eq!(
                signers.len(),
                1,
                "signer count mismatch for '{}'",
                case.label
            );
            let signer = &signers[0];
            assert_eq!(
                hex::encode(&signer.sign_bytes),
                case.expected_sign_bytes_hex,
                "sign bytes mismatch for '{}'",
                case.label
            );
            assert_eq!(
                hex::encode(&signer.signature),
                case.expected_signature_hex,
                "signature bytes mismatch for '{}'",
                case.label
            );
            assert_eq!(
                signer.account_number, case.account_number,
                "account number mismatch for '{}'",
                case.label
            );
            assert_eq!(
                signer.sequence, case.account_sequence,
                "account sequence mismatch for '{}'",
                case.label
            );
        }

        let result = handler.run(deliver_context(), &tx_bytes, &mut accounts, &mut fees);
        if case.expected_ok {
            let outcome = result.unwrap_or_else(|error| {
                panic!(
                    "signature fixture '{}' unexpectedly failed: {error}",
                    case.label
                )
            });
            let address = decode_address(&case.address_hex, &case.label);
            assert_eq!(
                outcome.signer_sequences,
                vec![(address, case.expected_final_sequence)],
                "final signer sequence mismatch for '{}'",
                case.label
            );
            let account = accounts
                .account(&address)
                .expect("account lookup")
                .expect("account exists after successful ante run");
            assert_eq!(
                account.sequence, case.expected_final_sequence,
                "stored sequence mismatch for '{}'",
                case.label
            );
            if case.expected_final_public_key {
                assert_eq!(
                    account.public_key,
                    Some(decode_any(
                        &case.public_key_any_hex,
                        &case.label,
                        "tx public key"
                    )),
                    "stored public key mismatch for '{}'",
                    case.label
                );
            }
        } else {
            let Err(error) = result else {
                panic!("signature fixture '{}' unexpectedly succeeded", case.label);
            };
            assert_eq!(
                error_code(&error),
                case.expected_error,
                "error mismatch for '{}'",
                case.label
            );
            assert_account_unchanged(case, &accounts);
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

fn accounts_for_case(case: &SignatureCase) -> Accounts {
    if !case.account_present {
        return Accounts::default();
    }
    let public_key = if case.account_public_key_any_hex.is_empty() {
        None
    } else {
        Some(decode_any(
            &case.account_public_key_any_hex,
            &case.label,
            "account public key",
        ))
    };
    Accounts::from_account(Account {
        address: decode_address(&case.address_hex, &case.label),
        account_number: case.account_number,
        sequence: case.account_sequence,
        public_key,
    })
}

fn assert_account_unchanged(case: &SignatureCase, accounts: &Accounts) {
    if !case.account_present {
        return;
    }
    let address = decode_address(&case.address_hex, &case.label);
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(
        account.sequence, case.account_sequence,
        "sequence mutated after failing fixture '{}'",
        case.label
    );
    let expected_public_key = if case.account_public_key_any_hex.is_empty() {
        None
    } else {
        Some(decode_any(
            &case.account_public_key_any_hex,
            &case.label,
            "account public key",
        ))
    };
    assert_eq!(
        account.public_key, expected_public_key,
        "public key mutated after failing fixture '{}'",
        case.label
    );
}

fn decode_hex(input: &str, label: &str, field: &str) -> Vec<u8> {
    hex::decode(input).unwrap_or_else(|error| panic!("decode {field} hex for '{label}': {error}"))
}

fn decode_address(input: &str, label: &str) -> cosmos_types::AccountAddress {
    let bytes = decode_hex(input, label, "address");
    bytes
        .as_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("address length mismatch for '{label}'"))
}

fn decode_any(input: &str, label: &str, field: &str) -> cosmos_types::Any {
    let bytes = decode_hex(input, label, field);
    let any = cosmos_types::proto::google::Any::decode(bytes.as_slice())
        .unwrap_or_else(|error| panic!("decode {field} protobuf for '{label}': {error}"));
    cosmos_types::Any::from_proto(any)
        .unwrap_or_else(|error| panic!("convert {field} for '{label}': {error}"))
}

fn deliver_context() -> AnteContext {
    AnteContext {
        mode: RunTxMode::Deliver,
        block_height: 10,
        gas_meter: GasMeter::infinite(),
        events: Vec::new(),
    }
}

#[derive(Debug, Default)]
struct Accounts {
    accounts: BTreeMap<cosmos_types::AccountAddress, Account>,
}

impl Accounts {
    fn from_account(account: Account) -> Self {
        Self {
            accounts: BTreeMap::from([(account.address, account)]),
        }
    }
}

impl AccountReader for Accounts {
    fn account(&self, address: &cosmos_types::AccountAddress) -> ante::Result<Option<Account>> {
        Ok(self.accounts.get(address).cloned())
    }
}

impl AccountWriter for Accounts {
    fn set_sequence(
        &mut self,
        address: &cosmos_types::AccountAddress,
        sequence: u64,
    ) -> ante::Result<()> {
        let account = self
            .accounts
            .get_mut(address)
            .ok_or(Error::AccountNotFound)?;
        account.sequence = sequence;
        Ok(())
    }

    fn set_public_key(
        &mut self,
        address: &cosmos_types::AccountAddress,
        public_key: cosmos_types::Any,
    ) -> ante::Result<()> {
        let account = self
            .accounts
            .get_mut(address)
            .ok_or(Error::AccountNotFound)?;
        account.public_key = Some(public_key);
        Ok(())
    }
}

struct NoFees;

impl FeeKeeper for NoFees {
    fn spendable_balance(
        &self,
        _address: &cosmos_types::AccountAddress,
        _denom: &str,
    ) -> ante::Result<Option<cosmos_types::Coin>> {
        Err(Error::State("unexpected balance lookup".to_owned()))
    }

    fn deduct_fees(
        &mut self,
        _payer: &cosmos_types::AccountAddress,
        _fees: &[cosmos_types::Coin],
    ) -> ante::Result<()> {
        Err(Error::State("unexpected fee deduction".to_owned()))
    }
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
