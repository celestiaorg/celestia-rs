//! Registered errors (`types/errors.go`): codespace `qgb`.

use sdk::SdkError;

/// Error codespace (`types.ModuleName`).
pub const CODESPACE: &str = "qgb";

/// `errors.Wrap(err, msg)` of a registered error: `msg: desc`.
fn wrapped(code: u32, desc: &str, msg: &str) -> SdkError {
    let log = if msg.is_empty() {
        desc.to_owned()
    } else {
        format!("{msg}: {desc}")
    };
    SdkError::new(CODESPACE, code, log)
}

/// `ErrDuplicate` wrapped with `msg`.
#[must_use]
pub fn duplicate(msg: &str) -> SdkError {
    wrapped(2, "duplicate", msg)
}

/// `ErrEmpty` wrapped with `msg`.
#[must_use]
pub fn empty(msg: &str) -> SdkError {
    wrapped(6, "empty", msg)
}

/// `ErrNoValidators`.
#[must_use]
pub fn no_validators() -> SdkError {
    wrapped(12, "no bonded validators in active set", "")
}

/// `ErrNilAttestation` wrapped with `msg`.
#[must_use]
pub fn nil_attestation(msg: &str) -> SdkError {
    wrapped(22, "nil attestation", msg)
}

/// `ErrUnmarshalllAttestation`.
#[must_use]
pub fn unmarshal_attestation() -> SdkError {
    wrapped(26, "couldn't unmarshall attestation from store", "")
}

/// `ErrLatestAttestationNonceStillNotInitialized`.
#[must_use]
pub fn latest_nonce_not_initialized() -> SdkError {
    wrapped(
        31,
        "the latest attestation nonce has still not been defined in store",
        "",
    )
}

/// `ErrEarliestAvailableNonceStillNotInitialized`.
#[must_use]
pub fn earliest_nonce_not_initialized() -> SdkError {
    wrapped(
        33,
        "the earliest available nonce after pruning has still not been defined in store",
        "",
    )
}

/// `ErrAttestationNotFound` wrapped with `msg`.
#[must_use]
pub fn attestation_not_found(msg: &str) -> SdkError {
    wrapped(18, "attestation not found", msg)
}

/// `ErrEVMAddressNotHex`.
#[must_use]
pub fn evm_address_not_hex() -> SdkError {
    wrapped(
        36,
        "the provided evm address is not a valid hex address",
        "",
    )
}

/// `ErrEVMAddressAlreadyExists` wrapped with `msg`.
#[must_use]
pub fn evm_address_already_exists(msg: &str) -> SdkError {
    wrapped(37, "the provided evm address already exists", msg)
}
