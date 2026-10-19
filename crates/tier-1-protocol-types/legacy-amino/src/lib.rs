//! go-amino v0.16 codec for the legacy `StdTx`, as celestia-appd's
//! `cosmos.tx.v1beta1.Service/TxDecodeAmino` and `TxEncodeAmino` run it
//! (`codec.LegacyAmino` over the app's amino registrations).
//!
//! go-amino encodes Go values by reflection, so the Go type graph reachable
//! from `legacytx.StdTx` (struct fields, json/amino tags, registered concrete
//! names and prefixes, interface implementers, custom marshalers, and which
//! `Any` fields `UnpackInterfaces` visits with which interface) is dumped from
//! celestia-appd v10.4.0 into `schema.json` (and from v9.0.8, which runs
//! app version 9, into `schema_v9.json`) by
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

/// Bytes as Go's `encoding/json` writes a string of them (each invalid
/// byte U+FFFD): what a Go caller of these functions sees in JSON.
pub(crate) fn go_lossy(b: &[u8]) -> String {
    gostr::lossy(&gostr::from_go_bytes(b)).into_owned()
}

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

/// `TxDecodeAmino`: amino binary `StdTx` to amino JSON (v10.4.0's codec).
pub fn tx_decode_amino(bz: &[u8]) -> Result<String, Error> {
    tx_decode_amino_in(Schema::get(), bz)
}

/// `TxDecodeAmino` over the codec `s` ([`Schema::for_app_version`]).
pub fn tx_decode_amino_in(s: &Schema, bz: &[u8]) -> Result<String, Error> {
    with_stack(bz.len(), || {
        let root = s.root;
        let mut v = s.unmarshal_binary_bare(bz, root)?;
        s.unpack(root, &mut v, unpack::Mode::Binary)?;
        let out = s.marshal_json(root, &v)?;
        Ok(go_lossy(&out))
    })
}

/// `TxEncodeAmino`: amino JSON `StdTx` to amino binary (v10.4.0's codec).
pub fn tx_encode_amino(json: &[u8]) -> Result<Vec<u8>, Error> {
    tx_encode_amino_in(Schema::get(), json)
}

/// `TxEncodeAmino` over the codec `s` ([`Schema::for_app_version`]).
pub fn tx_encode_amino_in(s: &Schema, json: &[u8]) -> Result<Vec<u8>, Error> {
    with_stack(json.len(), || {
        let root = s.root;
        let mut v = s.unmarshal_json(json, root)?;
        s.unpack(root, &mut v, unpack::Mode::Json)?;
        s.marshal_binary_bare(root, &v)
    })
}

/// Inputs longer than this are processed on a thread with a large stack:
/// the codec recurses once per nested value (e.g. `MsgExec` in `MsgExec`),
/// Go's goroutine stacks grow to 1 GB and decode thousands of levels, and
/// a query handler's 2 MiB stack overflowed (aborting the node) on a few
/// hundred KB of nesting.
const LARGE_INPUT: usize = 4 * 1024;
const LARGE_STACK: usize = 256 * 1024 * 1024;

fn with_stack<T: Send>(
    len: usize,
    f: impl FnOnce() -> Result<T, Error> + Send,
) -> Result<T, Error> {
    if len <= LARGE_INPUT {
        return f();
    }
    std::thread::scope(|s| {
        match std::thread::Builder::new()
            .stack_size(LARGE_STACK)
            .spawn_scoped(s, f)
        {
            Ok(h) => match h.join() {
                Ok(v) => v,
                Err(e) => std::panic::resume_unwind(e),
            },
            Err(e) => Err(Error::Go(format!("spawning codec thread: {e}"))),
        }
    })
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
