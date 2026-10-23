//! Registered errors of `hyperlane`, `ism` and `post_dispatch`, plus the
//! ABCI mapping of Go's unregistered errors.

use sdk::SdkError;

/// A plain Go error (`fmt.Errorf`, `errors.New`): `ABCIInfo` reports it as
/// codespace `undefined`, code 1.
pub fn internal(log: impl Into<String>) -> SdkError {
    SdkError::new("undefined", 1, log)
}

/// A recovered Go panic inside a message handler (`sdkerrors.ErrPanic`).
pub fn panic_err(log: impl Into<String>) -> SdkError {
    SdkError::new("undefined", 111_222, log)
}

fn reg(codespace: &'static str, code: u32, desc: &str, log: &str) -> SdkError {
    let full = if log.is_empty() {
        desc.to_string()
    } else {
        format!("{log}: {desc}")
    };
    SdkError::new(codespace, code, full)
}

macro_rules! registered {
    ($codespace:literal, $($name:ident = $code:literal, $desc:literal;)*) => {
        $(
            #[doc = $desc]
            pub fn $name(log: impl AsRef<str>) -> SdkError {
                reg($codespace, $code, $desc, log.as_ref())
            }
        )*
    };
}

/// `x/core/types/errors.go` (codespace `hyperlane`).
pub mod core {
    use super::{SdkError, reg};
    registered! { "hyperlane",
        no_receiver_ism = 1, "no receiver ISM";
        required_hook_not_set = 2, "required hook not set";
        default_hook_not_set = 3, "default hook not set";
        ism_routing_cycle = 4, "ism routing cycle";
    }
}

/// `x/core/01_interchain_security/types/errors.go` (codespace `ism`).
pub mod ism {
    use super::{SdkError, reg};
    registered! { "ism",
        unexpected_error = 1, "unexpected error";
        invalid_multisig_configuration = 2, "invalid multisig configuration";
        invalid_announce = 3, "invalid announce";
        mailbox_does_not_exist = 4, "mailbox does not exist";
        invalid_signature = 5, "invalid signature";
        invalid_ism_type = 6, "invalid ism type";
        unknown_ism_id = 7, "unknown ism id";
        no_route_found = 8, "no route found";
        unauthorized = 9, "unauthorized";
        invalid_owner = 10, "invalid owner";
        duplicated_domains = 11, "route for domain already exists";
    }
}

/// `x/core/02_post_dispatch/types/errors.go` (codespace `post_dispatch`).
pub mod post_dispatch {
    use super::{SdkError, reg};
    registered! { "post_dispatch",
        mailbox_does_not_exist = 1, "mailbox does not exist";
        sender_is_not_designated_mailbox = 2, "sender is not designated mailbox";
        hook_does_not_exist_or_is_not_registered = 3, "hook does not exist or isn't registered";
        unauthorized = 4, "unauthorized";
        invalid_owner = 5, "invalid owner";
    }
}

/// True when `e` is the registered error `(codespace, code)`
/// (`errors.IsOf`).
#[must_use]
pub fn is_of(e: &SdkError, codespace: &str, code: u32) -> bool {
    e.codespace == codespace && e.code == code
}
