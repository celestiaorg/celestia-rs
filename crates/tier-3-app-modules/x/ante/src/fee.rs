//! `DeductFeeDecorator` (cosmos-sdk `x/auth/ante/fee.go`) with celestia's
//! fee checker (`app/ante/fee.go` `ValidateTxFee`).

use math::{Int, LegacyDec};
use sdk::Context;
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::tx::Tx;

use crate::basic::coin_amount;
use crate::deps::AnteDeps;
use crate::{Fail, R, is_check_tx, panic, txview};

/// `appconsts.BondDenom`.
pub const BOND_DENOM: &str = "utia";
/// `priorityScalingFactor`.
pub const PRIORITY_SCALING_FACTOR: i64 = 1_000_000;

/// A decoded fee coin (`sdk.Coin` with a possibly nil amount).
#[derive(Debug, Clone)]
struct FeeCoin {
    denom: String,
    amount: Option<Int>,
}

impl FeeCoin {
    fn amount(&self) -> R<&Int> {
        self.amount.as_ref().map_or_else(
            || panic("runtime error: invalid memory address or nil pointer dereference"),
            Ok,
        )
    }
}

fn fee_coins(tx: &Tx) -> R<Vec<FeeCoin>> {
    txview::fee(tx)?
        .amount
        .iter()
        .map(|c| {
            Ok(FeeCoin {
                denom: c.denom.clone(),
                amount: coin_amount(&c.amount)?,
            })
        })
        .collect()
}

/// `Coins.Find` (binary search assuming sorted coins).
fn find<'a>(coins: &'a [FeeCoin], denom: &str) -> Option<&'a FeeCoin> {
    match coins.len() {
        0 => None,
        1 => (coins[0].denom == denom).then(|| &coins[0]),
        n => {
            let mid = n / 2;
            let c = &coins[mid];
            match denom.cmp(c.denom.as_str()) {
                std::cmp::Ordering::Less => find(&coins[..mid], denom),
                std::cmp::Ordering::Equal => Some(c),
                std::cmp::Ordering::Greater => find(&coins[mid + 1..], denom),
            }
        }
    }
}

/// `Coins.String`.
fn coins_string(coins: &[FeeCoin]) -> R<String> {
    let mut parts = Vec::with_capacity(coins.len());
    for c in coins {
        parts.push(format!("{}{}", c.amount()?, c.denom));
    }
    Ok(parts.join(","))
}

/// `verifyMinFee`.
fn verify_min_fee(fee: &Int, gas: u64, min_gas_price: &LegacyDec, msg: &str) -> R<()> {
    let min_fee = min_gas_price
        .checked_mul_int(&Int::from_u64(gas))
        .and_then(|d| d.ceil())
        .or_else(|e| panic(e.to_string()))?;
    let required = min_fee.truncate_int();
    if fee < &required {
        return Err(errs::insufficient_fee(format!(
            "{msg}; got: {fee}{BOND_DENOM}, required: {required}{BOND_DENOM} (min gas price: {} {BOND_DENOM}/gas)",
            trim_trailing_zeros(min_gas_price)
        ))
        .into());
    }
    Ok(())
}

fn trim_trailing_zeros(d: &LegacyDec) -> String {
    let s = d.to_string();
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// `getTxPriority`: `fee * 1e6 / gas`, 0 when gas is 0 or the result does
/// not fit an int64. Multiplication overflow is a Go panic.
pub fn get_tx_priority(fee: &Int, gas: u64) -> R<i64> {
    if gas == 0 {
        return Ok(0);
    }
    let scaled = fee
        .checked_mul(&Int::from_i64(PRIORITY_SCALING_FACTOR))
        .or_else(|_| panic("integer overflow"))?;
    let p = scaled
        .checked_quo(&Int::from_u64(gas))
        .or_else(|_| panic("division by zero"))?;
    Ok(p.to_i64().unwrap_or(0))
}

/// The default node min gas price (`appconsts.DefaultMinGasPrice` = 0.004).
#[must_use]
pub fn default_min_gas_price() -> LegacyDec {
    LegacyDec::new_with_prec(4_000, 6).unwrap_or_else(|_| LegacyDec::zero())
}

/// celestia-app v9.0.8 `getTxPriority`: the lowest `amount * 1e6 / gas`
/// over all fee coins (results that do not fit an int64 are skipped; a 0
/// is replaced by the next coin's value). `Mul` overflow and `gas = 0`
/// panic like Go's `math.Int`.
fn get_tx_priority_v9(coins: &[FeeCoin], gas: u64) -> R<i64> {
    let mut priority: i64 = 0;
    for c in coins {
        let scaled = c
            .amount()?
            .checked_mul(&Int::from_i64(PRIORITY_SCALING_FACTOR))
            .or_else(|_| panic("integer overflow"))?;
        if gas == 0 {
            return panic("division by zero");
        }
        let p = scaled
            .checked_quo(&Int::from_i64(i64::from_ne_bytes(gas.to_ne_bytes())))
            .or_else(|_| panic("division by zero"))?;
        let Some(p) = p.to_i64() else {
            continue;
        };
        if priority == 0 || p < priority {
            priority = p;
        }
    }
    Ok(priority)
}

/// celestia `ValidateTxFee`: utia-only fees (v10), the node minimum in
/// CheckTx, the network minimum always; returns the tx priority.
fn validate_tx_fee(
    ctx: &mut Context<'_>,
    coins: &[FeeCoin],
    gas: u64,
    deps: &impl AnteDeps,
) -> R<i64> {
    let v9 = crate::is_v9(ctx);
    for c in coins {
        if !v9 && c.denom != BOND_DENOM {
            return Err(errs::invalid_coins(format!(
                "fee must be denominated in {BOND_DENOM}, got {}",
                c.denom
            ))
            .into());
        }
    }
    let fee = match find(coins, BOND_DENOM) {
        Some(c) => c.amount()?.clone(),
        None => Int::zero(),
    };
    if is_check_tx(ctx.mode) {
        let min = deps
            .node_min_gas_price()
            .unwrap_or_else(default_min_gas_price);
        if !min.is_zero() {
            verify_min_fee(
                &fee,
                gas,
                &min,
                "insufficient minimum gas price for this node",
            )?;
        }
    }
    let network = deps.network_min_gas_price(ctx)?;
    verify_min_fee(
        &fee,
        gas,
        &network,
        "insufficient gas price for the network",
    )?;
    let priority = if v9 {
        get_tx_priority_v9(coins, gas)?
    } else {
        get_tx_priority(&fee, gas)?
    };
    // Even though telemetry is not consensus state, Go evaluates this
    // conversion before deducting fees. Its panic is part of the tx result.
    if fee.to_i64().is_none() {
        return panic("Int64() out of bound");
    }
    Ok(priority)
}

/// `DeductFeeDecorator.AnteHandle`; returns the tx priority.
pub fn deduct_fee(ctx: &mut Context<'_>, tx: &Tx, simulate: bool, deps: &impl AnteDeps) -> R<i64> {
    let gas = txview::gas(tx)?;
    if !simulate && ctx.height() > 0 && gas == 0 {
        return Err(errs::invalid_gas_limit("must provide positive gas").into());
    }
    let coins = fee_coins(tx)?;
    let mut priority = 0;
    if !simulate {
        priority = validate_tx_fee(ctx, &coins, gas, deps)?;
    }
    check_deduct_fee(ctx, tx, &coins, deps)?;
    Ok(priority)
}

fn check_deduct_fee(
    ctx: &mut Context<'_>,
    tx: &Tx,
    fee: &[FeeCoin],
    deps: &impl AnteDeps,
) -> R<()> {
    let fee_payer = txview::fee_payer(tx)?;
    let fee_granter = txview::fee_granter(tx)?;
    let mut deduct_from = fee_payer.clone();
    if let Some(granter) = fee_granter {
        if granter != fee_payer {
            let coins = to_coins(fee)?;
            if let Err(e) = deps.use_granted_fees(ctx, &granter, &fee_payer, &coins, tx.msgs()) {
                if is_out_of_gas(&e) {
                    return Err(e.into());
                }
                // `%s` of the `[]byte` addresses `FeeTx` returns: the raw
                // bytes.
                return Err(e
                    .wrap(format!(
                        "{} does not allow to pay fees for {}",
                        gostr::from_go_bytes(&granter),
                        gostr::from_go_bytes(&fee_payer)
                    ))
                    .into());
            }
        }
        deduct_from = granter;
    }
    if x_auth::get_account(ctx, &deduct_from)?.is_none() {
        // `deductFeesFrom` is a `[]byte` variable: `%s` prints the raw
        // bytes.
        return Err(errs::unknown_address(format!(
            "fee payer address: {} does not exist",
            gostr::from_go_bytes(&deduct_from)
        ))
        .into());
    }
    let mut is_zero = true;
    for c in fee {
        if !c.amount()?.is_zero() {
            is_zero = false;
        }
    }
    if !is_zero {
        deduct_fees(ctx, &deduct_from, fee)?;
    }
    ctx.events.emit(event(
        "tx",
        [
            ("fee", coins_string(fee)?),
            ("fee_payer", txview::bytes_to_bech32(&deduct_from)),
        ],
    ));
    Ok(())
}

/// Whether an error is Go's recovered out-of-gas panic (passes through
/// error wrapping untouched in Go).
#[must_use]
pub fn is_out_of_gas(e: &sdk::SdkError) -> bool {
    e.codespace == "sdk" && e.code == 11
}

fn to_coins(fee: &[FeeCoin]) -> R<Coins> {
    Ok(Coins(
        fee.iter()
            .map(|c| Ok(Coin::with_amount(c.denom.clone(), c.amount()?.clone())))
            .collect::<R<Vec<_>>>()?,
    ))
}

/// `DeductFees`.
fn deduct_fees(ctx: &mut Context<'_>, from: &[u8], fee: &[FeeCoin]) -> R<()> {
    let coins = to_coins(fee)?;
    if !coins.is_valid() {
        return Err(
            errs::insufficient_fee(format!("invalid fee amount: {}", coins_string(fee)?)).into(),
        );
    }
    if let Err(e) = x_bank::send_from_account_to_module(ctx, from, x_auth::FEE_COLLECTOR, &coins) {
        if is_out_of_gas(&e) {
            return Err(Fail::Err(e));
        }
        return Err(errs::insufficient_funds(e.log).into());
    }
    Ok(())
}
