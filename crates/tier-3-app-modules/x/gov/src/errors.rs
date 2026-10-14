//! `x/gov` sentinel errors (codespace `gov`).

use sdk::SdkError;

macro_rules! gov_errors {
    ($($name:ident = $code:expr, $desc:expr;)*) => {
        $(
            #[doc = $desc]
            #[must_use]
            pub fn $name() -> SdkError {
                SdkError::new("gov", $code, $desc)
            }
        )*
    };
}

gov_errors! {
    inactive_proposal = 3, "inactive proposal";
    already_active_proposal = 4, "proposal already active";
    invalid_proposal_content = 5, "invalid proposal content";
    invalid_proposal_type = 6, "invalid proposal type";
    invalid_vote = 7, "invalid vote option";
    invalid_genesis = 8, "invalid genesis state";
    no_proposal_handler_exists = 9, "no handler exists for proposal type";
    unroutable_proposal_msg = 10, "proposal message not recognized by router";
    no_proposal_msgs = 11, "no messages proposed";
    invalid_proposal_msg = 12, "invalid proposal message";
    invalid_signer = 13, "expected gov account as only signer for proposal message";
    metadata_too_long = 15, "metadata too long";
    min_deposit_too_small = 16, "minimum deposit is too small";
    invalid_proposer = 18, "invalid proposer";
    voting_period_ended = 20, "voting period already ended";
    invalid_proposal = 21, "invalid proposal";
    summary_too_long = 22, "summary too long";
    invalid_deposit_denom = 23, "invalid deposit denom";
}

/// `errorsmod.Wrap(err, msg)` for a sentinel: `msg: desc`.
#[must_use]
pub fn wrap(e: fn() -> SdkError, msg: impl AsRef<str>) -> SdkError {
    e().wrap(msg)
}
