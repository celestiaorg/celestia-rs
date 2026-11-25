//! Validator set members (`types/validator.go`, `types/valset.go`):
//! parsed bridge validators, the power difference between two sets and
//! valset construction.

use math::{Int, LegacyDec};
use pbjson_types::Timestamp;
use sdk::Result;

use crate::errors;
use crate::types::{
    BridgeValidator, EvmAddress, Valset, checksum_hex, hex_to_address, is_hex_address,
};

/// `InternalBridgeValidator`: a member with a parsed EVM address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalBridgeValidator {
    /// Power.
    pub power: u64,
    /// EVM address.
    pub evm_address: EvmAddress,
}

impl InternalBridgeValidator {
    /// `NewInternalBridgeValidator`: the address must be hex and the power
    /// non-zero.
    pub fn new(b: &BridgeValidator) -> Result<Self> {
        if !is_hex_address(&b.evm_address) {
            return Err(errors::evm_address_not_hex());
        }
        if b.power == 0 {
            return Err(errors::empty("power").wrap("invalid bridge validator"));
        }
        Ok(Self {
            power: b.power,
            evm_address: hex_to_address(&b.evm_address),
        })
    }

    /// `ToExternal`.
    #[must_use]
    pub fn to_external(&self) -> BridgeValidator {
        BridgeValidator {
            power: self.power,
            evm_address: checksum_hex(&self.evm_address),
        }
    }
}

/// `BridgeValidators.ToInternal`.
pub fn to_internal(members: &[BridgeValidator]) -> Result<Vec<InternalBridgeValidator>> {
    members
        .iter()
        .enumerate()
        .map(|(i, m)| InternalBridgeValidator::new(m).map_err(|e| e.wrap(format!("member {i}"))))
        .collect()
}

fn math_err(e: math::MathError) -> sdk::SdkError {
    sdk::error::sdk::logic(e.to_string())
}

/// `InternalBridgeValidators.PowerDiff`: the sum of absolute power changes
/// per EVM address over `math.MaxUint32`. Go sums over a map; the sum of
/// exact decimals does not depend on the order.
pub fn power_diff(
    a: &[InternalBridgeValidator],
    b: &[InternalBridgeValidator],
) -> Result<LegacyDec> {
    let dec = |p: u64| LegacyDec::from_int(&Int::from_u64(p));
    let mut powers = std::collections::BTreeMap::<EvmAddress, LegacyDec>::new();
    for v in a {
        powers.insert(v.evm_address, dec(v.power));
    }
    for v in b {
        let p = dec(v.power);
        let entry = match powers.remove(&v.evm_address) {
            Some(cur) => cur.checked_sub(&p).map_err(math_err)?,
            None => p.neg(),
        };
        powers.insert(v.evm_address, entry);
    }
    let mut delta = LegacyDec::zero();
    for v in powers.values() {
        delta = delta.checked_add(&v.abs()).map_err(math_err)?;
    }
    delta
        .checked_quo(&LegacyDec::new(i64::from(u32::MAX)))
        .map_err(math_err)
}

/// `NewValset`: validates the members (non-empty, non-zero powers, no
/// duplicate address), then sorts them by power descending and, for
/// equal powers, by checksummed hex ascending.
pub fn new_valset(
    nonce: u64,
    height: u64,
    mut members: Vec<InternalBridgeValidator>,
    time: Timestamp,
) -> Result<Valset> {
    validate_members(&members).map_err(|e| e.wrap("invalid members"))?;
    let mut keyed: Vec<(String, InternalBridgeValidator)> = members
        .drain(..)
        .map(|m| (checksum_hex(&m.evm_address), m))
        .collect();
    keyed.sort_by(|(ha, a), (hb, b)| b.power.cmp(&a.power).then_with(|| ha.cmp(hb)));
    Ok(Valset {
        nonce,
        members: keyed
            .into_iter()
            .map(|(evm_address, m)| BridgeValidator {
                power: m.power,
                evm_address,
            })
            .collect(),
        height,
        time: Some(time),
    })
}

/// `InternalBridgeValidators.ValidateBasic`.
fn validate_members(m: &[InternalBridgeValidator]) -> Result<()> {
    if m.is_empty() {
        return Err(errors::empty(""));
    }
    for (i, v) in m.iter().enumerate() {
        if v.power == 0 {
            return Err(errors::empty("power")
                .wrap("invalid bridge validator")
                .wrap(format!("member {i}")));
        }
    }
    let unique: std::collections::BTreeSet<_> = m.iter().map(|v| v.evm_address).collect();
    if unique.len() != m.len() {
        return Err(errors::duplicate("addresses"));
    }
    Ok(())
}
