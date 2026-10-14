//! `x/distribution/types/errors.go` (codespace `distribution`).

use sdk::SdkError;

macro_rules! distr_errors {
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

distr_errors! {
    empty_delegator_addr = 2, "delegator address is empty";
    empty_withdraw_addr = 3, "withdraw address is empty";
    empty_validator_addr = 4, "validator address is empty";
    empty_delegation_dist_info = 5, "no delegation distribution info";
    no_validator_dist_info = 6, "no validator distribution info";
    no_validator_commission = 7, "no validator commission to withdraw";
    set_withdraw_addr_disabled = 8, "set withdraw address disabled";
    bad_distribution = 9, "community pool does not have sufficient coins to distribute";
    invalid_proposal_amount = 10, "invalid community pool spend proposal amount";
    empty_proposal_recipient = 11, "invalid community pool spend proposal recipient";
    no_validator_exists = 12, "validator does not exist";
    no_delegation_exists = 13, "delegation does not exist";
}
