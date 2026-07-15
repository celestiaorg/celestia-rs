//! API tests for the M3 ante crate.

#![allow(clippy::expect_used)]

use ante::{
    Account, AccountReader, AccountWriter, AnteConfig, AnteContext, AnteHandler, Error, FeeKeeper,
    Gas, GasMeter, ModeInfo, RunTxMode, SignDoc, SignMode, SignerInfo, TxRaw,
};
use cosmos_types::{Any, AuthInfo, Coin, Fee, TxBody};
use prost::Message;
use std::collections::BTreeMap;

#[test]
fn gas_meter_records_out_of_gas_and_overflow() {
    let mut meter = GasMeter::new(5);
    assert_eq!(meter.remaining(), Some(5));
    assert_eq!(meter.consume(3, "read"), Ok(()));
    assert_eq!(meter.remaining(), Some(2));
    assert_eq!(meter.consume(3, "write"), Err(Error::OutOfGas));
    assert_eq!(meter.consumed, 6);
    assert!(meter.is_out_of_gas());
    assert_eq!(meter.remaining(), Some(0));

    let mut infinite = GasMeter::infinite();
    assert_eq!(infinite.consume(Gas::MAX, "large"), Ok(()));
    assert_eq!(infinite.remaining(), None);
    assert!(!infinite.is_out_of_gas());

    let mut overflow = GasMeter {
        limit: Gas::MAX - 1,
        consumed: Gas::MAX,
    };
    assert_eq!(overflow.consume(1, "overflow"), Err(Error::GasOverflow));
}

#[test]
fn stateless_validation_accepts_direct_single_signer_tx() {
    let handler = handler();
    let tx_bytes = tx_bytes(SignMode::Direct, vec![message_any()], "", Vec::new(), 1);
    let tx = handler
        .validate_tx_stateless(&tx_bytes)
        .expect("valid direct tx");

    assert_eq!(tx.signatures, vec![vec![0xab; 64]]);
}

#[test]
fn stateless_validation_rejects_empty_messages_extensions_and_long_memo() {
    let handler = handler();
    let empty_messages = tx_bytes(SignMode::Direct, Vec::new(), "memo", Vec::new(), 1);
    assert_eq!(
        handler.validate_tx_stateless(&empty_messages),
        Err(Error::MalformedTx)
    );

    let extension_options = tx_bytes(
        SignMode::Direct,
        vec![message_any()],
        "",
        vec![message_any()],
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&extension_options),
        Err(Error::MalformedTx)
    );

    let long_memo = "x".repeat(257);
    let long_memo_tx = tx_bytes(
        SignMode::Direct,
        vec![message_any()],
        &long_memo,
        Vec::new(),
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&long_memo_tx),
        Err(Error::InvalidMemo)
    );
}

#[test]
fn stateless_validation_rejects_unsupported_sign_modes() {
    let handler = handler();
    let amino_tx = tx_bytes(
        SignMode::LegacyAminoJson,
        vec![message_any()],
        "",
        Vec::new(),
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&amino_tx),
        Err(Error::UnsupportedSignMode)
    );
}

#[test]
fn deliver_run_verifies_signature_sets_public_key_and_increments_sequence() {
    let handler = handler();
    let (tx_bytes, address, public_key) = signed_tx(7, 9);
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: None,
    });
    let mut fees = NoFees;

    let outcome = handler
        .run(deliver_context(), &tx_bytes, &mut accounts, &mut fees)
        .expect("valid signed tx");

    assert_eq!(outcome.gas_wanted, 500_000);
    assert!(outcome.gas_used > 0);
    assert_eq!(outcome.signer_sequences, vec![(address, 10)]);
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(account.sequence, 10);
    assert_eq!(account.public_key, Some(public_key));
}

#[test]
fn run_rejects_sequence_mismatch_and_bad_signature_without_state_mutation() {
    let handler = handler();
    let (tx_bytes, address, public_key) = signed_tx(7, 9);
    let mut wrong_sequence = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 10,
        public_key: Some(public_key.clone()),
    });
    let mut fees = NoFees;

    assert_eq!(
        handler.run(deliver_context(), &tx_bytes, &mut wrong_sequence, &mut fees),
        Err(Error::SequenceMismatch)
    );
    assert_eq!(
        wrong_sequence
            .account(&address)
            .expect("account lookup")
            .expect("account exists")
            .sequence,
        10
    );

    let mut bad_tx = TxRaw::decode(&tx_bytes).expect("valid tx");
    bad_tx.signatures[0][0] ^= 0xff;
    let bad_tx_bytes = bad_tx.encode_to_vec().expect("valid tx shape");
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: Some(public_key),
    });

    assert_eq!(
        handler.run(deliver_context(), &bad_tx_bytes, &mut accounts, &mut fees),
        Err(Error::SignatureVerification)
    );
    assert_eq!(
        accounts
            .account(&address)
            .expect("account lookup")
            .expect("account exists")
            .sequence,
        9
    );
}

#[test]
fn run_deducts_explicit_payer_fee_after_signature_checks() {
    let mut config = AnteConfig::celestia_mainnet_app_v6("celestia");
    config.min_gas_prices = vec![Coin::new("utia", "1").expect("valid coin")];
    let handler = AnteHandler::new(config);
    let payer = [7u8; cosmos_types::ADDRESS_LENGTH];
    let fee = Fee {
        amount: vec![Coin::new("utia", "600000").expect("valid coin")],
        gas_limit: 500_000,
        payer: cosmos_types::account_address_to_bech32(&payer),
        granter: String::new(),
    };
    let (tx_bytes, address, public_key) = signed_tx_with_fee(7, 9, fee);
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: Some(public_key),
    });
    let mut fees = Fees::from_balance(payer, Coin::new("utia", "700000").expect("valid coin"));

    let outcome = handler
        .run(deliver_context(), &tx_bytes, &mut accounts, &mut fees)
        .expect("valid fee tx");

    assert_eq!(outcome.signer_sequences, vec![(address, 10)]);
    assert_eq!(fees.balance(&payer, "utia"), Some("100000".to_owned()));
}

#[test]
fn run_rejects_insufficient_fee_and_insufficient_funds_before_sequence_mutation() {
    let mut config = AnteConfig::celestia_mainnet_app_v6("celestia");
    config.min_gas_prices = vec![Coin::new("utia", "2").expect("valid coin")];
    let handler = AnteHandler::new(config);
    let fee = Fee {
        amount: vec![Coin::new("utia", "600000").expect("valid coin")],
        gas_limit: 500_000,
        payer: String::new(),
        granter: String::new(),
    };
    let (tx_bytes, address, _public_key) = signed_tx_with_fee(7, 9, fee.clone());
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: None,
    });
    let mut fees = Fees::from_balance(address, Coin::new("utia", "2000000").expect("valid coin"));

    assert_eq!(
        handler.run(deliver_context(), &tx_bytes, &mut accounts, &mut fees),
        Err(Error::InsufficientFee)
    );
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(account.sequence, 9);
    assert_eq!(account.public_key, None);

    let mut config = AnteConfig::celestia_mainnet_app_v6("celestia");
    config.min_gas_prices = vec![Coin::new("utia", "1").expect("valid coin")];
    let handler = AnteHandler::new(config);
    let (tx_bytes, address, _public_key) = signed_tx_with_fee(7, 9, fee);
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: None,
    });
    let mut fees = Fees::from_balance(address, Coin::new("utia", "1").expect("valid coin"));

    assert_eq!(
        handler.run(deliver_context(), &tx_bytes, &mut accounts, &mut fees),
        Err(Error::InsufficientFunds)
    );
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(account.sequence, 9);
    assert_eq!(account.public_key, None);
}

#[test]
fn run_rejects_missing_fee_when_min_gas_price_is_nonzero() {
    let mut config = AnteConfig::celestia_mainnet_app_v6("celestia");
    config.min_gas_prices = vec![Coin::new("utia", "1").expect("valid coin")];
    config.tx_size_cost_per_byte = 0;
    config.consume_signature_verification_gas = false;
    let handler = AnteHandler::new(config);
    let (tx_bytes, address, _public_key) = signed_tx_with_optional_fee(7, 9, None);
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: None,
    });
    let mut fees = NoFees;

    assert_eq!(
        handler.run(deliver_context(), &tx_bytes, &mut accounts, &mut fees),
        Err(Error::InsufficientFee)
    );
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(account.sequence, 9);
    assert_eq!(account.public_key, None);
}

#[test]
fn run_accepts_missing_fee_when_min_gas_prices_are_zero() {
    let mut config = AnteConfig::celestia_mainnet_app_v6("celestia");
    config.min_gas_prices = vec![Coin::new("utia", "0").expect("valid coin")];
    config.tx_size_cost_per_byte = 0;
    config.consume_signature_verification_gas = false;
    let handler = AnteHandler::new(config);
    let (tx_bytes, address, public_key) = signed_tx_with_optional_fee(7, 9, None);
    let mut accounts = Accounts::from_account(Account {
        address,
        account_number: 7,
        sequence: 9,
        public_key: None,
    });
    let mut fees = NoFees;

    let outcome = handler
        .run(deliver_context(), &tx_bytes, &mut accounts, &mut fees)
        .expect("valid missing fee tx");

    assert_eq!(outcome.gas_wanted, 0);
    assert_eq!(outcome.gas_used, 0);
    assert_eq!(outcome.signer_sequences, vec![(address, 10)]);
    let account = accounts
        .account(&address)
        .expect("account lookup")
        .expect("account exists");
    assert_eq!(account.sequence, 10);
    assert_eq!(account.public_key, Some(public_key));
}

fn handler() -> AnteHandler {
    AnteHandler::new(AnteConfig::celestia_mainnet_app_v6("celestia"))
}

fn tx_bytes(
    sign_mode: SignMode,
    messages: Vec<Any>,
    memo: &str,
    extension_options: Vec<Any>,
    sequence: u64,
) -> Vec<u8> {
    TxRaw {
        body_bytes: TxBody {
            messages,
            memo: memo.to_owned(),
            timeout_height: 0,
            extension_options,
            non_critical_extension_options: Vec::new(),
        }
        .to_proto()
        .encode_to_vec(),
        auth_info_bytes: AuthInfo {
            signer_infos: vec![SignerInfo {
                public_key: None,
                mode_info: ModeInfo::Single { mode: sign_mode },
                sequence,
            }],
            fee: Some(Fee {
                amount: Vec::new(),
                gas_limit: 200_000,
                payer: String::new(),
                granter: String::new(),
            }),
            tip: None,
        }
        .to_proto()
        .encode_to_vec(),
        signatures: vec![vec![0xab; 64]],
    }
    .encode_to_vec()
    .expect("valid tx raw")
}

fn signed_tx(account_number: u64, sequence: u64) -> (Vec<u8>, cosmos_types::AccountAddress, Any) {
    signed_tx_with_fee(
        account_number,
        sequence,
        Fee {
            amount: Vec::new(),
            gas_limit: 500_000,
            payer: String::new(),
            granter: String::new(),
        },
    )
}

fn signed_tx_with_fee(
    account_number: u64,
    sequence: u64,
    fee: Fee,
) -> (Vec<u8>, cosmos_types::AccountAddress, Any) {
    signed_tx_with_optional_fee(account_number, sequence, Some(fee))
}

fn signed_tx_with_optional_fee(
    account_number: u64,
    sequence: u64,
    fee: Option<Fee>,
) -> (Vec<u8>, cosmos_types::AccountAddress, Any) {
    let secret_key = crypto::secp256k1::secret_key_from_secret(b"ante signed tx");
    let public_key =
        crypto::secp256k1::public_key_from_secret_key(&secret_key).expect("valid secret key");
    let public_key_any = cosmos_types::secp256k1_public_key_any(&public_key);
    let address = crypto::secp256k1::address(&public_key);
    let body_bytes = TxBody {
        messages: vec![message_any()],
        memo: String::new(),
        timeout_height: 0,
        extension_options: Vec::new(),
        non_critical_extension_options: Vec::new(),
    }
    .to_proto()
    .encode_to_vec();
    let auth_info_bytes = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(public_key_any.clone()),
            mode_info: ModeInfo::Single {
                mode: SignMode::Direct,
            },
            sequence,
        }],
        fee,
        tip: None,
    }
    .to_proto()
    .encode_to_vec();
    let sign_bytes = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: "celestia".to_owned(),
        account_number,
    }
    .sign_bytes()
    .expect("valid sign doc");
    let signature = crypto::secp256k1::sign(&secret_key, &sign_bytes).expect("valid signature");
    let tx_bytes = TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![signature.to_vec()],
    }
    .encode_to_vec()
    .expect("valid signed tx raw");
    (tx_bytes, address, public_key_any)
}

fn deliver_context() -> AnteContext {
    AnteContext {
        mode: RunTxMode::Deliver,
        block_height: 10,
        gas_meter: GasMeter::infinite(),
        events: Vec::new(),
    }
}

fn message_any() -> Any {
    Any {
        type_url: "/test.Msg".to_owned(),
        value: vec![1, 2, 3],
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
        public_key: Any,
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
    fn from_balance(address: cosmos_types::AccountAddress, coin: Coin) -> Self {
        Self {
            balances: BTreeMap::from([((address, coin.denom.clone()), coin)]),
        }
    }

    fn balance(&self, address: &cosmos_types::AccountAddress, denom: &str) -> Option<String> {
        self.balances
            .get(&(*address, denom.to_owned()))
            .map(|coin| coin.amount.clone())
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

/// Every ante error must expose the Cosmos SDK ABCI `(code, codespace)` pair
/// that celestia-app / cosmos-sdk v0.50 returns for the equivalent rejection,
/// so the M3 mainnet corpus runner can compare Rust rejections against Go
/// ABCI result fields.
#[test]
fn error_exposes_cosmos_sdk_abci_code_and_codespace() {
    // Grounded in cosmos-sdk v0.50.10 `types/errors/errors.go` (RootCodespace
    // "sdk") and the `x/auth/ante` decorators that return each error.
    let cases = [
        (Error::MalformedTx, 2u32),            // ErrTxDecode
        (Error::UnsupportedSignMode, 37),      // ErrNotSupported
        (Error::MissingSigner, 15),            // ErrNoSignatures
        (Error::SignatureVerification, 4),     // ErrUnauthorized
        (Error::AccountNotFound, 9),           // ErrUnknownAddress
        (Error::AccountNumberMismatch, 4),     // ErrUnauthorized
        (Error::SequenceMismatch, 32),         // ErrWrongSequence
        (Error::InsufficientFee, 13),          // ErrInsufficientFee
        (Error::InsufficientFunds, 5),         // ErrInsufficientFunds
        (Error::OutOfGas, 11),                 // ErrOutOfGas
        (Error::GasOverflow, 11),              // ErrOutOfGas
        (Error::InvalidMemo, 12),              // ErrMemoTooLarge
        (Error::InvalidPublicKey, 8),          // ErrInvalidPubKey
        (Error::State("boom".to_owned()), 35), // ErrLogic
    ];
    for (error, code) in cases {
        assert_eq!(error.abci_code(), code, "abci_code for {error:?}");
        assert_eq!(error.codespace(), "sdk", "codespace for {error:?}");
    }
}
