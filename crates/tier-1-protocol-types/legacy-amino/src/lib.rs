//! go-amino v0.16 codec for the legacy `StdTx`, as celestia-appd's
//! `cosmos.tx.v1beta1.Service/TxDecodeAmino` and `TxEncodeAmino` run it
//! (`codec.LegacyAmino` over the app's amino registrations).
//!
//! go-amino encodes Go values by reflection, so the Go type graph reachable
//! from `legacytx.StdTx` (struct fields, json/amino tags, registered concrete
//! names and prefixes, interface implementers, custom marshalers, and which
//! `Any` fields `UnpackInterfaces` visits with which interface) is dumped from
//! celestia-appd v10.4.0 into `schema.json` by
//! `conformance/aminoschema/schema_test.go`. The codec here is a port of
//! go-amino's reflection encoder/decoder over that schema, with the Go types
//! that override it (`time.Time`, `math.Int`, `math.LegacyDec`, `sdk.Coins`,
//! `codectypes.Any`, the amino key types, `LegacyAminoPubKey`,
//! hyperlane's `HexAddress`) ported by hand.

// go-amino's integer handling is a sequence of deliberate Go conversions
// (`int64(u64)`, `uint64(int64)`, byte truncation), ported as casts; the
// codec functions mirror Go's long functions one to one.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]

mod binary;
mod gojson;
mod json;
mod schema;
mod time;
mod unpack;

use num_bigint::BigInt;

pub use schema::Schema;

/// A codec failure, rendered like the Go error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A returned Go error (`err.Error()`).
    Go(String),
    /// A Go runtime panic (the value Go would recover).
    Panic(String),
    /// A (wrapped) registered `sdk` codespace error: its code and full
    /// message.
    Sdk(u32, String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Go(s) | Self::Panic(s) | Self::Sdk(_, s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) type Res<T> = Result<T, Error>;

pub(crate) fn err<T>(s: impl Into<String>) -> Res<T> {
    Err(Error::Go(s.into()))
}

/// A Go value of a schema type.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum V {
    /// A nil pointer, interface or slice.
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    /// A Go string (any bytes).
    Str(Vec<u8>),
    /// A non-nil `[]byte` or a `[N]byte`.
    Bytes(Vec<u8>),
    /// A non-nil slice or an array.
    List(Vec<V>),
    Struct(Vec<V>),
    /// A non-nil pointer.
    Ptr(Box<V>),
    /// A non-nil interface: concrete type and its (non-pointer) value.
    Iface(usize, Box<V>),
    /// `time.Time` as Unix seconds and nanoseconds (UTC).
    Time(i64, u32),
    /// `math.Int` (`None`: nil `*big.Int`).
    BigInt(Option<BigInt>),
    /// `math.LegacyDec`, `10^18`-scaled (`None`: nil `*big.Int`).
    Dec(Option<BigInt>),
    /// `codectypes.Any`.
    Any(AnyV),
}

/// The amino state of a `codectypes.Any` (its `compat` / `cachedValue`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AnyV {
    /// No compat, no cached value.
    Zero,
    /// `compat.aminoBz` (binary decoded, not unpacked).
    Amino(Vec<u8>),
    /// `compat.jsonBz` (JSON decoded, not unpacked).
    Json(Vec<u8>),
    /// Unpacked: the cached concrete value.
    Value(usize, Box<V>),
}

/// `TxDecodeAmino`: amino binary `StdTx` to amino JSON.
pub fn tx_decode_amino(bz: &[u8]) -> Result<String, Error> {
    let s = Schema::get();
    let root = s.root;
    let mut v = s.unmarshal_binary_bare(bz, root)?;
    s.unpack(root, &mut v, unpack::Mode::Binary)?;
    let out = s.marshal_json(root, &v)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// `TxEncodeAmino`: amino JSON `StdTx` to amino binary.
pub fn tx_encode_amino(json: &[u8]) -> Result<Vec<u8>, Error> {
    let s = Schema::get();
    let root = s.root;
    let mut v = s.unmarshal_json(json, root)?;
    s.unpack(root, &mut v, unpack::Mode::Json)?;
    s.marshal_binary_bare(root, &v)
}

/// Go `fmt` `%X` of a byte slice.
pub(crate) fn upper_hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02X}");
    }
    s
}

#[cfg(test)]
mod tests;
