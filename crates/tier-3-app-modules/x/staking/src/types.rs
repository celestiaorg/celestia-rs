//! Staking state types with exact gogoproto encodings.
//!
//! `math.Int` / `math.LegacyDec` customtype string fields are stored as
//! `Marshal()` text (`LegacyDec` as its scaled integer), while proto3 JSON
//! carries decimal strings; [`Repr`] selects which form a proto holds.

use std::fmt::Write as _;

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::{Int, LegacyDec};
use pbjson_types::{Duration, Timestamp};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::{Result, SdkError};

use crate::addr::PubKey;
use crate::errors;

/// How customtype string fields of a proto are represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repr {
    /// Binary store/wire form (`Marshal()` text).
    Wire,
    /// Proto3 JSON form (decimal strings).
    Json,
}

/// Maps a math error to the error Go would raise (a panic in most paths).
#[must_use]
pub fn math_err(e: math::MathError) -> SdkError {
    sdk::error::go_panic(e.to_string())
}

pub(crate) fn dec(s: &str, r: Repr) -> Result<LegacyDec> {
    match r {
        Repr::Wire => LegacyDec::unmarshal(s.as_bytes()).map_err(math_err),
        Repr::Json => LegacyDec::from_str_go(s).map_err(|e| errs::json_unmarshal(e.to_string())),
    }
}

pub(crate) fn int(s: &str, r: Repr) -> Result<Int> {
    match r {
        Repr::Wire => Int::unmarshal(s.as_bytes()).map_err(math_err),
        Repr::Json => Int::parse(s).map_err(|e| errs::json_unmarshal(e.to_string())),
    }
}

/// `LegacyDec.Marshal()` as a proto string.
#[must_use]
pub fn dec_str(d: &LegacyDec) -> String {
    String::from_utf8_lossy(&d.marshal()).into_owned()
}

/// `Int.Marshal()` as a proto string.
#[must_use]
pub fn int_str(i: &Int) -> String {
    i.to_string()
}

/// `sdk.DefaultPowerReduction` (10^6).
#[must_use]
pub fn power_reduction() -> Int {
    Int::from_u64(1_000_000)
}

/// `sdk.TokensToConsensusPower`.
pub fn tokens_to_consensus_power(tokens: &Int) -> Result<i64> {
    tokens
        .checked_quo(&power_reduction())
        .map_err(math_err)?
        .to_i64()
        .ok_or_else(|| sdk::error::go_panic("Int64() out of bound"))
}

/// `sdk.TokensFromConsensusPower`.
pub fn tokens_from_consensus_power(power: i64) -> Result<Int> {
    Int::from_i64(power)
        .checked_mul(&power_reduction())
        .map_err(math_err)
}

/// Validator bond status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BondStatus {
    /// `BOND_STATUS_UNSPECIFIED`.
    Unspecified = 0,
    /// `BOND_STATUS_UNBONDED`.
    Unbonded = 1,
    /// `BOND_STATUS_UNBONDING`.
    Unbonding = 2,
    /// `BOND_STATUS_BONDED`.
    Bonded = 3,
}

impl BondStatus {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Unbonded,
            2 => Self::Unbonding,
            3 => Self::Bonded,
            _ => Self::Unspecified,
        }
    }

    /// Go `BondStatus.String()`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "BOND_STATUS_UNSPECIFIED",
            Self::Unbonded => "BOND_STATUS_UNBONDED",
            Self::Unbonding => "BOND_STATUS_UNBONDING",
            Self::Bonded => "BOND_STATUS_BONDED",
        }
    }
}

/// `Commission`.
#[derive(Debug, Clone, PartialEq)]
pub struct Commission {
    /// Current rate.
    pub rate: LegacyDec,
    /// Maximum rate.
    pub max_rate: LegacyDec,
    /// Maximum daily increase.
    pub max_change_rate: LegacyDec,
    /// Last change time.
    pub update_time: Timestamp,
}

impl Commission {
    /// `CommissionRates.Validate`.
    pub fn validate_rates(&self) -> Result<()> {
        let one = LegacyDec::one();
        if self.max_rate.is_negative() {
            return Err(errors::commission_negative());
        }
        if self.max_rate > one {
            return Err(errors::commission_huge());
        }
        if self.rate.is_negative() {
            return Err(errors::commission_negative());
        }
        if self.rate > self.max_rate {
            return Err(errors::commission_gt_max_rate());
        }
        if self.max_change_rate.is_negative() {
            return Err(errors::commission_change_rate_negative());
        }
        if self.max_change_rate > self.max_rate {
            return Err(errors::commission_change_rate_gt_max_rate());
        }
        Ok(())
    }

    /// `Commission.ValidateNewRate`.
    pub fn validate_new_rate(&self, new_rate: &LegacyDec, block_time: &Timestamp) -> Result<()> {
        const DAY_NANOS: i64 = 24 * 3600 * 1_000_000_000;
        if sdk::time::sub(block_time, &self.update_time) < DAY_NANOS {
            return Err(errors::commission_update_time());
        }
        if new_rate.is_negative() {
            return Err(errors::commission_negative());
        }
        if *new_rate > self.max_rate {
            return Err(errors::commission_gt_max_rate());
        }
        if new_rate.checked_sub(&self.rate).map_err(math_err)? > self.max_change_rate {
            return Err(errors::commission_gt_max_change_rate());
        }
        Ok(())
    }

    fn rates_proto(&self) -> pb::CommissionRates {
        pb::CommissionRates {
            rate: dec_str(&self.rate),
            max_rate: dec_str(&self.max_rate),
            max_change_rate: dec_str(&self.max_change_rate),
        }
    }

    /// To proto.
    #[must_use]
    pub fn to_proto(&self) -> pb::Commission {
        pb::Commission {
            commission_rates: Some(self.rates_proto()),
            update_time: Some(self.update_time),
        }
    }

    /// From proto.
    pub fn from_proto(p: &pb::Commission, r: Repr) -> Result<Self> {
        let rates = p.commission_rates.clone().unwrap_or_default();
        Ok(Self {
            rate: dec(&rates.rate, r)?,
            max_rate: dec(&rates.max_rate, r)?,
            max_change_rate: dec(&rates.max_change_rate, r)?,
            update_time: p.update_time.unwrap_or_else(sdk::time::go_zero),
        })
    }

    /// Go `Commission.String()` (`proto.CompactTextString`).
    #[must_use]
    pub fn compact_text(&self) -> String {
        let r = self.rates_proto();
        let mut t = String::new();
        if self.update_time.seconds != 0 {
            let _ = write!(t, "seconds:{} ", self.update_time.seconds);
        }
        if self.update_time.nanos != 0 {
            let _ = write!(t, "nanos:{} ", self.update_time.nanos);
        }
        format!(
            "commission_rates:<rate:\"{}\" max_rate:\"{}\" max_change_rate:\"{}\" > update_time:<{t}> ",
            r.rate, r.max_rate, r.max_change_rate
        )
    }
}

/// `Validator`.
#[derive(Debug, Clone, PartialEq)]
pub struct Validator {
    /// Operator address (bech32 valoper).
    pub operator_address: String,
    /// Consensus public key.
    pub consensus_pubkey: Option<Any>,
    /// Jailed flag.
    pub jailed: bool,
    /// Bond status.
    pub status: BondStatus,
    /// Delegated tokens.
    pub tokens: Int,
    /// Total delegator shares.
    pub delegator_shares: LegacyDec,
    /// Description.
    pub description: pb::Description,
    /// Height at which unbonding started.
    pub unbonding_height: i64,
    /// Time at which unbonding completes.
    pub unbonding_time: Timestamp,
    /// Commission.
    pub commission: Commission,
    /// Minimum self delegation.
    pub min_self_delegation: Int,
    /// Unbonding on-hold reference count.
    pub unbonding_on_hold_ref_count: i64,
    /// Unbonding operation IDs.
    pub unbonding_ids: Vec<u64>,
}

impl Validator {
    /// `NewValidator`.
    #[must_use]
    pub fn new(operator: String, pubkey: Any, description: pb::Description) -> Self {
        Self {
            operator_address: operator,
            consensus_pubkey: Some(pubkey),
            jailed: false,
            status: BondStatus::Unbonded,
            tokens: Int::zero(),
            delegator_shares: LegacyDec::zero(),
            description,
            unbonding_height: 0,
            unbonding_time: sdk::time::unix_epoch(),
            commission: Commission {
                rate: LegacyDec::zero(),
                max_rate: LegacyDec::zero(),
                max_change_rate: LegacyDec::zero(),
                update_time: sdk::time::unix_epoch(),
            },
            min_self_delegation: Int::one(),
            unbonding_on_hold_ref_count: 0,
            unbonding_ids: Vec::new(),
        }
    }

    /// From proto.
    pub fn from_proto(p: &pb::Validator, r: Repr) -> Result<Self> {
        Ok(Self {
            operator_address: p.operator_address.clone(),
            consensus_pubkey: p.consensus_pubkey.clone(),
            jailed: p.jailed,
            status: BondStatus::from_i32(p.status),
            tokens: int(&p.tokens, r)?,
            delegator_shares: dec(&p.delegator_shares, r)?,
            description: p.description.clone().unwrap_or_default(),
            unbonding_height: p.unbonding_height,
            unbonding_time: p.unbonding_time.unwrap_or_else(sdk::time::go_zero),
            commission: Commission::from_proto(&p.commission.clone().unwrap_or_default(), r)?,
            min_self_delegation: int(&p.min_self_delegation, r)?,
            unbonding_on_hold_ref_count: p.unbonding_on_hold_ref_count,
            unbonding_ids: p.unbonding_ids.clone(),
        })
    }

    /// To proto (store form).
    #[must_use]
    pub fn to_proto(&self) -> pb::Validator {
        pb::Validator {
            operator_address: self.operator_address.clone(),
            consensus_pubkey: self.consensus_pubkey.clone(),
            jailed: self.jailed,
            status: self.status as i32,
            tokens: int_str(&self.tokens),
            delegator_shares: dec_str(&self.delegator_shares),
            description: Some(self.description.clone()),
            unbonding_height: self.unbonding_height,
            unbonding_time: Some(self.unbonding_time),
            commission: Some(self.commission.to_proto()),
            min_self_delegation: int_str(&self.min_self_delegation),
            unbonding_on_hold_ref_count: self.unbonding_on_hold_ref_count,
            unbonding_ids: self.unbonding_ids.clone(),
        }
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.to_proto().encode_to_vec()
    }

    /// Store decoding.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let p = pb::Validator::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))?;
        Self::from_proto(&p, Repr::Wire)
    }

    /// Operator address bytes.
    pub fn operator(&self) -> Result<Vec<u8>> {
        crate::addr::val_from_string(&self.operator_address)
    }

    /// Consensus public key.
    pub fn pubkey(&self) -> Result<PubKey> {
        let any = self
            .consensus_pubkey
            .as_ref()
            .ok_or_else(|| errs::invalid_type("expecting cryptotypes.PubKey, got <nil>"))?;
        PubKey::from_any(any)
    }

    /// `GetConsAddr`.
    pub fn cons_addr(&self) -> Result<Vec<u8>> {
        self.pubkey()?.address()
    }

    /// `IsBonded`.
    #[must_use]
    pub fn is_bonded(&self) -> bool {
        self.status == BondStatus::Bonded
    }

    /// `IsUnbonded`.
    #[must_use]
    pub fn is_unbonded(&self) -> bool {
        self.status == BondStatus::Unbonded
    }

    /// `IsUnbonding`.
    #[must_use]
    pub fn is_unbonding(&self) -> bool {
        self.status == BondStatus::Unbonding
    }

    /// `InvalidExRate`.
    #[must_use]
    pub fn invalid_ex_rate(&self) -> bool {
        self.tokens.is_zero() && self.delegator_shares.is_positive()
    }

    /// `TokensFromShares`.
    pub fn tokens_from_shares(&self, shares: &LegacyDec) -> Result<LegacyDec> {
        shares
            .checked_mul_int(&self.tokens)
            .and_then(|x| x.checked_quo(&self.delegator_shares))
            .map_err(math_err)
    }

    /// `TokensFromSharesTruncated`.
    pub fn tokens_from_shares_truncated(&self, shares: &LegacyDec) -> Result<LegacyDec> {
        shares
            .checked_mul_int(&self.tokens)
            .and_then(|x| x.checked_quo_truncate(&self.delegator_shares))
            .map_err(math_err)
    }

    /// `SharesFromTokens`.
    pub fn shares_from_tokens(&self, amt: &Int) -> Result<LegacyDec> {
        if self.tokens.is_zero() {
            return Err(errors::insufficient_shares());
        }
        self.delegator_shares
            .checked_mul_int(amt)
            .and_then(|x| x.checked_quo_int(&self.tokens))
            .map_err(math_err)
    }

    /// `SharesFromTokensTruncated`.
    pub fn shares_from_tokens_truncated(&self, amt: &Int) -> Result<LegacyDec> {
        if self.tokens.is_zero() {
            return Err(errors::insufficient_shares());
        }
        self.delegator_shares
            .checked_mul_int(amt)
            .and_then(|x| x.checked_quo_truncate(&LegacyDec::from_int(&self.tokens)))
            .map_err(math_err)
    }

    /// `ConsensusPower`.
    pub fn consensus_power(&self) -> Result<i64> {
        if self.is_bonded() {
            self.potential_consensus_power()
        } else {
            Ok(0)
        }
    }

    /// `PotentialConsensusPower`.
    pub fn potential_consensus_power(&self) -> Result<i64> {
        tokens_to_consensus_power(&self.tokens)
    }

    /// `AddTokensFromDel`: returns the issued shares.
    pub fn add_tokens_from_del(&mut self, amount: &Int) -> Result<LegacyDec> {
        let issued = if self.delegator_shares.is_zero() {
            LegacyDec::from_int(amount)
        } else {
            self.shares_from_tokens(amount)?
        };
        self.tokens = self.tokens.checked_add(amount).map_err(math_err)?;
        self.delegator_shares = self
            .delegator_shares
            .checked_add(&issued)
            .map_err(math_err)?;
        Ok(issued)
    }

    /// `RemoveTokens`.
    pub fn remove_tokens(&mut self, tokens: &Int) -> Result<()> {
        if tokens.is_negative() {
            return Err(sdk::error::go_panic(format!(
                "should not happen: trying to remove negative tokens {tokens}"
            )));
        }
        if self.tokens < *tokens {
            return Err(sdk::error::go_panic(format!(
                "should not happen: only have {} tokens, trying to remove {tokens}",
                self.tokens
            )));
        }
        self.tokens = self.tokens.checked_sub(tokens).map_err(math_err)?;
        Ok(())
    }

    /// `RemoveDelShares`: returns the removed tokens.
    pub fn remove_del_shares(&mut self, del_shares: &LegacyDec) -> Result<Int> {
        let remaining = self
            .delegator_shares
            .checked_sub(del_shares)
            .map_err(math_err)?;
        let issued = if remaining.is_zero() {
            std::mem::replace(&mut self.tokens, Int::zero())
        } else {
            let issued = self.tokens_from_shares(del_shares)?.truncate_int();
            self.tokens = self.tokens.checked_sub(&issued).map_err(math_err)?;
            if self.tokens.is_negative() {
                return Err(sdk::error::go_panic(
                    "attempting to remove more tokens than available in validator",
                ));
            }
            issued
        };
        self.delegator_shares = remaining;
        Ok(issued)
    }

    /// `ABCIValidatorUpdate`.
    pub fn abci_update(&self) -> Result<ValidatorUpdate> {
        Ok(ValidatorUpdate {
            pub_key: Some(self.pubkey()?.to_cmt()),
            power: self.consensus_power()?,
        })
    }

    /// `ABCIValidatorUpdateZero`.
    pub fn abci_update_zero(&self) -> Result<ValidatorUpdate> {
        Ok(ValidatorUpdate {
            pub_key: Some(self.pubkey()?.to_cmt()),
            power: 0,
        })
    }
}

/// `Delegation`.
#[derive(Debug, Clone, PartialEq)]
pub struct Delegation {
    /// Delegator (bech32).
    pub delegator_address: String,
    /// Validator (bech32 valoper).
    pub validator_address: String,
    /// Shares.
    pub shares: LegacyDec,
}

impl Delegation {
    /// From proto.
    pub fn from_proto(p: &pb::Delegation, r: Repr) -> Result<Self> {
        Ok(Self {
            delegator_address: p.delegator_address.clone(),
            validator_address: p.validator_address.clone(),
            shares: dec(&p.shares, r)?,
        })
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::Delegation {
            delegator_address: self.delegator_address.clone(),
            validator_address: self.validator_address.clone(),
            shares: dec_str(&self.shares),
        }
        .encode_to_vec()
    }

    /// Store decoding.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let p = pb::Delegation::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))?;
        Self::from_proto(&p, Repr::Wire)
    }
}

/// `UnbondingDelegationEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct UbdEntry {
    /// Creation height.
    pub creation_height: i64,
    /// Completion time.
    pub completion_time: Timestamp,
    /// Initial balance.
    pub initial_balance: Int,
    /// Remaining balance.
    pub balance: Int,
    /// Unbonding ID.
    pub unbonding_id: u64,
    /// On-hold reference count.
    pub unbonding_on_hold_ref_count: i64,
}

impl UbdEntry {
    /// `IsMature`.
    #[must_use]
    pub fn is_mature(&self, now: &Timestamp) -> bool {
        !sdk::time::after(&self.completion_time, now)
    }

    /// `OnHold`.
    #[must_use]
    pub fn on_hold(&self) -> bool {
        self.unbonding_on_hold_ref_count > 0
    }
}

/// `UnbondingDelegation`.
#[derive(Debug, Clone, PartialEq)]
pub struct UnbondingDelegation {
    /// Delegator (bech32).
    pub delegator_address: String,
    /// Validator (bech32 valoper).
    pub validator_address: String,
    /// Entries.
    pub entries: Vec<UbdEntry>,
}

impl UnbondingDelegation {
    /// From proto.
    pub fn from_proto(p: &pb::UnbondingDelegation, r: Repr) -> Result<Self> {
        Ok(Self {
            delegator_address: p.delegator_address.clone(),
            validator_address: p.validator_address.clone(),
            entries: p
                .entries
                .iter()
                .map(|e| {
                    Ok(UbdEntry {
                        creation_height: e.creation_height,
                        completion_time: e.completion_time.unwrap_or_else(sdk::time::go_zero),
                        initial_balance: int(&e.initial_balance, r)?,
                        balance: int(&e.balance, r)?,
                        unbonding_id: e.unbonding_id,
                        unbonding_on_hold_ref_count: e.unbonding_on_hold_ref_count,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::UnbondingDelegation {
            delegator_address: self.delegator_address.clone(),
            validator_address: self.validator_address.clone(),
            entries: self
                .entries
                .iter()
                .map(|e| pb::UnbondingDelegationEntry {
                    creation_height: e.creation_height,
                    completion_time: Some(e.completion_time),
                    initial_balance: int_str(&e.initial_balance),
                    balance: int_str(&e.balance),
                    unbonding_id: e.unbonding_id,
                    unbonding_on_hold_ref_count: e.unbonding_on_hold_ref_count,
                })
                .collect(),
        }
        .encode_to_vec()
    }

    /// Store decoding.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let p =
            pb::UnbondingDelegation::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))?;
        Self::from_proto(&p, Repr::Wire)
    }

    /// `AddEntry`: returns true when a new entry was appended.
    pub fn add_entry(
        &mut self,
        creation_height: i64,
        min_time: Timestamp,
        balance: &Int,
        id: u64,
    ) -> Result<bool> {
        if let Some(e) = self.entries.iter_mut().find(|e| {
            e.creation_height == creation_height
                && sdk::time::cmp(&e.completion_time, &min_time).is_eq()
        }) {
            e.balance = e.balance.checked_add(balance).map_err(math_err)?;
            e.initial_balance = e.initial_balance.checked_add(balance).map_err(math_err)?;
            return Ok(false);
        }
        self.entries.push(UbdEntry {
            creation_height,
            completion_time: min_time,
            initial_balance: balance.clone(),
            balance: balance.clone(),
            unbonding_id: id,
            unbonding_on_hold_ref_count: 0,
        });
        Ok(true)
    }
}

/// `RedelegationEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct RedEntry {
    /// Creation height.
    pub creation_height: i64,
    /// Completion time.
    pub completion_time: Timestamp,
    /// Initial balance.
    pub initial_balance: Int,
    /// Destination shares.
    pub shares_dst: LegacyDec,
    /// Unbonding ID.
    pub unbonding_id: u64,
    /// On-hold reference count.
    pub unbonding_on_hold_ref_count: i64,
}

impl RedEntry {
    /// `IsMature`.
    #[must_use]
    pub fn is_mature(&self, now: &Timestamp) -> bool {
        !sdk::time::after(&self.completion_time, now)
    }

    /// `OnHold`.
    #[must_use]
    pub fn on_hold(&self) -> bool {
        self.unbonding_on_hold_ref_count > 0
    }
}

/// `Redelegation`.
#[derive(Debug, Clone, PartialEq)]
pub struct Redelegation {
    /// Delegator (bech32).
    pub delegator_address: String,
    /// Source validator (bech32 valoper).
    pub validator_src_address: String,
    /// Destination validator (bech32 valoper).
    pub validator_dst_address: String,
    /// Entries.
    pub entries: Vec<RedEntry>,
}

impl Redelegation {
    /// From proto.
    pub fn from_proto(p: &pb::Redelegation, r: Repr) -> Result<Self> {
        Ok(Self {
            delegator_address: p.delegator_address.clone(),
            validator_src_address: p.validator_src_address.clone(),
            validator_dst_address: p.validator_dst_address.clone(),
            entries: p
                .entries
                .iter()
                .map(|e| {
                    Ok(RedEntry {
                        creation_height: e.creation_height,
                        completion_time: e.completion_time.unwrap_or_else(sdk::time::go_zero),
                        initial_balance: int(&e.initial_balance, r)?,
                        shares_dst: dec(&e.shares_dst, r)?,
                        unbonding_id: e.unbonding_id,
                        unbonding_on_hold_ref_count: e.unbonding_on_hold_ref_count,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::Redelegation {
            delegator_address: self.delegator_address.clone(),
            validator_src_address: self.validator_src_address.clone(),
            validator_dst_address: self.validator_dst_address.clone(),
            entries: self
                .entries
                .iter()
                .map(|e| pb::RedelegationEntry {
                    creation_height: e.creation_height,
                    completion_time: Some(e.completion_time),
                    initial_balance: int_str(&e.initial_balance),
                    shares_dst: dec_str(&e.shares_dst),
                    unbonding_id: e.unbonding_id,
                    unbonding_on_hold_ref_count: e.unbonding_on_hold_ref_count,
                })
                .collect(),
        }
        .encode_to_vec()
    }

    /// Store decoding.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let p = pb::Redelegation::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))?;
        Self::from_proto(&p, Repr::Wire)
    }
}

/// Staking `Params`.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// Unbonding period.
    pub unbonding_time: Duration,
    /// Maximum bonded validators.
    pub max_validators: u32,
    /// Maximum unbonding/redelegation entries per pair.
    pub max_entries: u32,
    /// Historical entries kept.
    pub historical_entries: u32,
    /// Bond denom.
    pub bond_denom: String,
    /// Minimum commission rate.
    pub min_commission_rate: LegacyDec,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            unbonding_time: Duration::default(),
            max_validators: 0,
            max_entries: 0,
            historical_entries: 0,
            bond_denom: String::new(),
            min_commission_rate: LegacyDec::zero(),
        }
    }
}

impl Params {
    /// From proto.
    pub fn from_proto(p: &pb::Params, r: Repr) -> Result<Self> {
        Ok(Self {
            unbonding_time: p.unbonding_time.unwrap_or_default(),
            max_validators: p.max_validators,
            max_entries: p.max_entries,
            historical_entries: p.historical_entries,
            bond_denom: p.bond_denom.clone(),
            min_commission_rate: if p.min_commission_rate.is_empty() && r == Repr::Wire {
                LegacyDec::zero()
            } else {
                dec(&p.min_commission_rate, r)?
            },
        })
    }

    /// To proto (store form).
    #[must_use]
    pub fn to_proto(&self) -> pb::Params {
        pb::Params {
            unbonding_time: Some(self.unbonding_time),
            max_validators: self.max_validators,
            max_entries: self.max_entries,
            historical_entries: self.historical_entries,
            bond_denom: self.bond_denom.clone(),
            min_commission_rate: dec_str(&self.min_commission_rate),
        }
    }

    /// `Params.Validate` (errors are unregistered in Go; their text is
    /// stored as a failed proposal's `failed_reason`, security.md 69).
    pub fn validate(&self) -> Result<()> {
        fn e(s: impl Into<String>) -> SdkError {
            sdk::error::internal(s)
        }
        let nanos = sdk::time::duration_nanos(&self.unbonding_time);
        if nanos <= 0 {
            // `fmt.Errorf("... must be positive: %d", v)` of a
            // `time.Duration`: its nanoseconds.
            return Err(e(format!("unbonding time must be positive: {nanos}")));
        }
        if self.max_validators == 0 {
            return Err(e("max validators must be positive: 0"));
        }
        if self.max_entries == 0 {
            return Err(e("max entries must be positive: 0"));
        }
        if self.bond_denom.trim().is_empty() {
            return Err(e("bond denom cannot be blank"));
        }
        if !sdk::coins::is_valid_denom(&self.bond_denom) {
            return Err(e(format!("invalid denom: {}", self.bond_denom)));
        }
        if self.min_commission_rate.is_negative() {
            return Err(e(format!(
                "minimum commission rate cannot be negative: {}",
                self.min_commission_rate
            )));
        }
        if self.min_commission_rate > LegacyDec::one() {
            return Err(e(format!(
                "minimum commission rate cannot be greater than 100%: {}",
                self.min_commission_rate
            )));
        }
        Ok(())
    }

    /// `Params.Validate` while retaining a nil minimum commission rate.
    pub fn validate_wire(&self, wire: &pb::Params) -> Result<()> {
        self.validate()?;
        if wire.min_commission_rate.is_empty() {
            return Err(sdk::error::internal(
                "minimum commission rate cannot be nil: <nil>",
            ));
        }
        Ok(())
    }
}

/// `Description.EnsureLength`.
pub fn ensure_description_length(d: &pb::Description) -> Result<()> {
    let checks = [
        ("moniker", gostr::go_len(&d.moniker), 70),
        ("identity", gostr::go_len(&d.identity), 3000),
        ("website", gostr::go_len(&d.website), 140),
        ("security contact", gostr::go_len(&d.security_contact), 140),
        ("details", gostr::go_len(&d.details), 280),
    ];
    for (name, got, max) in checks {
        if got > max {
            return Err(errs::invalid_request(format!(
                "invalid {name} length; got: {got}, max: {max}"
            )));
        }
    }
    Ok(())
}

/// `Description.UpdateDescription`.
pub fn update_description(old: &pb::Description, new: &pb::Description) -> Result<pb::Description> {
    const KEEP: &str = "[do-not-modify]";
    let pick = |n: &String, o: &String| if n == KEEP { o.clone() } else { n.clone() };
    let d = pb::Description {
        moniker: pick(&new.moniker, &old.moniker),
        identity: pick(&new.identity, &old.identity),
        website: pick(&new.website, &old.website),
        security_contact: pick(&new.security_contact, &old.security_contact),
        details: pick(&new.details, &old.details),
    };
    ensure_description_length(&d)?;
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_roundtrip_matches_fixture() {
        // staking 0x21 value at height 1 of the `empty` devnet scenario.
        let hex = "0a3663656c657374696176616c6f70657231736e34617230306b34746e39726838326e6e7878716a72613474357261703764796b6b7a643912430a1d2f636f736d6f732e63727970746f2e656432353531392e5075624b657912220a2067178e96d872300264d24c8bb80c59f63427efed4cf0f5dff43d435c132a54f120032a0a35303030303030303030321c353030303030303030303030303030303030303030303030303030303a060a0476616c304a00524b0a3b0a1231303030303030303030303030303030303012123230303030303030303030303030303030301a113130303030303030303030303030303030120c08aba6bcd60610c6edb1ce025a0131";
        let b: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let v = Validator::decode(&b).unwrap();
        assert_eq!(v.tokens, Int::from_u64(5_000_000_000));
        assert_eq!(v.consensus_power().unwrap(), 5000);
        assert_eq!(v.commission.rate.to_string(), "0.100000000000000000");
        assert_eq!(v.encode(), b);
        assert_eq!(
            crate::addr::cons_to_string(&v.cons_addr().unwrap()).unwrap(),
            "celestiavalcons130ac546luk0yh33fteucdqpzxy575z02jyus7k"
        );
    }

    // Found by `fuzz-diff deliver` (security.md 69): a gov proposal whose
    // staking `MsgUpdateParams` fails stores Go's error text as the
    // proposal's `failed_reason` (cosmos-sdk `x/staking/types/params.go`).
    #[test]
    fn params_validate_errors_match_go() {
        let log = |f: &dyn Fn(&mut Params)| {
            let mut p = Params {
                unbonding_time: Duration {
                    seconds: 600,
                    nanos: 0,
                },
                max_validators: 100,
                max_entries: 7,
                historical_entries: 0,
                bond_denom: "utia".into(),
                min_commission_rate: LegacyDec::zero(),
            };
            p.validate().unwrap();
            f(&mut p);
            p.validate().unwrap_err().log
        };
        assert_eq!(
            log(&|p| p.max_validators = 0),
            "max validators must be positive: 0"
        );
        assert_eq!(
            log(&|p| p.max_entries = 0),
            "max entries must be positive: 0"
        );
        assert_eq!(
            log(&|p| p.unbonding_time = Duration {
                seconds: -3600,
                nanos: 0
            }),
            "unbonding time must be positive: -3600000000000"
        );
        assert_eq!(
            log(&|p| p.min_commission_rate = "1.5".parse::<LegacyDec>().unwrap()),
            "minimum commission rate cannot be greater than 100%: 1.500000000000000000"
        );

        let p = Params {
            unbonding_time: Duration {
                seconds: 600,
                nanos: 0,
            },
            max_validators: 100,
            max_entries: 7,
            historical_entries: 0,
            bond_denom: "utia".into(),
            min_commission_rate: LegacyDec::zero(),
        };
        let mut wire = p.to_proto();
        wire.min_commission_rate.clear();
        assert_eq!(
            p.validate_wire(&wire).unwrap_err().log,
            "minimum commission rate cannot be nil: <nil>"
        );
        wire.min_commission_rate = "0".into();
        p.validate_wire(&wire).unwrap();
        wire.min_commission_rate.clear();
        let mut invalid = p.clone();
        invalid.bond_denom.clear();
        assert_eq!(
            invalid.validate_wire(&wire).unwrap_err().log,
            "bond denom cannot be blank"
        );
    }
}
