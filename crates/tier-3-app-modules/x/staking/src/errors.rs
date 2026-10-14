//! `x/staking/types/errors.go` (codespace `staking`).

use sdk::SdkError;

macro_rules! staking_errors {
    ($($name:ident = $code:expr, $desc:expr;)*) => {
        $(
            #[doc = $desc]
            #[must_use]
            pub fn $name() -> SdkError {
                SdkError::new(crate::MODULE_NAME, $code, $desc)
            }
        )*
    };
}

staking_errors! {
    empty_validator_addr = 2, "empty validator address";
    no_validator_found = 3, "validator does not exist";
    validator_owner_exists = 4, "validator already exist for this operator address; must use new validator operator address";
    validator_pub_key_exists = 5, "validator already exist for this pubkey; must use new validator pubkey";
    validator_pub_key_type_not_supported = 6, "validator pubkey type is not supported";
    validator_jailed = 7, "validator for this address is currently jailed";
    bad_remove_validator = 8, "failed to remove validator";
    commission_negative = 9, "commission must be positive";
    commission_huge = 10, "commission cannot be more than 100%";
    commission_gt_max_rate = 11, "commission cannot be more than the max rate";
    commission_update_time = 12, "commission cannot be changed more than once in 24h";
    commission_change_rate_negative = 13, "commission change rate must be positive";
    commission_change_rate_gt_max_rate = 14, "commission change rate cannot be more than the max rate";
    commission_gt_max_change_rate = 15, "commission cannot be changed more than max change rate";
    self_delegation_below_minimum = 16, "validator's self delegation must be greater than their minimum self delegation";
    min_self_delegation_decreased = 17, "minimum self delegation cannot be decrease";
    empty_delegator_addr = 18, "empty delegator address";
    no_delegation = 19, "no delegation for (address, validator) tuple";
    bad_delegator_addr = 20, "delegator does not exist with address";
    no_delegator_for_address = 21, "delegator does not contain delegation";
    insufficient_shares = 22, "insufficient delegation shares";
    delegation_validator_empty = 23, "cannot delegate to an empty validator";
    not_enough_delegation_shares = 24, "not enough delegation shares";
    not_mature = 25, "entry not mature";
    no_unbonding_delegation = 26, "no unbonding delegation found";
    max_unbonding_delegation_entries = 27, "too many unbonding delegation entries for (delegator, validator) tuple";
    no_redelegation = 28, "no redelegation found";
    self_redelegation = 29, "cannot redelegate to the same validator";
    tiny_redelegation_amount = 30, "too few tokens to redelegate (truncates to zero tokens)";
    bad_redelegation_dst = 31, "redelegation destination validator not found";
    transitive_redelegation = 32, "redelegation to this validator already in progress; first redelegation to this validator must complete before next redelegation";
    max_redelegation_entries = 33, "too many redelegation entries for (delegator, src-validator, dst-validator) tuple";
    delegator_share_ex_rate_invalid = 34, "cannot delegate to validators with invalid (zero) ex-rate";
    invalid_historical_info = 37, "invalid historical info";
    no_historical_info = 38, "no historical info found";
    empty_validator_pub_key = 39, "empty validator public key";
    commission_lt_min_rate = 40, "commission cannot be less than min rate";
    unbonding_not_found = 41, "unbonding operation not found";
    unbonding_on_hold_ref_count_negative = 42, "cannot un-hold unbonding operation that is not on hold";
    bad_redelegation_src = 44, "redelegation source validator not found";
    no_unbonding_type = 45, "unbonding type not found";
}

/// `govtypes.ErrInvalidSigner` (codespace `gov`, code 13).
#[must_use]
pub fn gov_invalid_signer() -> SdkError {
    SdkError::new(
        "gov",
        13,
        "expected gov account as only signer for proposal message",
    )
}

/// True when `e` is the given staking error.
#[must_use]
pub fn is(e: &SdkError, f: fn() -> SdkError) -> bool {
    let t = f();
    e.codespace == t.codespace && e.code == t.code
}
