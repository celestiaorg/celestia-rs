//! celestia-app v10 `x/fibre`: genesis and `BeginBlocker`.
//!
//! Store layout:
//! - `"params"` → `Params` (defaults when unset)
//! - `0x02 || signer` → `EscrowAccount`
//! - `0x03 || signer || "/" || time(requested)` → `Withdrawal`
//! - `0x04 || time(available) || "/" || time(requested) || "/" || signer` → `Withdrawal`
//! - `0x05 || hash` → `ProcessedPayment`
//! - `0x06 || time(processed) || "/" || hash` → `ProcessedPayment`
//! - `0x07` → promise freshness floor (`sdk.FormatTimeBytes`)
//! - `0x08` → first block height (v12 genesis, big-endian int64)
//!
//! `time(t)` is `sdk.FormatTimeBytes`. The Msg server, stateful promise
//! validation and validator-signature checks are in [`msgs`]; payment
//! promise encoding and signatures in [`promise`].

mod genesis;
pub mod msgs;
pub mod promise;

use celestia_proto::celestia::fibre::v1::{
    EscrowAccount, EventProcessedPaymentPruned, EventWithdrawFromEscrowExecuted, GenesisState,
    Params, ProcessedPayment, Withdrawal,
};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use pbjson_types::{Duration, Timestamp};
use prost::Message;
use sdk::address::{acc_from_bech32, undefined};
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as sdkerr;
use sdk::event::typed_event;
use sdk::time::{
    SORTABLE_TIME_LEN, add, duration_nanos, format_time_bytes, is_zero, parse_time_bytes, to_nanos,
};
use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "fibre";
/// Module name (also the escrow module account).
pub const MODULE_NAME: &str = "fibre";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"params";
/// `EscrowAccountKeyPrefix`.
pub const ESCROW_ACCOUNT_PREFIX: u8 = 0x02;
/// `WithdrawalsBySignerKeyPrefix`.
pub const WITHDRAWALS_BY_SIGNER_PREFIX: u8 = 0x03;
/// `WithdrawalsByAvailableKeyPrefix`.
pub const WITHDRAWALS_BY_AVAILABLE_PREFIX: u8 = 0x04;
/// `ProcessedPaymentsByHashKeyPrefix`.
pub const PROCESSED_BY_HASH_PREFIX: u8 = 0x05;
/// `ProcessedPaymentsByTimeKeyPrefix`.
pub const PROCESSED_BY_TIME_PREFIX: u8 = 0x06;
/// `PromiseFreshnessFloorKey`.
pub const PROMISE_FRESHNESS_FLOOR_KEY: &[u8] = &[0x07];
/// `MaxPromiseClockSkew` (10 minutes) in nanoseconds.
pub const MAX_PROMISE_CLOCK_SKEW_NANOS: i128 = 600 * 1_000_000_000;

/// Height of the first block, introduced by the v12 executable.
const GENESIS_HEIGHT_KEY: &[u8] = &[0x08];

fn executing_version(ctx: &Context<'_>) -> u64 {
    sdk::version::executing(ctx.consensus_params.version.as_ref().map_or(0, |v| v.app))
}

/// Upstream `maxProcessedPaymentPrunesPerBlock` in x/fibre/keeper/abci.go.
const MAX_PROCESSED_PAYMENT_PRUNES_PER_BLOCK: usize = 2_000;

const HOUR: i64 = 3600;

/// `DefaultParams`.
#[must_use]
pub fn default_params() -> Params {
    default_params_at(sdk::version::app_version())
}

fn default_params_at(version: u64) -> Params {
    Params {
        withdrawal_delay: Some(Duration {
            seconds: 24 * HOUR,
            nanos: 0,
        }),
        payment_promise_timeout: Some(Duration {
            seconds: HOUR,
            nanos: 0,
        }),
        payment_promise_height_window: if sdk::version::is_v12(version) {
            3_250
        } else {
            1_000
        },
        shard_retention: Some(Duration {
            seconds: 4 * HOUR,
            nanos: 0,
        }),
        full_stake_storage_budget: 2 << 40,
    }
}

/// gogoproto non-nullable `stdduration` fields are always encoded; make the
/// prost form carry them explicitly.
pub(crate) fn normalize_params(mut p: Params) -> Params {
    for d in [
        &mut p.withdrawal_delay,
        &mut p.payment_promise_timeout,
        &mut p.shard_retention,
    ] {
        d.get_or_insert_with(Duration::default);
    }
    p
}

pub(crate) fn ts_or_zero(t: Option<Timestamp>) -> Timestamp {
    t.unwrap_or_else(sdk::time::go_zero)
}

/// gogoproto non-nullable `Coin` with a nil `Int` encodes amount `"0"`.
pub(crate) fn normalize_coin(c: Option<PbCoin>) -> PbCoin {
    let mut c = c.unwrap_or_default();
    if c.amount.is_empty() {
        c.amount = "0".into();
    }
    c
}

fn normalize_escrow(mut a: EscrowAccount) -> EscrowAccount {
    a.balance = Some(normalize_coin(a.balance));
    a.available_balance = Some(normalize_coin(a.available_balance));
    a
}

fn normalize_withdrawal(mut w: Withdrawal) -> Withdrawal {
    w.amount = Some(normalize_coin(w.amount));
    w.requested_timestamp = Some(ts_or_zero(w.requested_timestamp));
    w.available_timestamp = Some(ts_or_zero(w.available_timestamp));
    w
}

fn normalize_processed(mut p: ProcessedPayment) -> ProcessedPayment {
    p.processed_at = Some(ts_or_zero(p.processed_at));
    p
}

pub(crate) fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| undefined(e.to_string()))
}

/// `Keeper.GetParams` (defaults when unset).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    match ctx.get(STORE_KEY, PARAMS_KEY)? {
        Some(bz) if !bz.is_empty() => decode(&bz),
        _ => Ok(default_params_at(executing_version(ctx))),
    }
}

/// `Keeper.SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(
        STORE_KEY,
        PARAMS_KEY.to_vec(),
        normalize_params(*p).encode_to_vec(),
    )
}

/// `Keeper.GetPromiseFreshnessFloor` (Go zero time when unset).
pub fn get_promise_freshness_floor(ctx: &mut Context<'_>) -> Result<Timestamp> {
    match ctx.get(STORE_KEY, PROMISE_FRESHNESS_FLOOR_KEY)? {
        Some(bz) if !bz.is_empty() => parse_time_bytes(&bz).ok_or_else(|| {
            undefined("failed to parse promise freshness floor: invalid time bytes")
        }),
        _ => Ok(sdk::time::go_zero()),
    }
}

/// `Keeper.SetPromiseFreshnessFloor`.
pub fn set_promise_freshness_floor(ctx: &mut Context<'_>, t: &Timestamp) -> Result<()> {
    ctx.set(
        STORE_KEY,
        PROMISE_FRESHNESS_FLOOR_KEY.to_vec(),
        format_time_bytes(t),
    )
}

fn escrow_key(signer: &str) -> Vec<u8> {
    let mut k = vec![ESCROW_ACCOUNT_PREFIX];
    k.extend_from_slice(signer.as_bytes());
    k
}

/// `Keeper.GetEscrowAccount`.
pub fn get_escrow_account(ctx: &mut Context<'_>, signer: &str) -> Result<Option<EscrowAccount>> {
    ctx.get(STORE_KEY, &escrow_key(signer))?
        .map(|b| decode(&b))
        .transpose()
}

/// `Keeper.SetEscrowAccount`.
pub fn set_escrow_account(ctx: &mut Context<'_>, a: &EscrowAccount) -> Result<()> {
    let a = normalize_escrow(a.clone());
    ctx.set(STORE_KEY, escrow_key(&a.signer), a.encode_to_vec())
}

pub(crate) fn withdrawal_by_signer_key(signer: &str, requested: &Timestamp) -> Vec<u8> {
    let mut k = vec![WITHDRAWALS_BY_SIGNER_PREFIX];
    k.extend_from_slice(signer.as_bytes());
    k.push(b'/');
    k.extend_from_slice(&format_time_bytes(requested));
    k
}

fn withdrawal_by_available_key(
    available: &Timestamp,
    requested: &Timestamp,
    signer: &str,
) -> Vec<u8> {
    let mut k = vec![WITHDRAWALS_BY_AVAILABLE_PREFIX];
    k.extend_from_slice(&format_time_bytes(available));
    k.push(b'/');
    k.extend_from_slice(&format_time_bytes(requested));
    k.push(b'/');
    k.extend_from_slice(signer.as_bytes());
    k
}

/// `Keeper.SetWithdrawal` (primary then secondary index).
pub fn set_withdrawal(ctx: &mut Context<'_>, w: &Withdrawal) -> Result<()> {
    let w = normalize_withdrawal(w.clone());
    let bz = w.encode_to_vec();
    let req = ts_or_zero(w.requested_timestamp);
    let avail = ts_or_zero(w.available_timestamp);
    ctx.set(
        STORE_KEY,
        withdrawal_by_signer_key(&w.signer, &req),
        bz.clone(),
    )?;
    ctx.set(
        STORE_KEY,
        withdrawal_by_available_key(&avail, &req, &w.signer),
        bz,
    )
}

/// `Keeper.DeleteWithdrawal`.
pub fn delete_withdrawal(ctx: &mut Context<'_>, w: &Withdrawal) -> Result<()> {
    let req = ts_or_zero(w.requested_timestamp);
    let avail = ts_or_zero(w.available_timestamp);
    ctx.delete(STORE_KEY, &withdrawal_by_signer_key(&w.signer, &req))?;
    ctx.delete(
        STORE_KEY,
        &withdrawal_by_available_key(&avail, &req, &w.signer),
    )
}

fn processed_by_hash_key(hash: &[u8]) -> Vec<u8> {
    let mut k = vec![PROCESSED_BY_HASH_PREFIX];
    k.extend_from_slice(hash);
    k
}

fn processed_by_time_key(at: &Timestamp, hash: &[u8]) -> Vec<u8> {
    let mut k = vec![PROCESSED_BY_TIME_PREFIX];
    k.extend_from_slice(&format_time_bytes(at));
    k.push(b'/');
    k.extend_from_slice(hash);
    k
}

/// `Keeper.SetProcessedPayment`.
pub fn set_processed_payment(ctx: &mut Context<'_>, p: &ProcessedPayment) -> Result<()> {
    let p = normalize_processed(p.clone());
    let bz = p.encode_to_vec();
    let at = ts_or_zero(p.processed_at);
    ctx.set(
        STORE_KEY,
        processed_by_hash_key(&p.payment_promise_hash),
        bz.clone(),
    )?;
    ctx.set(
        STORE_KEY,
        processed_by_time_key(&at, &p.payment_promise_hash),
        bz,
    )
}

/// `Keeper.DeleteProcessedPayment`.
pub fn delete_processed_payment(ctx: &mut Context<'_>, p: &ProcessedPayment) -> Result<()> {
    let at = ts_or_zero(p.processed_at);
    ctx.delete(STORE_KEY, &processed_by_hash_key(&p.payment_promise_hash))?;
    ctx.delete(
        STORE_KEY,
        &processed_by_time_key(&at, &p.payment_promise_hash),
    )
}

/// `Keeper.IsPaymentProcessedByHash`.
pub fn is_payment_processed_by_hash(ctx: &mut Context<'_>, hash: &[u8]) -> Result<bool> {
    ctx.has(STORE_KEY, &processed_by_hash_key(hash))
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let is_v12 = sdk::version::is_v12(executing_version(ctx));
    if is_v12 {
        genesis::validate_records(gs)?;
        let checkpoint = ctx.branch();
        let result = write_genesis(ctx, gs, true);
        if result.is_ok() {
            ctx.write(checkpoint);
        } else {
            ctx.discard(checkpoint);
        }
        return result;
    }
    write_genesis(ctx, gs, false)
}

/// Migrate committed v10/v11 escrow and withdrawal records to v12.
pub fn migrate_escrows(ctx: &mut Context<'_>) -> Result<()> {
    let mut records = GenesisState::default();
    for (key, value) in ctx
        .prefix_iter(STORE_KEY, &[ESCROW_ACCOUNT_PREFIX])?
        .into_items()?
    {
        let account: EscrowAccount = decode(&value)?;
        if key != escrow_key(&account.signer) {
            return Err(sdk::error::internal("invalid legacy fibre escrow key"));
        }
        records.escrow_accounts.push(account);
    }
    for (key, value) in ctx
        .prefix_iter(STORE_KEY, &[WITHDRAWALS_BY_SIGNER_PREFIX])?
        .into_items()?
    {
        let withdrawal: Withdrawal = decode(&value)?;
        if key
            != withdrawal_by_signer_key(
                &withdrawal.signer,
                &ts_or_zero(withdrawal.requested_timestamp),
            )
        {
            return Err(sdk::error::internal("invalid legacy fibre withdrawal key"));
        }
        records.withdrawals.push(withdrawal);
    }
    genesis::validate_records(&records)?;
    genesis::migrate_legacy_escrows(ctx, &records)
}

fn write_genesis(ctx: &mut Context<'_>, gs: &GenesisState, is_v12: bool) -> Result<()> {
    set_params(ctx, &gs.params.unwrap_or_default())?;
    if is_v12 {
        ctx.set(
            STORE_KEY,
            GENESIS_HEIGHT_KEY.to_vec(),
            ctx.height().max(1).to_be_bytes().to_vec(),
        )?;
    }
    for a in &gs.escrow_accounts {
        set_escrow_account(ctx, a)?;
    }
    for w in &gs.withdrawals {
        set_withdrawal(ctx, w)?;
    }
    if is_v12 {
        genesis::migrate_legacy_escrows(ctx, gs)?;
    }
    for p in &gs.processed_payments {
        set_processed_payment(ctx, p)?;
    }
    let floor = ts_or_zero(gs.promise_freshness_floor);
    if !is_zero(&floor) {
        set_promise_freshness_floor(ctx, &floor)?;
    }
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

fn update_promise_freshness_floor(ctx: &mut Context<'_>, params: &Params) -> Result<()> {
    let delay = duration_nanos(&params.withdrawal_delay.unwrap_or_default());
    let candidate = add(&ctx.header.time, -delay);
    let current = get_promise_freshness_floor(ctx)?;
    if to_nanos(&candidate) > to_nanos(&current) {
        set_promise_freshness_floor(ctx, &candidate)?;
    }
    Ok(())
}

/// Parses `time(available) "/" time(requested) "/" signer` after the prefix.
fn parse_withdrawals_by_available_key(key: &[u8]) -> Option<(Timestamp, Timestamp, String)> {
    let mut key = key.get(1..)?;
    let available = parse_time_bytes(key.get(..SORTABLE_TIME_LEN)?)?;
    key = &key[SORTABLE_TIME_LEN..];
    if key.first() == Some(&b'/') {
        key = &key[1..];
    }
    let requested = parse_time_bytes(key.get(..SORTABLE_TIME_LEN)?)?;
    key = &key[SORTABLE_TIME_LEN..];
    if key.first() == Some(&b'/') {
        key = &key[1..];
    }
    Some((
        available,
        requested,
        String::from_utf8_lossy(key).into_owned(),
    ))
}

fn coin_from(c: Option<&PbCoin>) -> Result<Coin> {
    Coin::from_proto(&normalize_coin(c.cloned()))
}

fn process_available_withdrawals(ctx: &mut Context<'_>) -> Result<()> {
    let now = ctx.header.time;
    let mut upto = vec![WITHDRAWALS_BY_AVAILABLE_PREFIX];
    upto.extend_from_slice(&format_time_bytes(&now));
    let end = sdk::context::prefix_end(&upto);
    let mut it = ctx.iterator(
        STORE_KEY,
        Some(&[WITHDRAWALS_BY_AVAILABLE_PREFIX]),
        end.as_deref(),
    )?;
    while let Some((k, v)) = it.next(ctx)? {
        // Unparseable keys are logged and skipped in Go.
        let Some((available, _, signer)) = parse_withdrawals_by_available_key(&k) else {
            continue;
        };
        if to_nanos(&available) > to_nanos(&now) {
            break;
        }
        // Go's MustUnmarshal panics on corrupt values.
        let w: Withdrawal = decode(&v)?;
        let amount = coin_from(w.amount.as_ref())?;
        let Ok(signer_addr) = acc_from_bech32(&signer) else {
            continue;
        };
        let Some(mut escrow) = get_escrow_account(ctx, &signer)? else {
            continue;
        };
        let balance = coin_from(escrow.balance.as_ref())?;
        if balance.denom != amount.denom {
            // `Coin.IsLT` panics on mismatched denoms.
            return Err(sdkerr::invalid_coins(format!(
                "invalid coin denominations; {}, {}",
                balance.denom, amount.denom
            )));
        }
        if balance.amount < amount.amount {
            continue;
        }
        let coins = Coins::sanitized(vec![amount.clone()])?;
        if x_bank::send_from_module_to_account(ctx, MODULE_NAME, &signer_addr, &coins).is_err() {
            continue;
        }
        let rest = balance
            .amount
            .checked_sub(&amount.amount)
            .map_err(sdk::coins::math_err)?;
        escrow.balance = Some(Coin::with_amount(balance.denom.clone(), rest).to_proto());
        set_escrow_account(ctx, &escrow)?;
        delete_withdrawal(ctx, &w)?;
        if let Ok(ev) = typed_event(&EventWithdrawFromEscrowExecuted {
            signer: signer.clone(),
            amount: Some(amount.to_proto()),
        }) {
            ctx.events.emit(ev);
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct PaymentEvent<T> {
    #[serde(flatten)]
    base: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    creation_timestamp: Option<Timestamp>,
}

fn prune_processed_payments(ctx: &mut Context<'_>, params: &Params) -> Result<()> {
    let retention =
        duration_nanos(&params.withdrawal_delay.unwrap_or_default()) + MAX_PROMISE_CLOCK_SKEW_NANOS;
    let cutoff = add(&ctx.header.time, -retention);
    let mut upto = vec![PROCESSED_BY_TIME_PREFIX];
    upto.extend_from_slice(&format_time_bytes(&cutoff));
    let end = sdk::context::prefix_end(&upto);
    let is_v12 = sdk::version::is_v12(executing_version(ctx));
    let budget = if is_v12 {
        MAX_PROCESSED_PAYMENT_PRUNES_PER_BLOCK
    } else {
        usize::MAX
    };
    let mut it = ctx.iterator(STORE_KEY, Some(&[PROCESSED_BY_TIME_PREFIX]), end.as_deref())?;
    for _ in 0..budget {
        let Some((k, v)) = it.next(ctx)? else {
            break;
        };
        let Some(ts_bytes) = k.get(1..=SORTABLE_TIME_LEN) else {
            // Go slices key[:29] and would panic on a short key.
            return Err(sdkerr::logic("processed payment key too short"));
        };
        let Some(processed_at) = parse_time_bytes(ts_bytes) else {
            continue;
        };
        if to_nanos(&processed_at) > to_nanos(&cutoff) {
            break;
        }
        let mut hash = &k[1 + SORTABLE_TIME_LEN..];
        if hash.first() == Some(&b'/') {
            hash = &hash[1..];
        }
        let p: ProcessedPayment = decode(&v)?;
        delete_processed_payment(ctx, &p)?;
        let event = PaymentEvent {
            base: EventProcessedPaymentPruned {
                payment_promise_hash: hash.to_vec(),
                processed_at: Some(processed_at),
            },
            // Payments in the legacy index predate the creation-time field.
            creation_timestamp: is_v12.then(sdk::time::go_zero),
        };
        if let Ok(ev) = sdk::event::typed_event_with_name(
            <EventProcessedPaymentPruned as prost::Name>::full_name(),
            &event,
        ) {
            ctx.events.emit(ev);
        }
    }
    Ok(())
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    let params = get_params(ctx)?;
    update_promise_freshness_floor(ctx, &params)?;
    process_available_withdrawals(ctx)?;
    let params = get_params(ctx)?;
    prune_processed_payments(ctx, &params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl).unwrap_or_default();
        ms
    }

    fn hex(b: &[u8]) -> String {
        b.iter().fold(String::new(), |mut s, x| {
            use std::fmt::Write as _;
            let _ = write!(s, "{x:02x}");
            s
        })
    }

    #[test]
    fn default_height_window_follows_the_executing_release_until_params_are_stored() -> Result<()> {
        use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;

        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 11 });
        assert_eq!(get_params(&mut ctx)?.payment_promise_height_window, 1_000);

        // The v12 binary can query a height whose consensus version is still 11.
        let previous = sdk::version::app_version();
        sdk::version::set_app_version(12);
        let current = default_params();
        let historical = get_params(&mut ctx);
        sdk::version::set_app_version(previous);
        assert_eq!(current.payment_promise_height_window, 3_250);
        assert_eq!(historical?, current);

        ctx.consensus_params.version = Some(VersionParams { app: 12 });
        let stored = Params {
            payment_promise_height_window: 77,
            ..current
        };
        set_params(&mut ctx, &stored)?;
        assert_eq!(get_params(&mut ctx)?, stored);
        Ok(())
    }

    #[test]
    fn genesis_and_floor_match_devnet() -> Result<()> {
        let mut ms = ms();
        let header = Header {
            // 2026-10-14T05:29:15.701265606Z
            time: Timestamp {
                seconds: 1_791_955_755,
                nanos: 701_265_606,
            },
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        init_genesis_json(
            &mut ctx,
            &serde_json::json!({
                "params": {"withdrawal_delay": "86400s", "payment_promise_timeout": "3600s",
                    "payment_promise_height_window": "1000", "shard_retention": "14400s",
                    "full_stake_storage_budget": "2199023255552"},
                "escrow_accounts": [], "withdrawals": [], "processed_payments": [],
                "promise_freshness_floor": "0001-01-01T00:00:00Z"
            }),
        )?;
        assert_eq!(
            hex(&ctx.get(STORE_KEY, b"params")?.unwrap_or_default()),
            "0a040880a305120308901c18e807220308c07028808080808040"
        );
        assert_eq!(ctx.get(STORE_KEY, &[0x07])?, None);
        assert_eq!(get_params(&mut ctx)?, default_params());
        begin_block(&mut ctx)?;
        assert_eq!(
            ctx.get(STORE_KEY, &[0x07])?,
            Some(b"2026-10-13T05:29:15.701265606".to_vec())
        );
        Ok(())
    }

    #[test]
    fn validates_legacy_payments_before_v12_genesis_writes() -> Result<()> {
        use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;

        // Unix epoch is a valid processed time. Go's zero time is year one.
        let payment = ProcessedPayment {
            payment_promise_hash: vec![1],
            processed_at: Some(Timestamp::default()),
        };
        for (invalid, reason) in [
            (
                ProcessedPayment {
                    payment_promise_hash: vec![],
                    ..payment.clone()
                },
                "invalid hash",
            ),
            (payment.clone(), "duplicate hash"),
            (
                ProcessedPayment {
                    payment_promise_hash: vec![2],
                    processed_at: None,
                },
                "invalid timestamp",
            ),
            (
                ProcessedPayment {
                    payment_promise_hash: vec![2],
                    processed_at: Some(sdk::time::go_zero()),
                },
                "invalid timestamp",
            ),
        ] {
            let mut ms = ms();
            let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
            let gs = GenesisState {
                processed_payments: vec![payment.clone(), invalid],
                ..Default::default()
            };
            ctx.consensus_params.version = Some(VersionParams { app: 12 });
            let err = init_genesis(&mut ctx, &gs).unwrap_err();
            assert!(err.is_panic());
            assert_eq!(err.log, format!("invalid fibre genesis state: {reason}"));
            assert!(!ctx.has(STORE_KEY, PARAMS_KEY)?);
            assert!(!is_payment_processed_by_hash(&mut ctx, &[1])?);

            ctx.consensus_params.version = Some(VersionParams { app: 11 });
            init_genesis(&mut ctx, &gs)?;
            assert!(is_payment_processed_by_hash(&mut ctx, &[1])?);
        }
        Ok(())
    }

    #[test]
    fn prunes_expired_processed_payments() -> Result<()> {
        let mut ms = ms();
        let now = Timestamp {
            seconds: 10 * 86_400,
            nanos: 0,
        };
        let header = Header {
            time: now,
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        let old = ProcessedPayment {
            payment_promise_hash: vec![1; 32],
            processed_at: Some(add(&now, -(25 * 3600) * 1_000_000_000)),
        };
        let fresh = ProcessedPayment {
            payment_promise_hash: vec![2; 32],
            processed_at: Some(add(&now, -3600 * 1_000_000_000)),
        };
        set_processed_payment(&mut ctx, &old)?;
        set_processed_payment(&mut ctx, &fresh)?;
        begin_block(&mut ctx)?;
        assert!(!is_payment_processed_by_hash(&mut ctx, &[1; 32])?);
        assert!(is_payment_processed_by_hash(&mut ctx, &[2; 32])?);
        assert_eq!(
            ctx.events.events()[0].r#type,
            "celestia.fibre.v1.EventProcessedPaymentPruned"
        );
        Ok(())
    }

    #[test]
    fn prune_budget_for_payments_inherited_from_v11() -> Result<()> {
        use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;

        let now = Timestamp {
            seconds: 10 * 86_400,
            nanos: 0,
        };
        let expired = add(&now, -(25 * 3600) * 1_000_000_000);
        let hash = |i: u32| {
            let mut bytes = [0; 32];
            bytes[28..].copy_from_slice(&i.to_be_bytes());
            bytes
        };

        for version in [11, 12] {
            let mut ms = ms();
            let mut ctx = Context::new(
                &mut ms,
                Header {
                    time: now,
                    ..Header::default()
                },
                ExecMode::Finalize,
            );
            ctx.consensus_params.version = Some(VersionParams { app: 11 });
            for i in 0..=2000 {
                set_processed_payment(
                    &mut ctx,
                    &ProcessedPayment {
                        payment_promise_hash: hash(i).to_vec(),
                        processed_at: Some(expired),
                    },
                )?;
            }
            ctx.consensus_params.version = Some(VersionParams { app: version });
            begin_block(&mut ctx)?;
            assert_eq!(ctx.events.len(), if version == 12 { 2000 } else { 2001 });
            let creation = ctx.events.events()[0]
                .attributes
                .iter()
                .find(|attr| attr.key == "creation_timestamp");
            assert_eq!(
                creation.map(|attr| attr.value.as_str()),
                (version == 12).then_some(r#""0001-01-01T00:00:00Z""#)
            );
            assert!(!is_payment_processed_by_hash(&mut ctx, &hash(1999))?);
            assert_eq!(
                is_payment_processed_by_hash(&mut ctx, &hash(2000))?,
                version == 12
            );
            ctx.header.height += 1;
            begin_block(&mut ctx)?;
            assert_eq!(ctx.events.len(), 2001);
            assert!(!is_payment_processed_by_hash(&mut ctx, &hash(2000))?);
        }
        Ok(())
    }

    #[test]
    fn available_key_roundtrip() {
        let a = Timestamp {
            seconds: 100,
            nanos: 5,
        };
        let r = Timestamp {
            seconds: 50,
            nanos: 0,
        };
        let k = withdrawal_by_available_key(&a, &r, "celestia1xyz");
        assert_eq!(
            parse_withdrawals_by_available_key(&k),
            Some((a, r, "celestia1xyz".to_string()))
        );
    }
}
