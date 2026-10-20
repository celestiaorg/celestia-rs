//! Consensus-compatible port of Go's [`cosmossdk.io/math`] v1.5.3.
//!
//! The Cosmos SDK state machine stores amounts and rates as
//! [`Int`] (256-bit bounded signed integers), [`Uint`] (256-bit bounded
//! unsigned integers) and [`LegacyDec`] (18-decimal fixed point). Rounding,
//! range checks, string formatting and the gogoproto `customtype` binary
//! encoding all affect state and therefore the app hash, so this crate mirrors
//! the Go implementation operation-for-operation.
//!
//! Differences from Go are limited to error handling: every Go panic
//! (overflow, division by zero, out-of-range conversion) is surfaced as a
//! [`MathError`] instead. Go's "nil" values (an uninitialised `big.Int`) are
//! not modelled; operations that would leave a Go value nil (unmarshalling an
//! empty byte slice) produce zero, which marshals to the same bytes (`"0"`).
//!
//! Conformance: `tests/conformance.rs` replays the Go-generated vectors in
//! `conformance/fixtures/math/vectors.json` (generator:
//! `conformance/mathvectors`).
//!
//! [`cosmossdk.io/math`]: https://pkg.go.dev/cosmossdk.io/math@v1.5.3
#![warn(missing_docs)]

mod dec;
mod int;
mod parse;
mod uint;

pub use dec::{LEGACY_PRECISION, LegacyDec};
pub use int::{Int, MAX_BIT_LEN};
pub use uint::Uint;

/// Parses `text` like Go `(*big.Int).UnmarshalText` (`SetString(text, 0)`:
/// optional sign, `0x`/`0b`/`0o`/`0` prefixes, `_` separators), without a
/// range check; `None` where Go fails.
#[must_use]
pub fn parse_big_int_text(text: &[u8]) -> Option<num_bigint::BigInt> {
    parse::parse_go_big_int(text, parse::Base::Auto)
}

/// Errors returned by the math types. Each variant corresponds to an error or
/// panic in the Go implementation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MathError {
    /// Go `ErrIntOverflow` / `"Int overflow"` panics: a result exceeds the
    /// type's range.
    #[error("integer overflow")]
    IntOverflow,
    /// Go `ErrDivideByZero` / big.Int division-by-zero panic.
    #[error("divide by zero")]
    DivideByZero,
    /// The string is not a valid Go integer literal (`SetString(s, 0)` failed).
    #[error("cannot convert {0:?} to big.Int")]
    InvalidInt(String),
    /// The integer has more than [`MAX_BIT_LEN`] bits.
    #[error("integer out of range; max: {MAX_BIT_LEN} bits")]
    IntOutOfRange,
    /// A [`Uint`] was constructed from a negative value.
    #[error("non-positive integer")]
    NegativeUint,
    /// Go `ErrLegacyEmptyDecimalStr`.
    #[error("decimal string cannot be empty")]
    EmptyDecimalStr,
    /// Go `ErrLegacyInvalidDecimalLength`.
    #[error("invalid decimal length")]
    InvalidDecimalLength,
    /// Go `ErrLegacyInvalidDecimalStr` (including Go's "out of range" wrap).
    #[error("invalid decimal string")]
    InvalidDecimalStr,
    /// The decimal string has more than [`LEGACY_PRECISION`] fractional digits.
    #[error("value has {0} decimal places: max precision {LEGACY_PRECISION}")]
    ExcessPrecision(usize),
    /// The digits of a decimal string could not be parsed in base 10.
    #[error("failed to set decimal string with base 10: {0}")]
    InvalidDecimalDigits(String),
    /// A decimal value is outside `±(2^256 * 10^18 - 1)` (scaled).
    #[error("decimal out of range")]
    DecOutOfRange,
    /// A precision argument outside `0..=18`.
    #[error("invalid precision {0}")]
    InvalidPrecision(i64),
    /// A conversion to `i64` / `u64` is out of bounds.
    #[error("value out of bound")]
    ConversionOutOfRange,
    /// A JSON value was not a string.
    #[error("invalid JSON: {0}")]
    Json(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, MathError>;

/// Converts a proto3-JSON / amino-JSON `LegacyDec` string (the contents of the
/// JSON string, without quotes, e.g. `"0.001"`) into the gogoproto
/// `customtype` binary encoding (`LegacyDec.Marshal()`, e.g.
/// `b"1000000000000000"`).
///
/// Mirrors Go's `LegacyDec.UnmarshalJSON` followed by `LegacyDec.Marshal`.
pub fn legacy_dec_json_to_proto_bytes(s: &str) -> Result<Vec<u8>> {
    Ok(LegacyDec::from_str_go(s)?.marshal())
}

/// Converts a JSON `Int` string (contents of the JSON string, without quotes)
/// into the gogoproto `customtype` binary encoding (`Int.Marshal()`).
///
/// Mirrors Go's `Int.UnmarshalJSON` followed by `Int.Marshal`; like Go it
/// accepts integer literal syntax such as `0x10`.
pub fn int_json_to_proto_bytes(s: &str) -> Result<Vec<u8>> {
    Ok(Int::parse(s)?.marshal())
}

/// Decodes a JSON document that must be a string (or `null`, which Go decodes
/// to the empty string).
fn json_string(bz: &[u8]) -> Result<String> {
    let v: Option<String> =
        serde_json::from_slice(bz).map_err(|e| MathError::Json(e.to_string()))?;
    Ok(v.unwrap_or_default())
}

/// Encodes `s` as a JSON string.
fn to_json_string(s: &str) -> String {
    // Digits, '-' and '.' never need escaping.
    format!("\"{s}\"")
}
