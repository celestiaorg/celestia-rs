//! Store keys (`x/staking/types/keys.go`).

use pbjson_types::Timestamp;
use sdk::codec::length_prefixed as lp;

/// Bonded validator powers (`0x11 || len || valAddr`).
pub const LAST_VALIDATOR_POWER: u8 = 0x11;
/// Last total power.
pub const LAST_TOTAL_POWER: &[u8] = &[0x12];
/// Validators.
pub const VALIDATORS: u8 = 0x21;
/// Validator by consensus address.
pub const VALIDATORS_BY_CONS_ADDR: u8 = 0x22;
/// Validators by power index.
pub const VALIDATORS_BY_POWER_INDEX: u8 = 0x23;
/// Delegations.
pub const DELEGATION: u8 = 0x31;
/// Unbonding delegations.
pub const UNBONDING_DELEGATION: u8 = 0x32;
/// Unbonding delegations by validator.
pub const UNBONDING_DELEGATION_BY_VAL_INDEX: u8 = 0x33;
/// Redelegations.
pub const REDELEGATION: u8 = 0x34;
/// Redelegations by source validator.
pub const REDELEGATION_BY_VAL_SRC_INDEX: u8 = 0x35;
/// Redelegations by destination validator.
pub const REDELEGATION_BY_VAL_DST_INDEX: u8 = 0x36;
/// Unbonding ID counter.
pub const UNBONDING_ID: &[u8] = &[0x37];
/// Unbonding operation index by ID.
pub const UNBONDING_INDEX: u8 = 0x38;
/// Unbonding operation type by ID.
pub const UNBONDING_TYPE: u8 = 0x39;
/// Unbonding delegation queue.
pub const UNBONDING_QUEUE: u8 = 0x41;
/// Redelegation queue.
pub const REDELEGATION_QUEUE: u8 = 0x42;
/// Validator queue.
pub const VALIDATOR_QUEUE: u8 = 0x43;
/// Historical info.
pub const HISTORICAL_INFO: u8 = 0x50;
/// Params.
pub const PARAMS: &[u8] = &[0x51];
/// End-block validator updates.
pub const VALIDATOR_UPDATES: &[u8] = &[0x61];
/// Delegations by validator.
pub const DELEGATION_BY_VAL_INDEX: u8 = 0x71;
/// Lazy migration cursor for the delegations-by-validator index.
pub const NEXT_MIGRATE_DELEGATIONS_BY_VALIDATOR_INDEX: &[u8] = &[0x81];
/// Lazy migration cursor for historical info keys.
pub const NEXT_MIGRATE_HISTORICAL_INFO: &[u8] = &[0x82];

fn cat(prefix: u8, parts: &[&[u8]]) -> Vec<u8> {
    let mut k = vec![prefix];
    for p in parts {
        k.extend_from_slice(p);
    }
    k
}

/// `GetValidatorKey`.
#[must_use]
pub fn validator(op: &[u8]) -> Vec<u8> {
    cat(VALIDATORS, &[&lp(op)])
}

/// `GetValidatorByConsAddrKey`.
#[must_use]
pub fn validator_by_cons_addr(cons: &[u8]) -> Vec<u8> {
    cat(VALIDATORS_BY_CONS_ADDR, &[&lp(cons)])
}

/// `GetLastValidatorPowerKey`.
#[must_use]
pub fn last_validator_power(op: &[u8]) -> Vec<u8> {
    cat(LAST_VALIDATOR_POWER, &[&lp(op)])
}

/// `GetValidatorsByPowerIndexKey`.
#[must_use]
pub fn power_index(consensus_power: i64, op: &[u8]) -> Vec<u8> {
    let inv: Vec<u8> = op.iter().map(|b| !b).collect();
    #[allow(clippy::cast_sign_loss)]
    let power = (consensus_power as u64).to_be_bytes();
    cat(VALIDATORS_BY_POWER_INDEX, &[&power, &lp(&inv)])
}

/// `ParseValidatorPowerRankKey`.
#[must_use]
pub fn parse_power_index(key: &[u8]) -> Vec<u8> {
    key.get(10..)
        .unwrap_or_default()
        .iter()
        .map(|b| !b)
        .collect()
}

/// Strips `prefix || len` from a `prefix || len || addr` key.
#[must_use]
pub fn address_from_key(key: &[u8]) -> Vec<u8> {
    key.get(2..).unwrap_or_default().to_vec()
}

/// `GetDelegationsKey`.
#[must_use]
pub fn delegations(del: &[u8]) -> Vec<u8> {
    cat(DELEGATION, &[&lp(del)])
}

/// `GetDelegationKey`.
#[must_use]
pub fn delegation(del: &[u8], val: &[u8]) -> Vec<u8> {
    cat(DELEGATION, &[&lp(del), &lp(val)])
}

/// `GetDelegationsByValPrefixKey`.
#[must_use]
pub fn delegations_by_val(val: &[u8]) -> Vec<u8> {
    cat(DELEGATION_BY_VAL_INDEX, &[&lp(val)])
}

/// `GetDelegationsByValKey`.
#[must_use]
pub fn delegation_by_val(val: &[u8], del: &[u8]) -> Vec<u8> {
    cat(DELEGATION_BY_VAL_INDEX, &[&lp(val), del])
}

/// `ParseDelegationsByValKey`: `(val, del)`.
#[must_use]
pub fn parse_delegation_by_val(key: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let rest = key.get(1..)?;
    let n = usize::from(*rest.first()?);
    let val = rest.get(1..=n)?.to_vec();
    let del = rest.get(1 + n..)?.to_vec();
    Some((val, del))
}

/// `GetUBDsKey`.
#[must_use]
pub fn ubds(del: &[u8]) -> Vec<u8> {
    cat(UNBONDING_DELEGATION, &[&lp(del)])
}

/// `GetUBDKey`.
#[must_use]
pub fn ubd(del: &[u8], val: &[u8]) -> Vec<u8> {
    cat(UNBONDING_DELEGATION, &[&lp(del), &lp(val)])
}

/// `GetUBDsByValIndexKey`.
#[must_use]
pub fn ubds_by_val(val: &[u8]) -> Vec<u8> {
    cat(UNBONDING_DELEGATION_BY_VAL_INDEX, &[&lp(val)])
}

/// `GetUBDByValIndexKey`.
#[must_use]
pub fn ubd_by_val(del: &[u8], val: &[u8]) -> Vec<u8> {
    cat(UNBONDING_DELEGATION_BY_VAL_INDEX, &[&lp(val), &lp(del)])
}

/// `GetUBDKeyFromValIndexKey`.
#[must_use]
pub fn ubd_from_val_index(key: &[u8]) -> Option<Vec<u8>> {
    let rest = key.get(1..)?;
    let n = usize::from(*rest.first()?);
    let val = rest.get(1..=n)?;
    let del = rest.get(n + 2..)?;
    Some(ubd(del, val))
}

/// `GetREDsKey`.
#[must_use]
pub fn reds(del: &[u8]) -> Vec<u8> {
    cat(REDELEGATION, &[&lp(del)])
}

/// `GetREDKey`.
#[must_use]
pub fn red(del: &[u8], src: &[u8], dst: &[u8]) -> Vec<u8> {
    cat(REDELEGATION, &[&lp(del), &lp(src), &lp(dst)])
}

/// `GetREDsFromValSrcIndexKey`.
#[must_use]
pub fn reds_from_val_src(src: &[u8]) -> Vec<u8> {
    cat(REDELEGATION_BY_VAL_SRC_INDEX, &[&lp(src)])
}

/// `GetREDByValSrcIndexKey`.
#[must_use]
pub fn red_by_val_src(del: &[u8], src: &[u8], dst: &[u8]) -> Vec<u8> {
    cat(
        REDELEGATION_BY_VAL_SRC_INDEX,
        &[&lp(src), &lp(del), &lp(dst)],
    )
}

/// `GetREDByValDstIndexKey`.
#[must_use]
pub fn red_by_val_dst(del: &[u8], src: &[u8], dst: &[u8]) -> Vec<u8> {
    cat(
        REDELEGATION_BY_VAL_DST_INDEX,
        &[&lp(dst), &lp(del), &lp(src)],
    )
}

/// `GetREDsByDelToValDstIndexKey`.
#[must_use]
pub fn reds_by_del_to_val_dst(del: &[u8], dst: &[u8]) -> Vec<u8> {
    cat(REDELEGATION_BY_VAL_DST_INDEX, &[&lp(dst), &lp(del)])
}

/// `GetREDKeyFromValSrcIndexKey`.
#[must_use]
pub fn red_from_val_src_index(key: &[u8]) -> Option<Vec<u8>> {
    let a = key.get(1..)?;
    let src_len = usize::from(*a.first()?);
    let src = a.get(1..=src_len)?;
    let del_len = usize::from(*a.get(src_len + 1)?);
    let del = a.get(src_len + 2..src_len + 2 + del_len)?;
    let dst = a.get(src_len + del_len + 3..)?;
    Some(red(del, src, dst))
}

/// `GetUnbondingDelegationTimeKey`.
#[must_use]
pub fn ubd_queue(t: &Timestamp) -> Vec<u8> {
    cat(UNBONDING_QUEUE, &[&sdk::time::format_time_bytes(t)])
}

/// `GetRedelegationTimeKey`.
#[must_use]
pub fn red_queue(t: &Timestamp) -> Vec<u8> {
    cat(REDELEGATION_QUEUE, &[&sdk::time::format_time_bytes(t)])
}

/// `GetValidatorQueueKey`.
#[must_use]
pub fn validator_queue(t: &Timestamp, height: i64) -> Vec<u8> {
    let tb = sdk::time::format_time_bytes(t);
    #[allow(clippy::cast_sign_loss)]
    let h = (height as u64).to_be_bytes();
    cat(
        VALIDATOR_QUEUE,
        &[&(tb.len() as u64).to_be_bytes(), &tb, &h],
    )
}

/// `ParseValidatorQueueKey`: `(time bytes, height)`.
#[must_use]
pub fn parse_validator_queue(key: &[u8]) -> Option<(Vec<u8>, i64)> {
    let len = usize::try_from(u64::from_be_bytes(key.get(1..9)?.try_into().ok()?)).ok()?;
    let tb = key.get(9..9 + len)?.to_vec();
    let h = u64::from_be_bytes(key.get(9 + len..)?.try_into().ok()?);
    #[allow(clippy::cast_possible_wrap)]
    Some((tb, h as i64))
}

/// `GetHistoricalInfoKey`.
#[must_use]
pub fn historical_info(height: i64) -> Vec<u8> {
    #[allow(clippy::cast_sign_loss)]
    let h = (height as u64).to_be_bytes();
    cat(HISTORICAL_INFO, &[&h])
}

/// `GetUnbondingIndexKey`.
#[must_use]
pub fn unbonding_index(id: u64) -> Vec<u8> {
    cat(UNBONDING_INDEX, &[&id.to_be_bytes()])
}

/// `GetUnbondingTypeKey`.
#[must_use]
pub fn unbonding_type(id: u64) -> Vec<u8> {
    cat(UNBONDING_TYPE, &[&id.to_be_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_index_matches_fixture() {
        let op = hex("84ebd1bdf6aae651dcea9ccc60487daae83e87cd");
        assert_eq!(
            power_index(5000, &op),
            hex("230000000000001388147b142e42095519ae231563339fb7825517c17832")
        );
        assert_eq!(parse_power_index(&power_index(5000, &op)), op);
    }

    #[test]
    fn index_rearrangement() {
        let (d, s, t) = (vec![1u8; 20], vec![2u8; 20], vec![3u8; 20]);
        assert_eq!(
            red_from_val_src_index(&red_by_val_src(&d, &s, &t)),
            Some(red(&d, &s, &t))
        );
        assert_eq!(ubd_from_val_index(&ubd_by_val(&d, &s)), Some(ubd(&d, &s)));
        assert_eq!(
            parse_delegation_by_val(&delegation_by_val(&s, &d)),
            Some((s.clone(), d.clone()))
        );
        let t0 = sdk::time::unix_epoch();
        let k = validator_queue(&t0, 7);
        assert_eq!(
            parse_validator_queue(&k),
            Some((sdk::time::format_time_bytes(&t0), 7))
        );
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
