//! `x/fibre` Msg server (`keeper/msg_server.go`), stateful promise
//! validation (`keeper/keeper.go`) and `ValidateBasic` (`types/msgs.go`).

use celestia_proto::celestia::fibre::v1::{
    EventDepositToEscrow, EventPayForFibre, EventPaymentPromiseTimeout, EventUpdateFibreParams,
    EventWithdrawFromEscrowRequest, MsgDepositToEscrow, MsgDepositToEscrowResponse, MsgPayForFibre,
    MsgPayForFibreResponse, MsgPaymentPromiseTimeout, MsgPaymentPromiseTimeoutResponse,
    MsgRequestWithdrawal, MsgRequestWithdrawalResponse, MsgUpdateFibreParams,
    MsgUpdateFibreParamsResponse, Params, PaymentPromise, ProcessedPayment, Withdrawal,
};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use pbjson_types::Timestamp;
use sdk::address::{ACC_PREFIX, acc_from_bech32, to_bech32};
use sdk::coins::{Amount, Coin, Coins};
use sdk::error::sdk as errs;
use sdk::error::{go_panic, internal};
use sdk::event::typed_event;
use sdk::time::{add, after, before, duration_nanos};
use sdk::{Context, Result, SdkError};

use crate::promise::{self, ED25519_SIGNATURE_SIZE, SECP256K1_PUBKEY_SIZE};
use crate::{
    MAX_PROMISE_CLOCK_SKEW_NANOS, MODULE_NAME, STORE_KEY, get_escrow_account, get_params,
    get_promise_freshness_floor, is_payment_processed_by_hash, normalize_coin, set_escrow_account,
    set_params, set_processed_payment, set_withdrawal, ts_or_zero,
};

/// `appconsts.BondDenom`.
pub const BOND_DENOM: &str = "utia";
/// `appconsts.MaxFibreValidatorSignatures`.
pub const MAX_FIBRE_VALIDATOR_SIGNATURES: usize = 1_000;
/// `appconsts.PFBFibreGasFixedCost`.
pub const PFB_FIBRE_GAS_FIXED_COST: u64 = 650_000;
/// `appconsts.PFBFibreGasPerChunk`.
pub const PFB_FIBRE_GAS_PER_CHUNK: u64 = 45_000;
/// `appconsts.PFBFibreChunkSize`.
pub const PFB_FIBRE_CHUNK_SIZE: u32 = 262_144;
/// `authtypes.FeeCollectorName`.
pub const FEE_COLLECTOR: &str = "fee_collector";

const MINUTE_NANOS: i128 = 60 * 1_000_000_000;
const HOUR_NANOS: i128 = 60 * MINUTE_NANOS;
/// `MinPaymentPromiseTimeout`.
const MIN_PAYMENT_PROMISE_TIMEOUT: i128 = 10 * MINUTE_NANOS;
/// `MaxPaymentPromiseTimeout`.
const MAX_PAYMENT_PROMISE_TIMEOUT: i128 = 12 * HOUR_NANOS;
/// `MinWithdrawalDelay` = `MaxPaymentPromiseTimeout + MinTimeoutSettlementWindow`.
const MIN_WITHDRAWAL_DELAY: i128 = MAX_PAYMENT_PROMISE_TIMEOUT + 10 * MINUTE_NANOS;
/// `MaxWithdrawalDelay`.
const MAX_WITHDRAWAL_DELAY: i128 = 7 * 24 * HOUR_NANOS;
/// `MinShardRetention`.
const MIN_SHARD_RETENTION: i128 = 10 * MINUTE_NANOS;
/// `MaxShardRetention`.
const MAX_SHARD_RETENTION: i128 = 7 * 24 * HOUR_NANOS;

/// `EstimateGasForPayForFibre`.
#[must_use]
pub fn estimate_gas_for_pay_for_fibre(blob_size: u32) -> u64 {
    if blob_size == 0 {
        return PFB_FIBRE_GAS_FIXED_COST;
    }
    let chunks = (u64::from(blob_size) + u64::from(PFB_FIBRE_CHUNK_SIZE) - 1)
        / u64::from(PFB_FIBRE_CHUNK_SIZE);
    PFB_FIBRE_GAS_FIXED_COST + PFB_FIBRE_GAS_PER_CHUNK * chunks
}

/// `PaymentAmount`.
#[must_use]
pub fn payment_amount(blob_size: u32) -> Coin {
    Coin::new(BOND_DENOM, estimate_gas_for_pay_for_fibre(blob_size))
}

/// The gov module address, the fibre authority.
#[must_use]
pub fn authority() -> String {
    to_bech32(ACC_PREFIX, &sdk::address::module_address("gov"))
}

fn coin(c: Option<&PbCoin>) -> Result<Coin> {
    Coin::from_proto(&normalize_coin(c.cloned()))
}

/// Go `sdk.Coin` arithmetic panics on mismatched denominations.
fn same_denom(a: &Coin, b: &Coin) -> Result<()> {
    if a.denom == b.denom {
        Ok(())
    } else {
        Err(go_panic(format!(
            "invalid coin denominations; {}, {}",
            a.denom, b.denom
        )))
    }
}

fn coin_add(a: &Coin, b: &Coin) -> Result<Coin> {
    same_denom(a, b)?;
    let amt = a
        .amount
        .checked_add(&b.amount)
        .map_err(|e| go_panic(e.to_string()))?;
    Ok(Coin::with_amount(a.denom.clone(), amt))
}

fn coin_sub(a: &Coin, b: &Coin) -> Result<Coin> {
    same_denom(a, b)?;
    let amt = a
        .amount
        .checked_sub(&b.amount)
        .map_err(|e| go_panic(e.to_string()))?;
    if amt.is_negative() {
        return Err(go_panic(format!("negative coin amount: {amt}")));
    }
    Ok(Coin::with_amount(a.denom.clone(), amt))
}

fn coin_lt(a: &Coin, b: &Coin) -> Result<bool> {
    same_denom(a, b)?;
    Ok(a.amount < b.amount)
}

fn coins(c: &Coin) -> Result<Coins> {
    Coins::sanitized(vec![c.clone()])
}

fn emit<T: prost::Name + serde::Serialize>(ctx: &mut Context<'_>, ev: &T) -> Result<()> {
    let e = typed_event(ev).map_err(|e| errs::json_marshal(e.to_string()))?;
    ctx.events.emit(e);
    Ok(())
}

// ---- ValidateBasic ----

fn validate_coin_positive(c: Option<&PbCoin>) -> Result<()> {
    // A nil amount fails `IsValid`; `Coin.String()` prints it `<nil>`.
    if c.is_none_or(|c| c.amount.is_empty()) {
        return Err(errs::invalid_coins(sdk::coins::raw_coins_string(&[c
            .cloned()
            .unwrap_or_default()])));
    }
    let c = coin(c)?;
    if !c.is_valid() {
        return Err(errs::invalid_coins(c.to_string()));
    }
    if c.amount.is_zero() || c.amount.is_negative() {
        return Err(errs::invalid_coins("amount must be positive"));
    }
    Ok(())
}

fn validate_signer(signer: &str) -> Result<()> {
    acc_from_bech32(signer)
        .map(|_| ())
        .map_err(|e| errs::invalid_address(format!("invalid signer address: {}", e.log)))
}

/// `MsgDepositToEscrow.ValidateBasic` / `MsgRequestWithdrawal.ValidateBasic`.
pub fn validate_basic_escrow(signer: &str, amount: Option<&PbCoin>) -> Result<()> {
    validate_signer(signer)?;
    validate_coin_positive(amount)
}

/// go-square v4 `share.NewNamespaceFromBytes` (29 bytes) with Go's texts.
fn namespace_from_bytes(bz: &[u8]) -> core::result::Result<shares::Namespace, String> {
    shares::Namespace::from_slice_go(bz)
}

/// `share.Namespace.ValidateForBlob` (go-square v4) with Go's texts.
fn validate_for_blob(ns: shares::Namespace) -> core::result::Result<(), String> {
    if ns.is_parity_shares() || ns.is_tail_padding() {
        return Err(format!(
            "invalid data namespace({}): parity and tail padding namespace are forbidden",
            hex::encode(ns.as_bytes())
        ));
    }
    if ns.is_reserved() {
        return Err(format!(
            "invalid data namespace({}): reserved data is forbidden",
            hex::encode(ns.as_bytes())
        ));
    }
    if ns.version() != 0 {
        return Err(format!("blob version {} is not supported", ns.version()));
    }
    Ok(())
}

/// `PaymentPromise.ValidateBasic`.
pub fn validate_basic_promise(p: &PaymentPromise) -> Result<()> {
    let req = |s: String| errs::invalid_request(s);
    if p.chain_id.is_empty() {
        return Err(req("chain ID cannot be empty".into()));
    }
    if p.namespace.is_empty() {
        return Err(req("namespace cannot be empty".into()));
    }
    if p.namespace.len() != shares::NAMESPACE_SIZE {
        return Err(req(format!(
            "namespace must be {} bytes, got {}",
            shares::NAMESPACE_SIZE,
            p.namespace.len()
        )));
    }
    let ns =
        namespace_from_bytes(&p.namespace).map_err(|e| req(format!("invalid namespace: {e}")))?;
    validate_for_blob(ns).map_err(|e| req(format!("invalid blob namespace: {e}")))?;
    if p.blob_size == 0 {
        return Err(req("blob size must be positive".into()));
    }
    if p.commitment.len() != 32 {
        return Err(req(format!(
            "commitment must be 32 bytes, got {}",
            p.commitment.len()
        )));
    }
    if p.blob_version != 0 {
        // `validateBlobVersion` wraps `ErrInvalidRequest` itself.
        return Err(req(format!(
            "invalid blob version: unsupported blob version: {}: invalid request",
            p.blob_version
        )));
    }
    if p.height <= 0 {
        return Err(req("height must be positive".into()));
    }
    if sdk::time::is_zero(&ts_or_zero(p.creation_timestamp)) {
        return Err(req("creation timestamp cannot be zero".into()));
    }
    let key = promise::signer_key(p);
    if key.len() != SECP256K1_PUBKEY_SIZE {
        return Err(errs::invalid_pub_key(format!(
            "signer public key must be {SECP256K1_PUBKEY_SIZE} bytes, got {}",
            key.len()
        )));
    }
    if p.signature.is_empty() {
        return Err(req("signature cannot be empty".into()));
    }
    Ok(())
}

fn promise_of(p: Option<&PaymentPromise>) -> PaymentPromise {
    p.cloned().unwrap_or_default()
}

/// `MsgPayForFibre.ValidateBasic`.
pub fn validate_basic_pay_for_fibre(m: &MsgPayForFibre) -> Result<()> {
    validate_signer(&m.signer)?;
    validate_basic_promise(&promise_of(m.payment_promise.as_ref()))
        .map_err(|e| e.wrap("invalid payment promise"))?;
    if m.validator_signatures.is_empty() {
        return Err(errs::invalid_request(
            "must have at least one validator signature",
        ));
    }
    if m.validator_signatures.len() > MAX_FIBRE_VALIDATOR_SIGNATURES {
        return Err(errs::invalid_request(format!(
            "too many validator signatures: {} exceeds maximum {MAX_FIBRE_VALIDATOR_SIGNATURES}",
            m.validator_signatures.len()
        )));
    }
    for (i, s) in m.validator_signatures.iter().enumerate() {
        if !s.is_empty() && s.len() != ED25519_SIGNATURE_SIZE {
            return Err(errs::invalid_request(format!(
                "validator signature {i} must be empty or {ED25519_SIGNATURE_SIZE} bytes, got {}",
                s.len()
            )));
        }
    }
    Ok(())
}

/// `MsgPaymentPromiseTimeout.ValidateBasic`.
pub fn validate_basic_timeout(m: &MsgPaymentPromiseTimeout) -> Result<()> {
    validate_signer(&m.signer)?;
    validate_basic_promise(&promise_of(m.payment_promise.as_ref()))
        .map_err(|e| e.wrap("invalid payment promise"))
}

fn dur(d: Option<&pbjson_types::Duration>) -> i128 {
    d.map_or(0, duration_nanos)
}

/// `Params.Validate`.
pub fn validate_params(p: &Params) -> core::result::Result<(), String> {
    let wd = dur(p.withdrawal_delay.as_ref());
    if wd < MIN_WITHDRAWAL_DELAY {
        return Err(format!(
            "withdrawal delay must be at least {}: {}",
            sdk::time::go_duration_string(MIN_WITHDRAWAL_DELAY),
            sdk::time::go_duration_string(wd)
        ));
    }
    if wd > MAX_WITHDRAWAL_DELAY {
        return Err(format!(
            "withdrawal delay must be at most {}: {}",
            sdk::time::go_duration_string(MAX_WITHDRAWAL_DELAY),
            sdk::time::go_duration_string(wd)
        ));
    }
    let t = dur(p.payment_promise_timeout.as_ref());
    if t < MIN_PAYMENT_PROMISE_TIMEOUT {
        return Err(format!(
            "payment promise timeout must be at least {}: {}",
            sdk::time::go_duration_string(MIN_PAYMENT_PROMISE_TIMEOUT),
            sdk::time::go_duration_string(t)
        ));
    }
    if t > MAX_PAYMENT_PROMISE_TIMEOUT {
        return Err(format!(
            "payment promise timeout must be at most {}: {}",
            sdk::time::go_duration_string(MAX_PAYMENT_PROMISE_TIMEOUT),
            sdk::time::go_duration_string(t)
        ));
    }
    if p.payment_promise_height_window == 0 {
        return Err("payment promise height window cannot be 0".into());
    }
    let r = dur(p.shard_retention.as_ref());
    if r < MIN_SHARD_RETENTION {
        return Err(format!(
            "shard retention must be at least {}: {}",
            sdk::time::go_duration_string(MIN_SHARD_RETENTION),
            sdk::time::go_duration_string(r)
        ));
    }
    if r > MAX_SHARD_RETENTION {
        return Err(format!(
            "shard retention must be at most {}: {}",
            sdk::time::go_duration_string(MAX_SHARD_RETENTION),
            sdk::time::go_duration_string(r)
        ));
    }
    if p.full_stake_storage_budget == 0 {
        return Err("full stake storage budget must be positive; disable the limiter locally with the server --unlimited-budget flag instead".into());
    }
    Ok(())
}

/// `MsgUpdateFibreParams.ValidateBasic`.
pub fn validate_basic_update_params(m: &MsgUpdateFibreParams) -> Result<()> {
    acc_from_bech32(&m.authority)
        .map_err(|e| errs::invalid_address(format!("invalid authority address: {}", e.log)))?;
    validate_params(&m.params.unwrap_or_default())
        .map_err(|e| internal(format!("invalid params: {e}")))
}

// ---- stateful validation ----

/// `validatePaymentPromiseStatefulInternal`: the expiration time on
/// success, a plain (unregistered) error message otherwise.
pub fn validate_payment_promise_stateful_internal(
    ctx: &mut Context<'_>,
    p: &PaymentPromise,
    is_timeout: bool,
) -> Result<core::result::Result<Timestamp, String>> {
    if p.chain_id != ctx.header.chain_id {
        return Ok(Err(format!(
            "payment promise chain_id {} does not match executing chain {}",
            gostr::quote(&p.chain_id),
            gostr::quote(&ctx.header.chain_id)
        )));
    }
    let params = get_params(ctx)?;
    let now = ctx.header.time;
    let creation = ts_or_zero(p.creation_timestamp);
    let mut min_allowed = add(&now, -dur(params.withdrawal_delay.as_ref()));
    let floor = match get_promise_freshness_floor(ctx) {
        Ok(f) => f,
        Err(e) => return Ok(Err(e.log)),
    };
    if after(&floor, &min_allowed) {
        min_allowed = floor;
    }
    if !after(&creation, &min_allowed) {
        return Ok(Err(format!(
            "creation_timestamp {} is too old; it must be after the freshness cutoff {}",
            sdk::time::go_time_string(&creation),
            sdk::time::go_time_string(&min_allowed)
        )));
    }
    let max_allowed = add(&now, MAX_PROMISE_CLOCK_SKEW_NANOS);
    if after(&creation, &max_allowed) {
        return Ok(Err(format!(
            "creation_timestamp {} must not be after {} (current_time + max_clock_skew)",
            sdk::time::go_time_string(&creation),
            sdk::time::go_time_string(&max_allowed)
        )));
    }
    let expiration = add(&creation, dur(params.payment_promise_timeout.as_ref()));
    if !is_timeout {
        if !before(&now, &expiration) {
            return Ok(Err(format!(
                "payment promise expired: creation_timestamp {} + timeout {} = {}, current_time: {}",
                sdk::time::go_time_string(&creation),
                sdk::time::go_duration_string(dur(params.payment_promise_timeout.as_ref())),
                sdk::time::go_time_string(&expiration),
                sdk::time::go_time_string(&now)
            )));
        }
        let height = ctx.header.height;
        let window = params.payment_promise_height_window;
        #[allow(clippy::cast_possible_wrap)]
        if height.wrapping_sub(p.height) > window as i64 {
            return Ok(Err(format!(
                "payment promise height {} is too far in the past (current height: {height}, max window: {window})",
                p.height
            )));
        }
        if p.height > height.wrapping_add(1) {
            return Ok(Err(format!(
                "payment promise height {} is too far in the future (current height: {height}, max allowed: {})",
                p.height,
                height + 1
            )));
        }
    }
    // IsPaymentPromiseProcessed: an unhashable promise reads as unprocessed.
    if let Ok(h) = promise::promise_hash(p)
        && is_payment_processed_by_hash(ctx, &h)?
    {
        return Ok(Err("payment promise has already been processed".into()));
    }
    let signer = escrow_signer(p);
    let Some(escrow) = get_escrow_account(ctx, &signer)? else {
        return Ok(Err(format!("escrow account not found for signer {signer}")));
    };
    let required = payment_amount(p.blob_size);
    let balance = coin(escrow.balance.as_ref())?;
    // IsGTE panics on mismatched denominations.
    if coin_lt(&balance, &required)? {
        return Ok(Err(format!(
            "insufficient balance in escrow account. required: {required}, balance: {balance}"
        )));
    }
    Ok(Ok(expiration))
}

/// `ValidatePaymentPromiseStateful` with the error as an unregistered Go
/// error (as the ante decorator returns it).
pub fn validate_payment_promise_stateful(
    ctx: &mut Context<'_>,
    p: &PaymentPromise,
) -> Result<Timestamp> {
    validate_payment_promise_stateful_internal(ctx, p, false)?.map_err(internal)
}

/// `sdk.AccAddress(promise.SignerPublicKey.Address()).String()`.
#[must_use]
pub fn escrow_signer(p: &PaymentPromise) -> String {
    let key = promise::signer_key(p);
    let addr = hash::ripemd160(&hash::sha256(key));
    to_bech32(ACC_PREFIX, &addr)
}

/// `ValidatePayForFibreSignatures`.
pub fn validate_pay_for_fibre_signatures(ctx: &mut Context<'_>, m: &MsgPayForFibre) -> Result<()> {
    let p = promise_of(m.payment_promise.as_ref());
    promise::from_proto_and_validate(&p)?;
    let sb = promise::sign_bytes(&p)
        .map_err(|e| errs::invalid_request(format!("failed to get validator sign bytes: {e}")))?;
    validate_validator_signatures(ctx, &sb, p.height, &m.validator_signatures).map_err(|e| {
        errs::invalid_request(format!("validator signature validation failed: {}", e.log))
    })
}

fn validate_validator_signatures(
    ctx: &mut Context<'_>,
    sign_bytes: &[u8],
    height: i64,
    signatures: &[Vec<u8>],
) -> Result<()> {
    use celestia_proto::cosmos::crypto::ed25519::PubKey as Ed25519PubKey;
    use prost::{Message, Name};
    let info = x_staking::store::get_historical_info(ctx, height)?.ok_or_else(|| {
        SdkError::new(
            "staking",
            38,
            format!("failed to get historical validator set at height {height}: no historical info found"),
        )
    })?;
    // (consensus pubkey, voting power = tokens) in historical-info order.
    let mut vals: Vec<([u8; 32], i64)> = Vec::with_capacity(info.valset.len());
    for v in &info.valset {
        let any = v.consensus_pubkey.clone().unwrap_or_default();
        if any.type_url != Ed25519PubKey::type_url() {
            if any.type_url == celestia_proto::cosmos::crypto::secp256k1::PubKey::type_url() {
                return Err(errs::invalid_request(format!(
                    "invalid ed25519 public key size for validator {}",
                    v.operator_address
                )));
            }
            return Err(errs::invalid_type(format!(
                "failed to get consensus public key for validator {}",
                v.operator_address
            )));
        }
        let pk = Ed25519PubKey::decode(any.value.as_slice())
            .map_err(|e| errs::invalid_type(e.to_string()))?;
        let key: [u8; 32] = pk.key.as_slice().try_into().map_err(|_| {
            errs::invalid_request(format!(
                "invalid ed25519 public key size for validator {}",
                v.operator_address
            ))
        })?;
        let tokens = Amount::parse(&v.tokens).map_err(|e| go_panic(e.to_string()))?;
        let power = tokens
            .to_i64()
            .ok_or_else(|| go_panic("Int64() out of bound"))?;
        vals.push((key, power));
    }
    let total: i64 = vals.iter().map(|v| v.1).sum();
    let min_required = total * 2 / 3;
    if signatures.len() > vals.len() {
        return Err(errs::invalid_request(format!(
            "signature count {} exceeds validator count {}",
            signatures.len(),
            vals.len()
        )));
    }
    let mut signed: Vec<[u8; 20]> = Vec::new();
    let mut power = 0i64;
    for (i, sig) in signatures.iter().enumerate() {
        if sig.is_empty() {
            continue;
        }
        let (key, vp) = vals[i];
        let addr: [u8; 20] = hash::sha256(&key)[..20].try_into().unwrap_or([0; 20]);
        if !promise::verify_ed25519_go(&key, sign_bytes, sig) {
            return Err(errs::invalid_request(format!(
                "invalid signature at index {i}: invalid signature from validator {}",
                hex::encode_upper(addr)
            )));
        }
        if !signed.contains(&addr) {
            signed.push(addr);
            power += vp;
        }
        if power >= min_required {
            return Ok(());
        }
    }
    if power < min_required {
        return Err(errs::invalid_request(format!(
            "signature validation failed: not enough voting power: collected {power}, required {min_required}"
        )));
    }
    Ok(())
}

// ---- Msg server ----

/// `DepositToEscrow`.
pub fn deposit_to_escrow(
    ctx: &mut Context<'_>,
    m: &MsgDepositToEscrow,
) -> Result<MsgDepositToEscrowResponse> {
    let signer_addr = acc_from_bech32(&m.signer)
        .map_err(|e| errs::invalid_address(format!("invalid signer address: {}", e.log)))?;
    let amount = coin(m.amount.as_ref())?;
    let mut escrow = get_escrow_account(ctx, &m.signer)?.unwrap_or_else(|| {
        celestia_proto::celestia::fibre::v1::EscrowAccount {
            signer: m.signer.clone(),
            balance: Some(Coin::new(amount.denom.clone(), 0).to_proto()),
            available_balance: Some(Coin::new(amount.denom.clone(), 0).to_proto()),
        }
    });
    x_bank::send_from_account_to_module(ctx, &signer_addr, MODULE_NAME, &coins(&amount)?)
        .map_err(|e| e.wrap("failed to transfer funds to escrow"))?;
    let balance = coin_add(&coin(escrow.balance.as_ref())?, &amount)?;
    let available = coin_add(&coin(escrow.available_balance.as_ref())?, &amount)?;
    escrow.balance = Some(balance.to_proto());
    escrow.available_balance = Some(available.to_proto());
    set_escrow_account(ctx, &escrow)?;
    emit(
        ctx,
        &EventDepositToEscrow {
            signer: m.signer.clone(),
            amount: Some(amount.to_proto()),
        },
    )?;
    Ok(MsgDepositToEscrowResponse {})
}

/// `RequestWithdrawal`.
pub fn request_withdrawal(
    ctx: &mut Context<'_>,
    m: &MsgRequestWithdrawal,
) -> Result<MsgRequestWithdrawalResponse> {
    let Some(mut escrow) = get_escrow_account(ctx, &m.signer)? else {
        return Err(errs::not_found(format!(
            "escrow account not found for signer: {}",
            m.signer
        )));
    };
    let amount = coin(m.amount.as_ref())?;
    let available = coin(escrow.available_balance.as_ref())?;
    if coin_lt(&available, &amount)? {
        return Err(errs::insufficient_funds(format!(
            "insufficient available balance: have {available}, need {amount}"
        )));
    }
    let params = get_params(ctx)?;
    let requested = ctx.header.time;
    if ctx
        .get(
            STORE_KEY,
            &crate::withdrawal_by_signer_key(&m.signer, &requested),
        )?
        .is_some()
    {
        return Err(errs::invalid_request(format!(
            "withdrawal request already exists for signer {} at timestamp {}",
            m.signer,
            sdk::time::go_time_string(&requested)
        )));
    }
    let available_at = add(&requested, dur(params.withdrawal_delay.as_ref()));
    let w = Withdrawal {
        signer: m.signer.clone(),
        amount: Some(amount.to_proto()),
        requested_timestamp: Some(requested),
        available_timestamp: Some(available_at),
    };
    escrow.available_balance = Some(coin_sub(&available, &amount)?.to_proto());
    set_escrow_account(ctx, &escrow)?;
    set_withdrawal(ctx, &w)?;
    emit(
        ctx,
        &EventWithdrawFromEscrowRequest {
            signer: m.signer.clone(),
            amount: Some(amount.to_proto()),
            requested_at: Some(requested),
            available_at: Some(available_at),
        },
    )?;
    Ok(MsgRequestWithdrawalResponse {})
}

/// `PayForFibre`.
pub fn pay_for_fibre(ctx: &mut Context<'_>, m: &MsgPayForFibre) -> Result<MsgPayForFibreResponse> {
    let p = promise_of(m.payment_promise.as_ref());
    promise::from_proto_and_validate(&p)?;
    validate_payment_promise_stateful_internal(ctx, &p, false)?.map_err(|e| {
        errs::invalid_request(format!("payment promise stateful verification failed: {e}"))
    })?;
    let hash = promise::hash(&p)
        .map_err(|e| errs::invalid_request(format!("failed to hash payment promise: {e}")))?;
    let signer = escrow_signer(&p);
    let Some(mut escrow) = get_escrow_account(ctx, &signer)? else {
        return Err(errs::not_found(format!(
            "escrow account not found for signer: {signer}"
        )));
    };
    let amount = payment_amount(p.blob_size);
    deduct_payment_from_escrow(ctx, &mut escrow, &amount)?;
    set_processed_payment(
        ctx,
        &ProcessedPayment {
            payment_promise_hash: hash,
            processed_at: Some(ctx.header.time),
        },
    )?;
    emit(
        ctx,
        &EventPayForFibre {
            signer,
            namespace: p.namespace.clone(),
            commitment: p.commitment.clone(),
            validator_count: u32::try_from(m.validator_signatures.len()).unwrap_or(u32::MAX),
        },
    )?;
    Ok(MsgPayForFibreResponse {})
}

/// `PaymentPromiseTimeout`.
pub fn payment_promise_timeout(
    ctx: &mut Context<'_>,
    m: &MsgPaymentPromiseTimeout,
) -> Result<MsgPaymentPromiseTimeoutResponse> {
    let p = promise_of(m.payment_promise.as_ref());
    promise::from_proto_and_validate(&p)?;
    let expiration = validate_payment_promise_stateful_internal(ctx, &p, true)?.map_err(|e| {
        errs::invalid_request(format!("payment promise stateful verification failed: {e}"))
    })?;
    let now = ctx.header.time;
    if before(&now, &expiration) {
        return Err(errs::invalid_request(format!(
            "payment promise has not yet timed out. Timeout at: {}, current time: {}",
            sdk::time::go_time_string(&expiration),
            sdk::time::go_time_string(&now)
        )));
    }
    let amount = payment_amount(p.blob_size);
    let hash = promise::hash(&p)
        .map_err(|e| errs::invalid_request(format!("failed to hash payment promise: {e}")))?;
    let escrow_signer = escrow_signer(&p);
    let Some(mut escrow) = get_escrow_account(ctx, &escrow_signer)? else {
        return Err(errs::not_found(format!(
            "escrow account not found for signer: {escrow_signer}"
        )));
    };
    deduct_payment_from_escrow(ctx, &mut escrow, &amount)?;
    set_processed_payment(
        ctx,
        &ProcessedPayment {
            payment_promise_hash: hash.clone(),
            processed_at: Some(ctx.header.time),
        },
    )?;
    emit(
        ctx,
        &EventPaymentPromiseTimeout {
            processor: m.signer.clone(),
            escrow_signer,
            payment_promise_hash: hash,
        },
    )?;
    Ok(MsgPaymentPromiseTimeoutResponse {})
}

/// `UpdateFibreParams`.
pub fn update_fibre_params(
    ctx: &mut Context<'_>,
    m: &MsgUpdateFibreParams,
) -> Result<MsgUpdateFibreParamsResponse> {
    let auth = authority();
    if m.authority != auth {
        return Err(errs::unauthorized(format!(
            "invalid authority; expected {auth}, got {}",
            m.authority
        )));
    }
    let params = crate::normalize_params(m.params.unwrap_or_default());
    validate_params(&params)
        .map_err(|e| errs::invalid_request(format!("invalid parameters: {e}")))?;
    set_params(ctx, &params)?;
    emit(
        ctx,
        &EventUpdateFibreParams {
            signer: m.authority.clone(),
            params: Some(params),
        },
    )?;
    Ok(MsgUpdateFibreParamsResponse {})
}

/// `deductPaymentFromEscrow`.
fn deduct_payment_from_escrow(
    ctx: &mut Context<'_>,
    escrow: &mut celestia_proto::celestia::fibre::v1::EscrowAccount,
    payment: &Coin,
) -> Result<()> {
    let balance = coin(escrow.balance.as_ref())?;
    if coin_lt(&balance, payment)? {
        return Err(errs::insufficient_funds(format!(
            "insufficient balance: have {balance}, need {payment}"
        )));
    }
    let available = coin(escrow.available_balance.as_ref())?;
    let from_available = if coin_lt(&available, payment)? {
        available.clone()
    } else {
        payment.clone()
    };
    escrow.balance = Some(coin_sub(&balance, payment)?.to_proto());
    escrow.available_balance = Some(coin_sub(&available, &from_available)?.to_proto());
    set_escrow_account(ctx, escrow)?;
    let shortfall = coin_sub(payment, &from_available)?;
    if !shortfall.amount.is_zero() {
        reduce_withdrawals_for_payment(ctx, &escrow.signer, shortfall)?;
    }
    x_bank::send_from_module_to_module(ctx, MODULE_NAME, FEE_COLLECTOR, &coins(payment)?)
        .map_err(|e| e.wrap("failed to send fibre payment to fee collector"))
}

/// `ReduceWithdrawalsForPayment`: cancels pending withdrawals oldest first.
pub fn reduce_withdrawals_for_payment(
    ctx: &mut Context<'_>,
    signer: &str,
    mut remaining: Coin,
) -> Result<()> {
    let mut prefix = vec![crate::WITHDRAWALS_BY_SIGNER_PREFIX];
    prefix.extend_from_slice(signer.as_bytes());
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    // `for ; it.Valid() && remaining.IsPositive(); it.Next()`
    while let Some((_, v)) = it.next(ctx)? {
        let w: Withdrawal = crate::decode(&v)?;
        let amt = coin(w.amount.as_ref())?;
        if coin_lt(&remaining, &amt)? {
            let mut w = w;
            w.amount = Some(coin_sub(&amt, &remaining)?.to_proto());
            set_withdrawal(ctx, &w)?;
            remaining = Coin::new(remaining.denom.clone(), 0);
        } else {
            remaining = coin_sub(&remaining, &amt)?;
            crate::delete_withdrawal(ctx, &w)?;
        }
        if remaining.amount.is_zero() || remaining.amount.is_negative() {
            // The loop's post statement still advances the iterator.
            let _ = it.next(ctx)?;
            break;
        }
    }
    if !(remaining.amount.is_zero() || remaining.amount.is_negative()) {
        return Err(internal(format!(
            "pending withdrawals for signer {signer} do not cover the shortfall of {remaining}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::cosmos::crypto::ed25519::PubKey as EdPub;
    use celestia_proto::cosmos::crypto::secp256k1::PubKey as SecpPub;
    use celestia_proto::cosmos::staking::v1beta1::{HistoricalInfo, Validator};
    use prost::Message;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    const CHAIN: &str = "test-chain";

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [
            STORE_KEY,
            x_bank::STORE_KEY,
            x_auth::STORE_KEY,
            x_staking::STORE_KEY,
        ] {
            ms.mount(s, Kind::Iavl).unwrap_or_default();
        }
        ms
    }

    fn header(secs: i64, height: i64) -> Header {
        Header {
            chain_id: CHAIN.into(),
            height,
            time: Timestamp {
                seconds: secs,
                nanos: 0,
            },
            ..Header::default()
        }
    }

    struct Keys {
        secp: [u8; 32],
        ed: [u8; 64],
    }

    fn keys() -> Keys {
        Keys {
            secp: crypto::secp256k1::secret_key_from_secret(b"escrow"),
            ed: crypto::ed25519::private_key_from_seed(&[3u8; 32]),
        }
    }

    fn signer_addr(k: &Keys) -> Vec<u8> {
        let pk = crypto::secp256k1::public_key_from_secret_key(&k.secp).unwrap_or([0; 33]);
        crypto::secp256k1::address(&pk).to_vec()
    }

    fn promise(k: &Keys, height: i64, created: i64, blob_size: u32) -> PaymentPromise {
        let pk = crypto::secp256k1::public_key_from_secret_key(&k.secp).unwrap_or([0; 33]);
        let mut ns = vec![0u8; 29];
        ns[19] = 0xaa;
        let mut p = PaymentPromise {
            chain_id: CHAIN.into(),
            height,
            namespace: ns,
            blob_size,
            blob_version: 0,
            commitment: vec![5; 32],
            creation_timestamp: Some(Timestamp {
                seconds: created,
                nanos: 0,
            }),
            signer_public_key: Some(SecpPub { key: pk.to_vec() }),
            signature: Vec::new(),
        };
        let sb = promise::sign_bytes(&p).unwrap_or_default();
        p.signature = crypto::secp256k1::sign(&k.secp, &sb)
            .unwrap_or([0; 64])
            .to_vec();
        p
    }

    fn setup(ctx: &mut Context<'_>, k: &Keys) -> Result<String> {
        let addr = signer_addr(k);
        x_bank::set_balance(ctx, &addr, &Coin::new("utia", 10_000_000))?;
        x_bank::set_supply(ctx, &Coin::new("utia", 10_000_000))?;
        let edpub = crypto::ed25519::public_key_from_private_key(&k.ed).unwrap_or([0; 32]);
        let info = HistoricalInfo {
            header: None,
            valset: vec![Validator {
                operator_address: "celestiavaloper1x".into(),
                consensus_pubkey: Some(celestia_proto::Any::pack(&EdPub {
                    key: edpub.to_vec(),
                })),
                tokens: "1000".into(),
                ..Validator::default()
            }],
        };
        ctx.set(
            x_staking::STORE_KEY,
            x_staking::keys::historical_info(10),
            info.encode_to_vec(),
        )?;
        Ok(to_bech32(ACC_PREFIX, &addr))
    }

    fn escrow(ctx: &mut Context<'_>, s: &str) -> (String, String) {
        let a = get_escrow_account(ctx, s)
            .ok()
            .flatten()
            .unwrap_or_default();
        (
            a.balance.map(|c| c.amount).unwrap_or_default(),
            a.available_balance.map(|c| c.amount).unwrap_or_default(),
        )
    }

    #[test]
    fn deposit_withdraw_pay_and_timeout() -> Result<()> {
        let k = keys();
        let mut ms = ms();
        let now = 1_800_000_000;
        let mut ctx = Context::new(&mut ms, header(now, 10), ExecMode::Finalize);
        let signer = setup(&mut ctx, &k)?;
        deposit_to_escrow(
            &mut ctx,
            &MsgDepositToEscrow {
                signer: signer.clone(),
                amount: Some(Coin::new("utia", 2_000_000).to_proto()),
            },
        )?;
        assert_eq!(
            escrow(&mut ctx, &signer),
            ("2000000".into(), "2000000".into())
        );
        request_withdrawal(
            &mut ctx,
            &MsgRequestWithdrawal {
                signer: signer.clone(),
                amount: Some(Coin::new("utia", 1_500_000).to_proto()),
            },
        )?;
        assert_eq!(
            escrow(&mut ctx, &signer),
            ("2000000".into(), "500000".into())
        );
        // A second request in the same block collides.
        let dup = request_withdrawal(
            &mut ctx,
            &MsgRequestWithdrawal {
                signer: signer.clone(),
                amount: Some(Coin::new("utia", 1).to_proto()),
            },
        );
        assert_eq!(dup.err().map(|e| e.code), Some(18));

        // Pay 695000 (one chunk): 500000 from available, 195000 from the
        // pending withdrawal.
        let p = promise(&k, 10, now - 5, 1000);
        let edpub = crypto::ed25519::public_key_from_private_key(&k.ed).unwrap_or([0; 32]);
        let sb = promise::sign_bytes(&p).unwrap_or_default();
        let sig = crypto::ed25519::sign(&k.ed, &sb).unwrap_or([0; 64]);
        assert!(promise::verify_ed25519_go(&edpub, &sb, &sig));
        let msg = MsgPayForFibre {
            signer: signer.clone(),
            payment_promise: Some(p.clone()),
            validator_signatures: vec![sig.to_vec()],
        };
        validate_basic_pay_for_fibre(&msg)?;
        validate_pay_for_fibre_signatures(&mut ctx, &msg)?;
        let bad = MsgPayForFibre {
            validator_signatures: vec![vec![1; 64]],
            ..msg.clone()
        };
        assert!(validate_pay_for_fibre_signatures(&mut ctx, &bad).is_err());
        let none = MsgPayForFibre {
            validator_signatures: vec![Vec::new()],
            ..msg.clone()
        };
        assert!(validate_pay_for_fibre_signatures(&mut ctx, &none).is_err());
        pay_for_fibre(&mut ctx, &msg)?;
        assert_eq!(escrow(&mut ctx, &signer), ("1305000".into(), "0".into()));
        let w = ctx
            .prefix_iter(STORE_KEY, &[crate::WITHDRAWALS_BY_SIGNER_PREFIX])?
            .into_items();
        assert_eq!(w.len(), 1);
        let w = Withdrawal::decode(w[0].1.as_slice()).map_err(|e| internal(e.to_string()))?;
        assert_eq!(w.amount.map(|c| c.amount), Some("1305000".into()));
        // Replays are rejected.
        assert!(pay_for_fibre(&mut ctx, &msg).is_err());
        let fee = x_bank::get_balance(&mut ctx, &x_auth::module_address(FEE_COLLECTOR), "utia")?;
        assert_eq!(fee.amount.to_string(), "695000");

        // Timeout: not before creation + timeout, then settles once.
        let old = promise(&k, 10, now - 4000, 1000);
        let t = MsgPaymentPromiseTimeout {
            signer: signer.clone(),
            payment_promise: Some(old.clone()),
        };
        assert!(validate_payment_promise_stateful(&mut ctx, &old).is_err());
        payment_promise_timeout(&mut ctx, &t)?;
        assert_eq!(escrow(&mut ctx, &signer), ("610000".into(), "0".into()));
        assert!(payment_promise_timeout(&mut ctx, &t).is_err());
        let fresh = promise(&k, 10, now - 1, 1000);
        let early = payment_promise_timeout(
            &mut ctx,
            &MsgPaymentPromiseTimeout {
                signer,
                payment_promise: Some(fresh),
            },
        );
        assert!(early.is_err());
        Ok(())
    }

    #[test]
    fn stateful_window_checks() -> Result<()> {
        let k = keys();
        let mut ms = ms();
        let now = 1_800_000_000;
        let mut ctx = Context::new(&mut ms, header(now, 2000), ExecMode::Finalize);
        let signer = setup(&mut ctx, &k)?;
        deposit_to_escrow(
            &mut ctx,
            &MsgDepositToEscrow {
                signer,
                amount: Some(Coin::new("utia", 2_000_000).to_proto()),
            },
        )?;
        let check = |ctx: &mut Context<'_>, p: &PaymentPromise| {
            validate_payment_promise_stateful(ctx, p).is_ok()
        };
        assert!(!check(&mut ctx, &promise(&k, 999, now, 1)));
        assert!(check(&mut ctx, &promise(&k, 1000, now, 1)));
        assert!(check(&mut ctx, &promise(&k, 2001, now + 600, 1)));
        assert!(!check(&mut ctx, &promise(&k, 2002, now, 1)));
        assert!(!check(&mut ctx, &promise(&k, 2001, now + 601, 1)));
        let mut other = promise(&k, 2000, now, 1);
        other.chain_id = "x".into();
        assert!(!check(&mut ctx, &other));
        assert!(!check(&mut ctx, &promise(&k, 2000, now, u32::MAX)));
        Ok(())
    }

    #[test]
    fn gas_formula_matches_go() {
        assert_eq!(estimate_gas_for_pay_for_fibre(0), 650_000);
        assert_eq!(estimate_gas_for_pay_for_fibre(1), 695_000);
        assert_eq!(estimate_gas_for_pay_for_fibre(262_144), 695_000);
        assert_eq!(estimate_gas_for_pay_for_fibre(262_145), 740_000);
    }

    #[test]
    fn params_validation() {
        let mut p = crate::default_params();
        assert!(validate_params(&p).is_ok());
        p.withdrawal_delay = Some(pbjson_types::Duration {
            seconds: 3600,
            nanos: 0,
        });
        assert!(validate_params(&p).is_err());
    }
}
