//! celestia-app v10 `x/mint`.
//!
//! Store layout: `"Minter"` → `Minter`, `"GenesisTime"` → `GenesisTime`.
//! `LegacyDec` fields are stored in their gogoproto binary form (the
//! `10^18`-scaled integer as a decimal string).
//!
//! Inflation is a function of whole years since genesis:
//! `0.0267 * (1 - 0.067)^years`, floored at `0.015`. Each block mints
//! `annual_provisions * elapsed_ns / ns_per_year` (truncated) to the fee
//! collector.

use celestia_proto::celestia::mint::v1::{GenesisState, GenesisTime, Minter as PbMinter};
use math::{Int, LegacyDec};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::undefined;
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as sdkerr;
use sdk::event::event;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "mint";
/// Module name.
pub const MODULE_NAME: &str = "mint";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `KeyMinter`.
pub const KEY_MINTER: &[u8] = b"Minter";
/// `KeyGenesisTime`.
pub const KEY_GENESIS_TIME: &[u8] = b"GenesisTime";
/// `NanosecondsPerYear` (365.2425 days).
pub const NANOSECONDS_PER_YEAR: i64 = 31_556_952_000_000_000;
/// `appconsts.BondDenom`.
pub const BOND_DENOM: &str = "utia";

fn math_err(e: math::MathError) -> SdkError {
    undefined(e.to_string())
}

/// `InitialInflationRateAsDec` (0.0267).
#[must_use]
pub fn initial_inflation_rate() -> LegacyDec {
    LegacyDec::new_with_prec(267, 4).unwrap_or_default()
}

/// `DisinflationRateAsDec` (0.067).
#[must_use]
pub fn disinflation_rate() -> LegacyDec {
    LegacyDec::new_with_prec(67, 3).unwrap_or_default()
}

/// `TargetInflationRateAsDec` (0.015).
#[must_use]
pub fn target_inflation_rate() -> LegacyDec {
    LegacyDec::new_with_prec(15, 3).unwrap_or_default()
}

/// The module's `Minter` with typed decimals.
#[derive(Debug, Clone, PartialEq)]
pub struct Minter {
    /// Current annual inflation rate.
    pub inflation_rate: LegacyDec,
    /// Current annual expected provisions.
    pub annual_provisions: LegacyDec,
    /// Time of the previous block (`None` before the first BeginBlock).
    pub previous_block_time: Option<Timestamp>,
    /// Denomination minted.
    pub bond_denom: String,
}

impl Minter {
    /// `DefaultMinter` with `bond_denom`.
    #[must_use]
    pub fn new_default(bond_denom: impl Into<String>) -> Self {
        Self {
            inflation_rate: initial_inflation_rate(),
            annual_provisions: LegacyDec::zero(),
            previous_block_time: None,
            bond_denom: bond_denom.into(),
        }
    }

    /// Protobuf form.
    #[must_use]
    pub fn to_proto(&self) -> PbMinter {
        PbMinter {
            inflation_rate: String::from_utf8_lossy(&self.inflation_rate.marshal()).into_owned(),
            annual_provisions: String::from_utf8_lossy(&self.annual_provisions.marshal())
                .into_owned(),
            previous_block_time: self.previous_block_time,
            bond_denom: self.bond_denom.clone(),
        }
    }

    /// From the protobuf form.
    pub fn from_proto(p: &PbMinter) -> Result<Self> {
        Ok(Self {
            inflation_rate: LegacyDec::unmarshal(p.inflation_rate.as_bytes()).map_err(math_err)?,
            annual_provisions: LegacyDec::unmarshal(p.annual_provisions.as_bytes())
                .map_err(math_err)?,
            previous_block_time: p.previous_block_time,
            bond_denom: p.bond_denom.clone(),
        })
    }

    /// `CalculateInflationRate` at `block_time`.
    pub fn calculate_inflation_rate(
        &self,
        genesis_time: &Timestamp,
        block_time: &Timestamp,
    ) -> Result<LegacyDec> {
        calculate_inflation_rate(genesis_time, block_time)
    }

    /// `CalculateBlockProvision`.
    pub fn calculate_block_provision(
        &self,
        current: &Timestamp,
        previous: &Timestamp,
    ) -> Result<Coin> {
        if sdk::time::to_nanos(current) < sdk::time::to_nanos(previous) {
            return Err(undefined(format!(
                "current time {} cannot be before previous time {}",
                sdk::time::format_rfc3339_nano(current),
                sdk::time::format_rfc3339_nano(previous)
            )));
        }
        let elapsed = sdk::time::sub(current, previous);
        let portion = LegacyDec::new(elapsed)
            .checked_quo(&LegacyDec::new(NANOSECONDS_PER_YEAR))
            .map_err(math_err)?;
        let provision = self
            .annual_provisions
            .checked_mul(&portion)
            .map_err(math_err)?;
        Ok(Coin::with_amount(
            self.bond_denom.clone(),
            provision.truncate_int(),
        ))
    }
}

/// `yearsSinceGenesis` (whole years, 0 before genesis).
#[must_use]
pub fn years_since_genesis(genesis: &Timestamp, current: &Timestamp) -> i64 {
    if sdk::time::to_nanos(current) < sdk::time::to_nanos(genesis) {
        return 0;
    }
    sdk::time::sub(current, genesis) / NANOSECONDS_PER_YEAR
}

/// `Minter.CalculateInflationRate` (independent of the minter's state).
pub fn calculate_inflation_rate(genesis: &Timestamp, block_time: &Timestamp) -> Result<LegacyDec> {
    let years = years_since_genesis(genesis, block_time);
    let factor = LegacyDec::one()
        .checked_sub(&disinflation_rate())
        .and_then(|d| d.checked_power(years.unsigned_abs()))
        .map_err(math_err)?;
    let rate = initial_inflation_rate()
        .checked_mul(&factor)
        .map_err(math_err)?;
    let target = target_inflation_rate();
    Ok(if rate < target { target } else { rate })
}

/// `Keeper.GetMinter` (Go panics when unset).
pub fn get_minter(ctx: &mut Context<'_>) -> Result<Minter> {
    let bz = ctx
        .get(STORE_KEY, KEY_MINTER)?
        .ok_or_else(|| sdkerr::logic("stored minter should not have been nil"))?;
    let p = PbMinter::decode(bz.as_slice()).map_err(|e| undefined(e.to_string()))?;
    Minter::from_proto(&p)
}

/// `Keeper.SetMinter`.
pub fn set_minter(ctx: &mut Context<'_>, m: &Minter) -> Result<()> {
    ctx.set(STORE_KEY, KEY_MINTER.to_vec(), m.to_proto().encode_to_vec())
}

/// `Keeper.GetGenesisTime` (Go panics when unset).
pub fn get_genesis_time(ctx: &mut Context<'_>) -> Result<Timestamp> {
    let bz = ctx
        .get(STORE_KEY, KEY_GENESIS_TIME)?
        .ok_or_else(|| sdkerr::logic("stored genesis time should not have been nil"))?;
    let gt = GenesisTime::decode(bz.as_slice()).map_err(|e| undefined(e.to_string()))?;
    // A nil pointer would panic on dereference in Go.
    gt.genesis_time
        .ok_or_else(|| sdkerr::logic("genesis time is nil"))
}

/// `Keeper.SetGenesisTime`.
pub fn set_genesis_time(ctx: &mut Context<'_>, t: &Timestamp) -> Result<()> {
    let gt = GenesisTime {
        genesis_time: Some(*t),
    };
    ctx.set(STORE_KEY, KEY_GENESIS_TIME.to_vec(), gt.encode_to_vec())
}

/// `InitGenesis`: default minter with the genesis bond denom, genesis time
/// = the InitChain block time, and the mint module account.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    set_minter(ctx, &Minter::new_default(gs.bond_denom.clone()))?;
    let t = ctx.header.time;
    set_genesis_time(ctx, &t)?;
    x_auth::get_module_account(ctx, MODULE_NAME)?;
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// `StakingKeeper.StakingTokenSupply`: the bank supply of the staking bond
/// denom (read from the staking params, as in Go).
fn staking_token_supply(ctx: &mut Context<'_>) -> Result<Int> {
    let denom = x_staking::store::bond_denom(ctx)?;
    Ok(x_bank::get_supply(ctx, &denom)?.amount)
}

fn maybe_update_minter(ctx: &mut Context<'_>) -> Result<()> {
    let mut minter = get_minter(ctx)?;
    let genesis = get_genesis_time(ctx)?;
    let block_time = ctx.header.time;
    let rate = calculate_inflation_rate(&genesis, &block_time)?;
    if rate == minter.inflation_rate && !minter.annual_provisions.is_zero() {
        return Ok(());
    }
    let supply = staking_token_supply(ctx)?;
    minter.annual_provisions = rate.checked_mul_int(&supply).map_err(math_err)?;
    minter.inflation_rate = rate;
    set_minter(ctx, &minter)
}

fn mint_block_provision(ctx: &mut Context<'_>) -> Result<()> {
    let minter = get_minter(ctx)?;
    let Some(prev) = minter.previous_block_time else {
        return Ok(());
    };
    let block_time = ctx.header.time;
    let coin = minter.calculate_block_provision(&block_time, &prev)?;
    let coins = Coins::sanitized(vec![coin.clone()])?;
    if !coins.is_empty() {
        x_bank::mint_coins(ctx, MODULE_NAME, &coins)?;
    }
    x_bank::send_from_module_to_module(ctx, MODULE_NAME, x_auth::FEE_COLLECTOR, &coins)?;
    ctx.events.emit(event(
        MODULE_NAME,
        [
            ("inflation_rate", minter.inflation_rate.to_string()),
            ("annual_provisions", minter.annual_provisions.to_string()),
            ("amount", coin.amount.to_string()),
        ],
    ));
    Ok(())
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    maybe_update_minter(ctx)?;
    mint_block_provision(ctx)?;
    let mut minter = get_minter(ctx)?;
    minter.previous_block_time = Some(ctx.header.time);
    set_minter(ctx, &minter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds: i64, nanos: i32) -> Timestamp {
        Timestamp { seconds, nanos }
    }

    #[test]
    fn devnet_minter_encoding() -> Result<()> {
        // Height-1 Minter of the empty devnet: supply 2e12 utia.
        let m = Minter {
            inflation_rate: initial_inflation_rate(),
            annual_provisions: initial_inflation_rate()
                .checked_mul_int(&Int::from_i64(2_000_000_000_000))
                .map_err(math_err)?,
            previous_block_time: Some(ts(1_791_955_755, 701_265_606)),
            bond_denom: "utia".into(),
        };
        let bz = m.to_proto().encode_to_vec();
        let want = "0a113236373030303030303030303030303030121d3533343030303030303030303030303030303030303030303030303030220c08aba6bcd60610c6edb1ce022a0475746961";
        let got: String = bz.iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        });
        assert_eq!(got, want);
        Ok(())
    }

    #[test]
    fn schedule_floors_at_target() -> Result<()> {
        let g = ts(0, 0);
        let year = NANOSECONDS_PER_YEAR / 1_000_000_000;
        assert_eq!(
            calculate_inflation_rate(&g, &ts(year - 1, 0))?.to_string(),
            "0.026700000000000000"
        );
        assert_eq!(
            calculate_inflation_rate(&g, &ts(year, 0))?.to_string(),
            "0.024911100000000000"
        );
        assert_eq!(
            calculate_inflation_rate(&g, &ts(100 * year, 0))?,
            target_inflation_rate()
        );
        // Before genesis counts as year 0.
        assert_eq!(
            calculate_inflation_rate(&g, &ts(-5, 0))?,
            initial_inflation_rate()
        );
        Ok(())
    }
}
