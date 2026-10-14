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

use std::fmt;

/// Errors returned by the math types. Each variant corresponds to an error or
/// panic in the Go implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MathError {
    /// Go `ErrIntOverflow` / `"Int overflow"` panics: a result exceeds the
    /// type's range.
    IntOverflow,
    /// Go `ErrDivideByZero` / big.Int division-by-zero panic.
    DivideByZero,
    /// The string is not a valid Go integer literal (`SetString(s, 0)` failed).
    InvalidInt(String),
    /// The integer has more than [`MAX_BIT_LEN`] bits.
    IntOutOfRange,
    /// A [`Uint`] was constructed from a negative value.
    NegativeUint,
    /// Go `ErrLegacyEmptyDecimalStr`.
    EmptyDecimalStr,
    /// Go `ErrLegacyInvalidDecimalLength`.
    InvalidDecimalLength,
    /// Go `ErrLegacyInvalidDecimalStr` (including Go's "out of range" wrap).
    InvalidDecimalStr,
    /// The decimal string has more than [`LEGACY_PRECISION`] fractional digits.
    ExcessPrecision(usize),
    /// The digits of a decimal string could not be parsed in base 10.
    InvalidDecimalDigits(String),
    /// A decimal value is outside `±(2^256 * 10^18 - 1)` (scaled).
    DecOutOfRange,
    /// A precision argument outside `0..=18`.
    InvalidPrecision(i64),
    /// A conversion to `i64` / `u64` is out of bounds.
    ConversionOutOfRange,
    /// A JSON value was not a string.
    Json(String),
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntOverflow => f.write_str("integer overflow"),
            Self::DivideByZero => f.write_str("divide by zero"),
            Self::InvalidInt(s) => write!(f, "cannot convert {s:?} to big.Int"),
            Self::IntOutOfRange => write!(f, "integer out of range; max: {MAX_BIT_LEN} bits"),
            Self::NegativeUint => f.write_str("non-positive integer"),
            Self::EmptyDecimalStr => f.write_str("decimal string cannot be empty"),
            Self::InvalidDecimalLength => f.write_str("invalid decimal length"),
            Self::InvalidDecimalStr => f.write_str("invalid decimal string"),
            Self::ExcessPrecision(n) => write!(
                f,
                "value has {n} decimal places: max precision {LEGACY_PRECISION}"
            ),
            Self::InvalidDecimalDigits(s) => {
                write!(f, "failed to set decimal string with base 10: {s}")
            }
            Self::DecOutOfRange => f.write_str("decimal out of range"),
            Self::InvalidPrecision(p) => write!(f, "invalid precision {p}"),
            Self::ConversionOutOfRange => f.write_str("value out of bound"),
            Self::Json(s) => write!(f, "invalid JSON: {s}"),
        }
    }
}

impl std::error::Error for MathError {}

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
