//! Conformance tests for ante against committed Go-generated fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use ante::{
    Account, AccountReader, AccountWriter, AnteConfig, AnteContext, AnteHandler, Error, FeeKeeper,
    GasMeter, RunTxMode,
};
use cosmos_types::Coin;
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

#[derive(Debug, Deserialize)]
struct FeesFixture {
    chain_id: String,
    cases: Vec<FeeCase>,
}

#[derive(Debug, Deserialize)]
struct FeeCase {
    label: String,
    tx_raw_hex: String,
    address_hex: String,
    public_key_any_hex: String,
    account_number: u64,
    account_sequence: u64,
    min_gas_prices: Vec<FixtureCoin>,
    balances: Vec<FixtureBalance>,
    expected_ok: bool,
    #[serde(default)]
    expected_error: String,
    #[serde(default)]
    expected_gas_wanted: u64,
    #[serde(default)]
    expected_final_sequence: u64,
    expected_balances: Vec<FixtureBalance>,
}

#[derive(Debug, Deserialize)]
struct FixtureCoin {
    denom: String,
    amount: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureBalance {
    address_hex: String,
    denom: String,
    amount: String,
}

#[derive(Debug, Deserialize)]
struct GasFixture {
    chain_id: String,
    cases: Vec<GasCase>,
}

#[derive(Debug, Deserialize)]
struct GasCase {
    label: String,
    mode: String,
    tx_raw_hex: String,
    address_hex: String,
    public_key_any_hex: String,
    account_number: u64,
    account_sequence: u64,
    expected_ok: bool,
    #[serde(default)]
    expected_error: String,
    #[serde(default)]
    expected_gas_wanted: u64,
    #[serde(default)]
    expected_gas_used: u64,
    #[serde(default)]
    expected_final_sequence: u64,
    #[serde(default)]
    expected_public_key_set: bool,
}

#[derive(Debug, Deserialize)]
struct MalformedFixture {
    chain_id: String,
    cases: Vec<MalformedCase>,
}

#[derive(Debug, Deserialize)]
struct MalformedCase {
    label: String,
    tx_raw_hex: String,
    expected_error: String,
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

#[test]
fn fee_fixtures_match_min_gas_and_deduction_behavior() {
    let fixture: FeesFixture = load_fixture("ante/fees.json");

    for case in &fixture.cases {
        let mut config = AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id);
        config.min_gas_prices = case.min_gas_prices.iter().map(coin_from_fixture).collect();
        let handler = AnteHandler::new(config);
        let tx_bytes = decode_hex(&case.tx_raw_hex, &case.label, "tx raw");
        let address = decode_address(&case.address_hex, &case.label);
        let mut accounts = Accounts::from_account(Account {
            address,
            account_number: case.account_number,
            sequence: case.account_sequence,
            public_key: None,
        });
        let mut fees = Fees::from_fixture_balances(&case.balances);
        let initial_balances = fees.snapshot();

        let result = handler.run(deliver_context(), &tx_bytes, &mut accounts, &mut fees);
        if case.expected_ok {
            let outcome = result.unwrap_or_else(|error| {
                panic!("fee fixture '{}' unexpectedly failed: {error}", case.label)
            });
            assert_eq!(
                outcome.gas_wanted, case.expected_gas_wanted,
                "gas wanted mismatch for '{}'",
                case.label
            );
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
            assert_eq!(
                fees.snapshot(),
                balances_from_fixture(&case.expected_balances),
                "balance mismatch for '{}'",
                case.label
            );
        } else {
            let Err(error) = result else {
                panic!("fee fixture '{}' unexpectedly succeeded", case.label);
            };
            assert_eq!(
                error_code(&error),
                case.expected_error,
                "error mismatch for '{}'",
                case.label
            );
            let account = accounts
                .account(&address)
                .expect("account lookup")
                .expect("account exists after failed ante run");
            assert_eq!(
                account.sequence, case.account_sequence,
                "sequence mutated after failing fixture '{}'",
                case.label
            );
            assert_eq!(
                account.public_key, None,
                "public key mutated after failing fixture '{}'",
                case.label
            );
            assert_eq!(
                fees.snapshot(),
                initial_balances,
                "balances mutated after failing fixture '{}'",
                case.label
            );
        }
    }
}

#[test]
fn gas_fixtures_match_ante_accounting_and_modes() {
    let fixture: GasFixture = load_fixture("ante/gas.json");
    let handler = AnteHandler::new(AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id));

    for case in &fixture.cases {
        let tx_bytes = decode_hex(&case.tx_raw_hex, &case.label, "tx raw");
        let address = decode_address(&case.address_hex, &case.label);
        let mut accounts = Accounts::from_account(Account {
            address,
            account_number: case.account_number,
            sequence: case.account_sequence,
            public_key: None,
        });
        let mut fees = NoFees;

        let result = handler.run(run_context(&case.mode), &tx_bytes, &mut accounts, &mut fees);
        if case.expected_ok {
            let outcome = result.unwrap_or_else(|error| {
                panic!("gas fixture '{}' unexpectedly failed: {error}", case.label)
            });
            assert_eq!(
                outcome.gas_wanted, case.expected_gas_wanted,
                "gas wanted mismatch for '{}'",
                case.label
            );
            assert_eq!(
                outcome.gas_used, case.expected_gas_used,
                "gas used mismatch for '{}'",
                case.label
            );
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
            let expected_public_key = if case.expected_public_key_set {
                Some(decode_any(
                    &case.public_key_any_hex,
                    &case.label,
                    "tx public key",
                ))
            } else {
                None
            };
            assert_eq!(
                account.public_key, expected_public_key,
                "stored public key mismatch for '{}'",
                case.label
            );
        } else {
            let Err(error) = result else {
                panic!("gas fixture '{}' unexpectedly succeeded", case.label);
            };
            assert_eq!(
                error_code(&error),
                case.expected_error,
                "error mismatch for '{}'",
                case.label
            );
            let account = accounts
                .account(&address)
                .expect("account lookup")
                .expect("account exists after failed ante run");
            assert_eq!(
                account.sequence, case.account_sequence,
                "sequence mutated after failing fixture '{}'",
                case.label
            );
        }
    }
}

#[test]
fn malformed_fixtures_match_stateless_error_mapping() {
    let fixture: MalformedFixture = load_fixture("ante/malformed.json");
    let handler = AnteHandler::new(AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id));

    for case in &fixture.cases {
        let tx_bytes = decode_hex(&case.tx_raw_hex, &case.label, "tx raw");
        let Err(error) = handler.validate_tx_stateless(&tx_bytes) else {
            panic!("malformed fixture '{}' unexpectedly succeeded", case.label);
        };
        assert_eq!(
            error_code(&error),
            case.expected_error,
            "error mismatch for '{}'",
            case.label
        );
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

fn coin_from_fixture(coin: &FixtureCoin) -> Coin {
    Coin::new(&coin.denom, &coin.amount).unwrap_or_else(|error| {
        panic!(
            "convert fixture coin {}{}: {error}",
            coin.amount, coin.denom
        )
    })
}

fn balances_from_fixture(
    balances: &[FixtureBalance],
) -> BTreeMap<(cosmos_types::AccountAddress, String), Coin> {
    balances
        .iter()
        .map(|balance| {
            let address = decode_address(&balance.address_hex, "fee balance");
            let coin = Coin::new(&balance.denom, &balance.amount).unwrap_or_else(|error| {
                panic!(
                    "convert fixture balance {}{}: {error}",
                    balance.amount, balance.denom
                )
            });
            ((address, balance.denom.clone()), coin)
        })
        .collect()
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
    run_context("deliver")
}

fn run_context(mode: &str) -> AnteContext {
    let mode = match mode {
        "check" => RunTxMode::Check,
        "recheck" => RunTxMode::Recheck,
        "deliver" => RunTxMode::Deliver,
        "simulate" => RunTxMode::Simulate,
        other => panic!("unknown run tx mode '{other}'"),
    };
    AnteContext {
        mode,
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

#[derive(Debug, Default)]
struct Fees {
    balances: BTreeMap<(cosmos_types::AccountAddress, String), Coin>,
}

impl Fees {
    fn from_fixture_balances(balances: &[FixtureBalance]) -> Self {
        Self {
            balances: balances_from_fixture(balances),
        }
    }

    fn snapshot(&self) -> BTreeMap<(cosmos_types::AccountAddress, String), Coin> {
        self.balances.clone()
    }
}

impl FeeKeeper for Fees {
    fn spendable_balance(
        &self,
        address: &cosmos_types::AccountAddress,
        denom: &str,
    ) -> ante::Result<Option<Coin>> {
        Ok(self.balances.get(&(*address, denom.to_owned())).cloned())
    }

    fn deduct_fees(
        &mut self,
        payer: &cosmos_types::AccountAddress,
        fees: &[Coin],
    ) -> ante::Result<()> {
        for fee in fees {
            let balance = self
                .balances
                .get_mut(&(*payer, fee.denom.clone()))
                .ok_or(Error::InsufficientFunds)?;
            let remaining = parse_amount(&balance.amount) - parse_amount(&fee.amount);
            balance.amount = remaining.to_string();
        }
        Ok(())
    }
}

fn parse_amount(input: &str) -> u128 {
    input.parse().expect("valid integer amount")
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
