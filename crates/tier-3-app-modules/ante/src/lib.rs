//! Cosmos SDK transaction ante machinery for celestia-rs.
//!
//! See `docs/superpowers/specs/m3/ante.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use base64::Engine as _;
use prost::Message;

pub use cosmos_types::tx::{AuthInfo, Fee, ModeInfo, SignDoc, SignMode, SignerInfo, Tx, TxRaw};

/// Gas units consumed by ante checks.
pub type Gas = u64;

const INFINITE_GAS_LIMIT: Gas = Gas::MAX;
const DEFAULT_TX_SIZE_COST_PER_BYTE: Gas = 10;
const DEFAULT_SIG_VERIFY_COST_SECP256K1: Gas = 1000;
const DEFAULT_MAX_MEMO_CHARACTERS: usize = 256;
const SECP256K1_PUBLIC_KEY_TYPE_URL: &str = "/cosmos.crypto.secp256k1.PubKey";
const FEE_COLLECTOR_MODULE_NAME: &[u8] = b"fee_collector";

/// Cosmos SDK transaction execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTxMode {
    /// Mempool admission.
    Check,
    /// Mempool recheck after commit.
    Recheck,
    /// Block execution.
    Deliver,
    /// Simulation for gas estimation.
    Simulate,
}

/// Ante-handler configuration values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteConfig {
    /// Chain ID used for `SIGN_MODE_DIRECT` sign docs.
    pub chain_id: String,
    /// Minimum gas prices accepted by this ante handler.
    pub min_gas_prices: Vec<cosmos_types::Coin>,
    /// Gas charged per encoded transaction byte.
    pub tx_size_cost_per_byte: Gas,
    /// Gas charged for one secp256k1 signature verification.
    pub sig_verify_cost_secp256k1: Gas,
    /// Maximum memo length in Unicode scalar values.
    pub max_memo_characters: usize,
    /// Whether signature verification consumes gas.
    pub consume_signature_verification_gas: bool,
}

impl AnteConfig {
    /// Return the current app-v6 Celestia mainnet ante defaults used by M3.
    pub fn celestia_mainnet_app_v6(chain_id: impl Into<String>) -> Self {
        Self {
            chain_id: chain_id.into(),
            min_gas_prices: Vec::new(),
            tx_size_cost_per_byte: DEFAULT_TX_SIZE_COST_PER_BYTE,
            sig_verify_cost_secp256k1: DEFAULT_SIG_VERIFY_COST_SECP256K1,
            max_memo_characters: DEFAULT_MAX_MEMO_CHARACTERS,
            consume_signature_verification_gas: true,
        }
    }
}

/// Minimal account data required by the ante handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Account address.
    pub address: cosmos_types::AccountAddress,
    /// Account number used in sign docs.
    pub account_number: u64,
    /// Current account sequence.
    pub sequence: u64,
    /// Optional account public key.
    pub public_key: Option<cosmos_types::Any>,
}

/// Read-only account lookup used by signer extraction.
pub trait AccountReader {
    /// Return account data for `address`, or `None` when it does not exist.
    fn account(&self, address: &cosmos_types::AccountAddress) -> Result<Option<Account>>;
}

/// Mutable account operations required by successful ante execution.
pub trait AccountWriter: AccountReader {
    /// Set an account sequence.
    fn set_sequence(&mut self, address: &cosmos_types::AccountAddress, sequence: u64)
    -> Result<()>;

    /// Persist an account public key.
    fn set_public_key(
        &mut self,
        address: &cosmos_types::AccountAddress,
        public_key: cosmos_types::Any,
    ) -> Result<()>;
}

/// Fee and balance operations required by ante execution.
pub trait FeeKeeper {
    /// Return spendable balance for `address` and `denom`.
    fn spendable_balance(
        &self,
        address: &cosmos_types::AccountAddress,
        denom: &str,
    ) -> Result<Option<cosmos_types::Coin>>;

    /// Deduct fees from `payer`.
    fn deduct_fees(
        &mut self,
        payer: &cosmos_types::AccountAddress,
        fees: &[cosmos_types::Coin],
    ) -> Result<()>;
}

/// Gas meter with Cosmos-style out-of-gas recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasMeter {
    /// Gas limit. `u64::MAX` represents an infinite meter.
    pub limit: Gas,
    /// Total gas consumed so far.
    pub consumed: Gas,
}

impl GasMeter {
    /// Create a finite gas meter.
    #[must_use]
    pub const fn new(limit: Gas) -> Self {
        Self { limit, consumed: 0 }
    }

    /// Create an infinite gas meter.
    #[must_use]
    pub const fn infinite() -> Self {
        Self {
            limit: INFINITE_GAS_LIMIT,
            consumed: 0,
        }
    }

    /// Consume gas or return a typed gas error.
    pub fn consume(&mut self, amount: Gas, _descriptor: &'static str) -> Result<()> {
        let consumed = self
            .consumed
            .checked_add(amount)
            .ok_or(Error::GasOverflow)?;
        self.consumed = consumed;
        if self.limit != INFINITE_GAS_LIMIT && self.consumed > self.limit {
            return Err(Error::OutOfGas);
        }
        Ok(())
    }

    /// Return remaining gas for finite meters, or `None` for infinite meters.
    #[must_use]
    pub fn remaining(&self) -> Option<Gas> {
        if self.limit == INFINITE_GAS_LIMIT {
            None
        } else {
            Some(self.limit.saturating_sub(self.consumed))
        }
    }

    /// Return true when this finite meter has consumed more than its limit.
    #[must_use]
    pub const fn is_out_of_gas(&self) -> bool {
        self.limit != INFINITE_GAS_LIMIT && self.consumed > self.limit
    }
}

/// Runtime context passed through ante execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteContext {
    /// Transaction execution mode.
    pub mode: RunTxMode,
    /// Current block height.
    pub block_height: i64,
    /// Gas meter for this transaction.
    pub gas_meter: GasMeter,
    /// Events emitted so far.
    pub events: Vec<abci_types::Event>,
}

/// Per-signer data assembled for signature verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerData {
    /// Signer account address.
    pub address: cosmos_types::AccountAddress,
    /// Account number used in the sign doc.
    pub account_number: u64,
    /// Account sequence.
    pub sequence: u64,
    /// Public key used for verification.
    pub public_key: cosmos_types::Any,
    /// Sign mode used by this signer.
    pub mode: SignMode,
    /// Bytes signed by the signer.
    pub sign_bytes: Vec<u8>,
    /// Raw signature bytes.
    pub signature: Vec<u8>,
}

/// Successful ante execution summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteOutcome {
    /// Gas wanted by the transaction.
    pub gas_wanted: Gas,
    /// Gas consumed by ante execution.
    pub gas_used: Gas,
    /// Events emitted by ante execution.
    pub events: Vec<abci_types::Event>,
    /// Final signer sequences after successful execution.
    pub signer_sequences: Vec<(cosmos_types::AccountAddress, u64)>,
}

/// Cosmos SDK ante handler.
#[derive(Debug, Clone)]
pub struct AnteHandler {
    config: AnteConfig,
}

impl AnteHandler {
    /// Create an ante handler from configuration.
    #[must_use]
    pub const fn new(config: AnteConfig) -> Self {
        Self { config }
    }

    /// Decode and perform stateless validation on raw transaction bytes.
    pub fn validate_tx_stateless(&self, tx_bytes: &[u8]) -> Result<TxRaw> {
        let tx = TxRaw::decode(tx_bytes).map_err(map_cosmos_error)?;
        let body = decode_tx_body(&tx)?;
        if body.messages.is_empty()
            || !body.extension_options.is_empty()
            || !body.non_critical_extension_options.is_empty()
        {
            return Err(Error::MalformedTx);
        }
        if body.memo.chars().count() > self.config.max_memo_characters {
            return Err(Error::InvalidMemo);
        }

        let auth_info = decode_auth_info(&tx)?;
        for signer_info in &auth_info.signer_infos {
            direct_sign_mode(&signer_info.mode_info)?;
        }
        Ok(tx)
    }

    /// Build per-signer verification data.
    pub fn signer_data<A: AccountReader>(
        &self,
        tx: &TxRaw,
        accounts: &A,
    ) -> Result<Vec<SignerData>> {
        let auth_info = decode_auth_info(tx)?;
        let mut out = Vec::with_capacity(auth_info.signer_infos.len());
        for (signer_info, signature) in auth_info.signer_infos.iter().zip(&tx.signatures) {
            let mode = direct_sign_mode(&signer_info.mode_info)?;
            let public_key = signer_info.public_key.clone().ok_or(Error::MissingSigner)?;
            let public_key_bytes = secp256k1_public_key_bytes(&public_key)?;
            let address = crypto::secp256k1::address(&public_key_bytes);
            let account = accounts.account(&address)?.ok_or(Error::AccountNotFound)?;
            if account.sequence != signer_info.sequence {
                return Err(Error::SequenceMismatch);
            }
            if let Some(existing_public_key) = &account.public_key {
                if existing_public_key != &public_key {
                    return Err(Error::InvalidPublicKey);
                }
            }
            let sign_bytes = SignDoc {
                body_bytes: tx.body_bytes.clone(),
                auth_info_bytes: tx.auth_info_bytes.clone(),
                chain_id: self.config.chain_id.clone(),
                account_number: account.account_number,
            }
            .sign_bytes()
            .map_err(map_cosmos_error)?;

            out.push(SignerData {
                address,
                account_number: account.account_number,
                sequence: account.sequence,
                public_key,
                mode,
                sign_bytes,
                signature: signature.clone(),
            });
        }
        Ok(out)
    }

    /// Run the ante handler against account and fee state.
    pub fn run<A, F>(
        &self,
        mut ctx: AnteContext,
        tx_bytes: &[u8],
        accounts: &mut A,
        fees: &mut F,
    ) -> Result<AnteOutcome>
    where
        A: AccountWriter,
        F: FeeKeeper,
    {
        let tx = self.validate_tx_stateless(tx_bytes)?;
        let auth_info = decode_auth_info(&tx)?;
        let fee = auth_info.fee.as_ref();
        let gas_wanted = fee.map_or(0, |fee| fee.gas_limit);
        // The runtime normally enters ante with an empty meter. Fixture-backed
        // state adapters may pre-consume measured keeper gas on the supplied
        // context because their minimal trait boundary intentionally hides the
        // underlying KV operations. Preserve that consumption while applying
        // the transaction's finite gas limit.
        ctx.gas_meter.limit = gas_wanted;
        self.consume_tx_size_gas(&mut ctx.gas_meter, tx_bytes.len())?;

        let signers = self.signer_data(&tx, accounts)?;
        for signer in &signers {
            if self.config.consume_signature_verification_gas {
                ctx.gas_meter.consume(
                    self.config.sig_verify_cost_secp256k1,
                    "secp256k1 signature verification",
                )?;
            }
            verify_signer(signer)?;
        }

        if ctx.mode != RunTxMode::Simulate {
            match fee {
                Some(fee) => self.validate_fees(fee, &signers, fees)?,
                None => self.validate_missing_fee()?,
            }
            if ctx.mode == RunTxMode::Deliver {
                for signer in &signers {
                    accounts.set_public_key(&signer.address, signer.public_key.clone())?;
                }
            }
            if ctx.mode == RunTxMode::Deliver {
                for signer in &signers {
                    accounts.set_sequence(&signer.address, signer.sequence + 1)?;
                }
            }
            if let Some(fee) = fee {
                deduct_fees(fee, &signers, fees)?;
                append_fee_events(&mut ctx.events, fee, &signers)?;
            }
        }

        append_signer_events(&mut ctx.events, &signers);

        let signer_sequences = signers
            .iter()
            .map(|signer| {
                let sequence = if ctx.mode == RunTxMode::Deliver {
                    signer.sequence + 1
                } else {
                    signer.sequence
                };
                (signer.address, sequence)
            })
            .collect();

        Ok(AnteOutcome {
            gas_wanted,
            gas_used: ctx.gas_meter.consumed,
            events: ctx.events,
            signer_sequences,
        })
    }

    fn consume_tx_size_gas(&self, meter: &mut GasMeter, tx_size: usize) -> Result<()> {
        let tx_size = Gas::try_from(tx_size).map_err(|_| Error::GasOverflow)?;
        let gas = tx_size
            .checked_mul(self.config.tx_size_cost_per_byte)
            .ok_or(Error::GasOverflow)?;
        meter.consume(gas, "tx size")
    }

    fn validate_fees<F: FeeKeeper>(
        &self,
        fee: &Fee,
        signers: &[SignerData],
        fees: &F,
    ) -> Result<()> {
        self.validate_minimum_fees(&fee.amount, fee.gas_limit)?;
        let payer = fee_payer(fee, signers)?;
        if fee.amount.is_empty() {
            return Ok(());
        }
        for coin in &fee.amount {
            let Some(balance) = fees.spendable_balance(&payer, &coin.denom)? else {
                return Err(Error::InsufficientFunds);
            };
            if parse_coin_amount(&balance.amount)? < parse_coin_amount(&coin.amount)? {
                return Err(Error::InsufficientFunds);
            }
        }
        Ok(())
    }

    fn validate_minimum_fees(&self, fees: &[cosmos_types::Coin], gas_limit: Gas) -> Result<()> {
        for min_price in &self.config.min_gas_prices {
            let required = parse_coin_amount(&min_price.amount)?
                .checked_mul(u128::from(gas_limit))
                .ok_or(Error::GasOverflow)?;
            let paid = fees
                .iter()
                .find(|coin| coin.denom == min_price.denom)
                .map(|coin| parse_coin_amount(&coin.amount))
                .transpose()?
                .unwrap_or(0);
            if paid < required {
                return Err(Error::InsufficientFee);
            }
        }
        Ok(())
    }

    fn validate_missing_fee(&self) -> Result<()> {
        for min_price in &self.config.min_gas_prices {
            if parse_coin_amount(&min_price.amount)? != 0 {
                return Err(Error::InsufficientFee);
            }
        }
        Ok(())
    }
}

fn deduct_fees<F: FeeKeeper>(fee: &Fee, signers: &[SignerData], fees: &mut F) -> Result<()> {
    if fee.amount.is_empty() {
        return Ok(());
    }
    let payer = fee_payer(fee, signers)?;
    fees.deduct_fees(&payer, &fee.amount)
}

fn append_fee_events(
    events: &mut Vec<abci_types::Event>,
    fee: &Fee,
    signers: &[SignerData],
) -> Result<()> {
    let payer = fee_payer(fee, signers)?;
    let payer = cosmos_types::account_address_to_bech32(&payer);
    let amount = coins_string(&fee.amount);

    if !fee.amount.is_empty() {
        let collector = cosmos_types::account_address_to_bech32(&fee_collector_address());
        events.extend([
            event(
                "coin_spent",
                [("spender", payer.as_str()), ("amount", amount.as_str())],
            ),
            event(
                "coin_received",
                [
                    ("receiver", collector.as_str()),
                    ("amount", amount.as_str()),
                ],
            ),
            event(
                "transfer",
                [
                    ("recipient", collector.as_str()),
                    ("sender", payer.as_str()),
                    ("amount", amount.as_str()),
                ],
            ),
            event("message", [("sender", payer.as_str())]),
        ]);
    }

    events.push(event(
        "tx",
        [("fee", amount.as_str()), ("fee_payer", payer.as_str())],
    ));
    Ok(())
}

fn fee_collector_address() -> cosmos_types::AccountAddress {
    let digest = hash::sha256(FEE_COLLECTOR_MODULE_NAME);
    let mut address = [0_u8; cosmos_types::ADDRESS_LENGTH];
    address.copy_from_slice(&digest[..cosmos_types::ADDRESS_LENGTH]);
    address
}

fn append_signer_events(events: &mut Vec<abci_types::Event>, signers: &[SignerData]) {
    for signer in signers {
        let address = cosmos_types::account_address_to_bech32(&signer.address);
        let sequence = format!("{address}/{}", signer.sequence);
        let signature = base64::engine::general_purpose::STANDARD.encode(&signer.signature);
        events.extend([
            event("tx", [("acc_seq", sequence.as_str())]),
            event("tx", [("signature", signature.as_str())]),
        ]);
    }
}

fn coins_string(coins: &[cosmos_types::Coin]) -> String {
    let mut coins = coins.iter().collect::<Vec<_>>();
    coins.sort_by(|left, right| left.denom.cmp(&right.denom));
    coins
        .into_iter()
        .map(|coin| format!("{}{}", coin.amount, coin.denom))
        .collect::<Vec<_>>()
        .join(",")
}

fn event<const N: usize>(ty: &str, attributes: [(&str, &str); N]) -> abci_types::Event {
    abci_types::Event {
        ty: ty.to_owned(),
        attributes: attributes
            .into_iter()
            .map(|(key, value)| abci_types::EventAttribute {
                key: key.to_owned(),
                value: value.to_owned(),
                index: false,
            })
            .collect(),
    }
}

fn fee_payer(fee: &Fee, signers: &[SignerData]) -> Result<cosmos_types::AccountAddress> {
    if fee.payer.is_empty() {
        signers
            .first()
            .map(|signer| signer.address)
            .ok_or(Error::MissingSigner)
    } else {
        cosmos_types::account_address_from_bech32(&fee.payer).map_err(|_| Error::MalformedTx)
    }
}

/// Errors returned by ante validation and execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Transaction bytes or decoded fields were malformed.
    #[error("malformed transaction")]
    MalformedTx,
    /// The transaction uses a sign mode this crate does not yet support.
    #[error("unsupported sign mode")]
    UnsupportedSignMode,
    /// Required signer data was missing.
    #[error("missing signer")]
    MissingSigner,
    /// Signature verification failed.
    #[error("signature verification failed")]
    SignatureVerification,
    /// A signer account was not found.
    #[error("account not found")]
    AccountNotFound,
    /// Sign-doc account number did not match account state.
    #[error("account number mismatch")]
    AccountNumberMismatch,
    /// Signer sequence did not match account state.
    #[error("sequence mismatch")]
    SequenceMismatch,
    /// Fee amount did not satisfy configured minimum gas prices.
    #[error("insufficient fee")]
    InsufficientFee,
    /// Fee payer did not have enough spendable balance.
    #[error("insufficient funds")]
    InsufficientFunds,
    /// Gas consumption exceeded a finite gas limit.
    #[error("out of gas")]
    OutOfGas,
    /// Gas arithmetic overflowed.
    #[error("gas overflow")]
    GasOverflow,
    /// Memo length exceeded configuration.
    #[error("invalid memo")]
    InvalidMemo,
    /// Public key bytes or type URL were invalid.
    #[error("invalid public key")]
    InvalidPublicKey,
    /// State adapter returned an error.
    #[error("state error: {0}")]
    State(String),
}

/// Cosmos SDK root error codespace (`sdk`) shared by every registered
/// `types/errors` code.
const SDK_CODESPACE: &str = "sdk";

impl Error {
    /// Cosmos SDK ABCI result code for this rejection.
    ///
    /// Codes are grounded in cosmos-sdk v0.50.10 `types/errors/errors.go` and
    /// the `x/auth/ante` decorator that returns each error, so the M3 mainnet
    /// corpus runner can compare Rust rejections against Go ABCI result fields.
    #[must_use]
    pub const fn abci_code(&self) -> u32 {
        match self {
            // ErrTxDecode: transaction bytes or fields could not be parsed.
            Self::MalformedTx => 2,
            // ErrNotSupported: sign mode celestia-app does not accept.
            Self::UnsupportedSignMode => 37,
            // ErrNoSignatures: signer/signature data was missing.
            Self::MissingSigner => 15,
            // ErrUnauthorized: signature or account-number check failed
            // (`SigVerificationDecorator`).
            Self::SignatureVerification | Self::AccountNumberMismatch => 4,
            // ErrUnknownAddress: signer account does not exist.
            Self::AccountNotFound => 9,
            // ErrWrongSequence: signer sequence did not match account state.
            Self::SequenceMismatch => 32,
            // ErrInsufficientFee: fee did not satisfy minimum gas prices.
            Self::InsufficientFee => 13,
            // ErrInsufficientFunds: fee payer could not cover the fee.
            Self::InsufficientFunds => 5,
            // ErrOutOfGas: gas limit exceeded or gas arithmetic overflowed.
            Self::OutOfGas | Self::GasOverflow => 11,
            // ErrMemoTooLarge: memo exceeded the configured maximum.
            Self::InvalidMemo => 12,
            // ErrInvalidPubKey: public key bytes or type URL were invalid.
            Self::InvalidPublicKey => 8,
            // ErrLogic: internal state-adapter failure with no direct Go path.
            Self::State(_) => 35,
        }
    }

    /// Cosmos SDK codespace for this rejection.
    ///
    /// Every ante rejection maps onto a `types/errors` code, so the codespace
    /// is always the SDK root codespace `sdk`.
    #[must_use]
    pub const fn codespace(&self) -> &'static str {
        SDK_CODESPACE
    }

    /// Stable `snake_case` error code for this rejection.
    ///
    /// This is the canonical `expected.error` identifier the M3 mainnet corpus
    /// records for rejected transactions, so both the fixture runner and the
    /// Go corpus generator agree on a single spelling per variant.
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::MalformedTx => "malformed_tx",
            Self::UnsupportedSignMode => "unsupported_sign_mode",
            Self::MissingSigner => "missing_signer",
            Self::SignatureVerification => "signature_verification",
            Self::AccountNotFound => "account_not_found",
            Self::AccountNumberMismatch => "account_number_mismatch",
            Self::SequenceMismatch => "sequence_mismatch",
            Self::InsufficientFee => "insufficient_fee",
            Self::InsufficientFunds => "insufficient_funds",
            Self::OutOfGas => "out_of_gas",
            Self::GasOverflow => "gas_overflow",
            Self::InvalidMemo => "invalid_memo",
            Self::InvalidPublicKey => "invalid_public_key",
            Self::State(_) => "state",
        }
    }
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn decode_tx_body(tx: &TxRaw) -> Result<cosmos_types::TxBody> {
    let proto = cosmos_types::proto::tx::TxBody::decode(tx.body_bytes.as_slice())
        .map_err(|_| Error::MalformedTx)?;
    cosmos_types::TxBody::from_proto(proto).map_err(map_cosmos_error)
}

fn decode_auth_info(tx: &TxRaw) -> Result<AuthInfo> {
    let proto = cosmos_types::proto::tx::AuthInfo::decode(tx.auth_info_bytes.as_slice())
        .map_err(|_| Error::MalformedTx)?;
    AuthInfo::from_proto(proto).map_err(map_cosmos_error)
}

fn direct_sign_mode(mode_info: &ModeInfo) -> Result<SignMode> {
    match mode_info {
        ModeInfo::Single {
            mode: SignMode::Direct,
        } => Ok(SignMode::Direct),
        ModeInfo::Single { .. } | ModeInfo::Multi { .. } => Err(Error::UnsupportedSignMode),
    }
}

fn secp256k1_public_key_bytes(
    any: &cosmos_types::Any,
) -> Result<crypto::secp256k1::PublicKeyBytes> {
    if any.type_url != SECP256K1_PUBLIC_KEY_TYPE_URL {
        return Err(Error::InvalidPublicKey);
    }
    let public_key = cosmos_types::proto::crypto::secp256k1::PubKey::decode(any.value.as_slice())
        .map_err(|_| Error::InvalidPublicKey)?;
    public_key
        .key
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidPublicKey)
}

fn verify_signer(signer: &SignerData) -> Result<()> {
    let public_key = secp256k1_public_key_bytes(&signer.public_key)?;
    let signature: crypto::secp256k1::SignatureBytes = signer
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| Error::SignatureVerification)?;
    if crypto::secp256k1::verify(&public_key, &signer.sign_bytes, &signature) {
        Ok(())
    } else {
        Err(Error::SignatureVerification)
    }
}

fn parse_coin_amount(input: &str) -> Result<u128> {
    input.parse().map_err(|_| Error::InsufficientFee)
}

fn map_cosmos_error(error: cosmos_types::Error) -> Error {
    match error {
        cosmos_types::Error::InvalidSignMode => Error::UnsupportedSignMode,
        cosmos_types::Error::InvalidPublicKey => Error::InvalidPublicKey,
        cosmos_types::Error::InvalidTx
        | cosmos_types::Error::InvalidSignDoc
        | cosmos_types::Error::InvalidSignerInfo
        | cosmos_types::Error::InvalidModeInfo
        | cosmos_types::Error::InvalidFee
        | cosmos_types::Error::InvalidCoin
        | cosmos_types::Error::InvalidDenom
        | cosmos_types::Error::InvalidAmount
        | cosmos_types::Error::InvalidAny
        | cosmos_types::Error::InvalidAddress
        | cosmos_types::Error::InvalidAddressLength { .. } => Error::MalformedTx,
        cosmos_types::Error::InvalidAddressPrefix { actual, .. }
        | cosmos_types::Error::Decode(actual)
        | cosmos_types::Error::Encode(actual) => {
            drop(actual);
            Error::MalformedTx
        }
    }
}
