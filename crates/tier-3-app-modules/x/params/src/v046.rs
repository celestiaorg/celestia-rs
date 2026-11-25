//! cosmos-sdk v0.46 parameter subspaces (celestia-app v1 to v3): every
//! module keeps its parameters here, one amino-JSON value per key.
//!
//! A subspace has a key table: the registered keys, each with its value
//! type and validator (`KeyTable`, `ParamSetPair`). `SetParamSet`
//! validates every pair and then `Set`s it; `Set` checks only that the key
//! is registered with that type. Both panic in Go on failure.
//!
//! Values are written with `LegacyAmino.MarshalJSON`: 64-bit integers
//! (and `time.Duration`, in nanoseconds) as quoted decimals, 32-bit
//! integers and booleans bare, `sdk.Dec` and `sdk.Int` through their
//! `MarshalJSON` (quoted strings), strings with Go's HTML-safe escaping,
//! a nil slice as `null`, and struct fields in declaration order with
//! `omitempty` honored.
//!
//! References: cosmos-sdk `v1.31.2-sdk-v0.46.16` `x/params/types/subspace.go`
//! and `table.go`, go-amino `v0.16.0` `json-encode.go`.

use math::{Int, LegacyDec};
use sdk::{Context, Result};

/// Go `encoding/json` string encoding (HTML-safe escaping, `\b` and `\f`
/// short escapes), as go-amino writes strings; each invalid UTF-8 byte of
/// the Go string `s` is written as U+FFFD.
#[must_use]
pub fn json_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut w = String::with_capacity(s.len().saturating_add(2));
    w.push('"');
    for r in gostr::runes(s) {
        let Some(c) = r else {
            w.push_str("\\ufffd");
            continue;
        };
        match c {
            '"' => w.push_str("\\\""),
            '\\' => w.push_str("\\\\"),
            '\n' => w.push_str("\\n"),
            '\r' => w.push_str("\\r"),
            '\t' => w.push_str("\\t"),
            '\u{8}' => w.push_str("\\b"),
            '\u{c}' => w.push_str("\\f"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(w, "\\u{:04x}", u32::from(c));
            }
            c if u32::from(c) < 0x20 => {
                let _ = write!(w, "\\u{:04x}", u32::from(c));
            }
            c => gostr::push_char(&mut w, c),
        }
    }
    w.push('"');
    w
}

/// An amino-JSON object built field by field in Go declaration order.
#[derive(Debug, Default)]
pub struct Object(Vec<(&'static str, String)>);

impl Object {
    /// An empty object.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A field written whatever its value.
    pub fn field(&mut self, name: &'static str, json: String) -> &mut Self {
        self.0.push((name, json));
        self
    }

    /// An `omitempty` field: written only when `json` is `Some`.
    pub fn omitempty(&mut self, name: &'static str, json: Option<String>) -> &mut Self {
        if let Some(json) = json {
            self.0.push((name, json));
        }
        self
    }

    /// The encoded object.
    #[must_use]
    pub fn finish(&self) -> String {
        let body: Vec<String> = self
            .0
            .iter()
            .map(|(k, v)| format!("{}:{v}", json_string(k)))
            .collect();
        format!("{{{}}}", body.join(","))
    }
}

/// A quoted 64-bit integer (amino writes `int64`/`uint64` as strings).
#[must_use]
pub fn int64(v: i64) -> String {
    format!("\"{v}\"")
}

/// A quoted unsigned 64-bit integer.
#[must_use]
pub fn uint64(v: u64) -> String {
    format!("\"{v}\"")
}

/// A non-empty string for an `omitempty` field.
#[must_use]
pub fn nonempty_string(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| json_string(s))
}

/// `sdk.Coin` (`denom` is `omitempty`; `amount` is `sdk.Int`).
#[must_use]
pub fn coin(denom: &str, amount: &Int) -> String {
    Object::new()
        .omitempty("denom", nonempty_string(denom))
        .field("amount", json_string(&amount.to_string()))
        .finish()
}

/// One entry of the bank `SendEnabled` list (`SendEnabled` proto struct:
/// both fields `omitempty`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendEnabled {
    /// Denomination (empty for the global default).
    pub denom: String,
    /// Whether sends of `denom` are enabled.
    pub enabled: bool,
}

/// A parameter value with the Go type a key table registers for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `uint64`.
    Uint64(u64),
    /// `uint32`.
    Uint32(u32),
    /// `int64`.
    Int64(i64),
    /// `time.Duration` (nanoseconds).
    Duration(i64),
    /// `bool`.
    Bool(bool),
    /// `string`.
    Str(String),
    /// `sdk.Dec`.
    Dec(LegacyDec),
    /// `sdk.Coin`.
    Coin {
        /// Denomination.
        denom: String,
        /// Amount.
        amount: Int,
    },
    /// `[]string`; `None` is a nil slice.
    Strings(Option<Vec<String>>),
    /// bank `[]*SendEnabled`; `None` is a nil slice.
    SendEnabled(Option<Vec<SendEnabled>>),
    /// A struct whose module encodes it (e.g. gov's `DepositParams`).
    Struct(String),
}

/// The Go type of a [`Value`] (`KeyTable` attribute type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `uint64`.
    Uint64,
    /// `uint32`.
    Uint32,
    /// `int64`.
    Int64,
    /// `time.Duration`.
    Duration,
    /// `bool`.
    Bool,
    /// `string`.
    Str,
    /// `sdk.Dec`.
    Dec,
    /// `sdk.Coin`.
    Coin,
    /// `[]string`.
    Strings,
    /// bank `[]*SendEnabled`.
    SendEnabled,
    /// A module-encoded struct; `&'static str` names its Go type.
    Struct(&'static str),
}

fn list<T>(items: Option<&[T]>, f: impl Fn(&T) -> String) -> String {
    match items {
        None => "null".to_owned(),
        Some(items) => {
            let v: Vec<String> = items.iter().map(f).collect();
            format!("[{}]", v.join(","))
        }
    }
}

impl Value {
    /// The value's type.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Uint64(_) => Kind::Uint64,
            Self::Uint32(_) => Kind::Uint32,
            Self::Int64(_) => Kind::Int64,
            Self::Duration(_) => Kind::Duration,
            Self::Bool(_) => Kind::Bool,
            Self::Str(_) => Kind::Str,
            Self::Dec(_) => Kind::Dec,
            Self::Coin { .. } => Kind::Coin,
            Self::Strings(_) => Kind::Strings,
            Self::SendEnabled(_) => Kind::SendEnabled,
            Self::Struct(_) => Kind::Struct(""),
        }
    }

    /// `LegacyAmino.MarshalJSON` of the value.
    #[must_use]
    pub fn amino_json(&self) -> String {
        match self {
            Self::Uint64(v) => uint64(*v),
            Self::Int64(v) | Self::Duration(v) => int64(*v),
            Self::Uint32(v) => v.to_string(),
            Self::Bool(v) => v.to_string(),
            Self::Str(s) => json_string(s),
            Self::Dec(d) => json_string(&d.to_string()),
            Self::Coin { denom, amount } => coin(denom, amount),
            Self::Strings(v) => list(v.as_deref(), |s| json_string(s)),
            Self::SendEnabled(v) => list(v.as_deref(), |e| {
                Object::new()
                    .omitempty("denom", nonempty_string(&e.denom))
                    .omitempty("enabled", e.enabled.then(|| "true".to_owned()))
                    .finish()
            }),
            Self::Struct(json) => json.clone(),
        }
    }
}

/// A parameter validator (`ParamSetPair.ValidatorFn`): Go's error text on
/// an invalid value.
pub type Validator = fn(&Value) -> std::result::Result<(), String>;

/// One registered key (`attribute` of a `KeyTable`).
#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// Store key within the subspace.
    pub key: &'static str,
    /// Registered type.
    pub kind: Kind,
    /// Validator.
    pub validate: Validator,
}

/// A subspace with its key table.
#[derive(Debug, Clone, Copy)]
pub struct Subspace {
    /// Subspace name (the store prefix without `/`).
    pub name: &'static str,
    /// Registered keys.
    pub table: &'static [Param],
}

fn kinds_match(registered: Kind, value: Kind) -> bool {
    match (registered, value) {
        (Kind::Struct(_), Kind::Struct(_)) => true,
        (a, b) => a == b,
    }
}

impl Subspace {
    fn param(&self, key: &str) -> Result<&Param> {
        self.table
            .iter()
            .find(|p| p.key == key)
            .ok_or_else(|| sdk::error::go_panic(format!("parameter {key} not registered")))
    }

    /// `Subspace.Set`: type check against the key table, then write the
    /// amino JSON (and mark the key in the transient store).
    pub fn set(&self, ctx: &mut Context<'_>, key: &str, value: &Value) -> Result<()> {
        let p = self.param(key)?;
        if !kinds_match(p.kind, value.kind()) {
            return Err(sdk::error::go_panic("type mismatch with registered table"));
        }
        crate::set_raw(
            ctx,
            self.name,
            key.as_bytes(),
            value.amino_json().into_bytes(),
        )
    }

    /// `Subspace.SetParamSet`: for each pair in order, validate, then
    /// [`Subspace::set`]. A pair that fails stops the set (Go panics), with
    /// the earlier pairs written.
    pub fn set_param_set(&self, ctx: &mut Context<'_>, pairs: &[(&str, Value)]) -> Result<()> {
        for (key, value) in pairs {
            let p = self.param(key)?;
            (p.validate)(value).map_err(|e| {
                sdk::error::go_panic(format!("value from ParamSetPair is invalid: {e}"))
            })?;
            self.set(ctx, key, value)?;
        }
        Ok(())
    }
}

/// `Subspace.Get`'s value: the stored amino JSON of `key`, parsed. Go
/// panics when the key is absent or does not decode.
fn get_json(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<serde_json::Value> {
    let raw = crate::get_raw(ctx, space, key.as_bytes())?.unwrap_or_default();
    serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace {space}: {key}: {e}")))
}

fn bad(space: &str, key: &str) -> sdk::SdkError {
    sdk::error::go_panic(format!("subspace {space}: {key}: unexpected amino JSON"))
}

/// A quoted 64-bit integer parameter (`int64`, `uint64`, `time.Duration`).
fn get_quoted<T: std::str::FromStr>(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<T> {
    match get_json(ctx, space, key)? {
        serde_json::Value::String(s) => s.parse().map_err(|_| bad(space, key)),
        _ => Err(bad(space, key)),
    }
}

/// A `uint64` parameter.
pub fn get_u64(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<u64> {
    get_quoted(ctx, space, key)
}

/// An `int64` parameter (also `time.Duration`, in nanoseconds).
pub fn get_i64(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<i64> {
    get_quoted(ctx, space, key)
}

/// A `uint32` parameter (a bare number).
pub fn get_u32(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<u32> {
    get_json(ctx, space, key)?
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| bad(space, key))
}

/// A `bool` parameter.
pub fn get_bool(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<bool> {
    get_json(ctx, space, key)?
        .as_bool()
        .ok_or_else(|| bad(space, key))
}

/// A `string` parameter.
pub fn get_string(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<String> {
    match get_json(ctx, space, key)? {
        serde_json::Value::String(s) => Ok(s),
        _ => Err(bad(space, key)),
    }
}

/// An `sdk.Dec` parameter.
pub fn get_dec(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<LegacyDec> {
    let s = get_string(ctx, space, key)?;
    LegacyDec::from_str_go(&s).map_err(|_| bad(space, key))
}

/// The bank `SendEnabled` list.
pub fn get_send_enabled(ctx: &mut Context<'_>, space: &str, key: &str) -> Result<Vec<SendEnabled>> {
    let v = get_json(ctx, space, key)?;
    let items = match &v {
        serde_json::Value::Null => return Ok(Vec::new()),
        serde_json::Value::Array(a) => a,
        _ => return Err(bad(space, key)),
    };
    items
        .iter()
        .map(|e| {
            let denom = match &e["denom"] {
                serde_json::Value::Null => String::new(),
                serde_json::Value::String(s) => s.clone(),
                _ => return Err(bad(space, key)),
            };
            let enabled = match &e["enabled"] {
                serde_json::Value::Null => false,
                serde_json::Value::Bool(b) => *b,
                _ => return Err(bad(space, key)),
            };
            Ok(SendEnabled { denom, enabled })
        })
        .collect()
}

/// The key-table entry of a registered parameter.
#[must_use]
pub const fn param(key: &'static str, kind: Kind, validate: Validator) -> Param {
    Param {
        key,
        kind,
        validate,
    }
}

/// Validators of the v0.46 modules (`x/*/types/params.go`), keyed by the
/// Go function they port. Each also rejects a value of another type
/// (`invalid parameter type`).
pub mod validate {
    use super::{Kind, Value};
    use math::LegacyDec;

    type R = std::result::Result<(), String>;

    fn bad_type(v: &Value) -> String {
        format!("invalid parameter type: {}", go_type(v.kind()))
    }

    const fn go_type(k: Kind) -> &'static str {
        match k {
            Kind::Uint64 => "uint64",
            Kind::Uint32 => "uint32",
            Kind::Int64 => "int64",
            Kind::Duration => "time.Duration",
            Kind::Bool => "bool",
            Kind::Str => "string",
            Kind::Dec => "types.Dec",
            Kind::Coin => "types.Coin",
            Kind::Strings => "[]string",
            Kind::SendEnabled => "[]*types.SendEnabled",
            Kind::Struct(name) => name,
        }
    }

    /// Accepts any value of the registered type (`validateIsBool`,
    /// `validateHistoricalEntries`, ...); the type check is the subspace's.
    pub fn any(_: &Value) -> R {
        Ok(())
    }

    /// A non-zero `uint64` (auth's validators): `msg` is Go's text before
    /// the value.
    pub fn nonzero_u64(v: &Value, msg: &str) -> R {
        match v {
            Value::Uint64(0) => Err(format!("{msg}: 0")),
            Value::Uint64(_) => Ok(()),
            _ => Err(bad_type(v)),
        }
    }

    /// A non-zero `uint32` (staking `MaxValidators`, `MaxEntries`).
    pub fn nonzero_u32(v: &Value, msg: &str) -> R {
        match v {
            Value::Uint32(0) => Err(format!("{msg}: 0")),
            Value::Uint32(_) => Ok(()),
            _ => Err(bad_type(v)),
        }
    }

    /// A positive duration (the error text shows nanoseconds).
    pub fn positive_duration(v: &Value, msg: &str) -> R {
        match v {
            Value::Duration(n) if *n <= 0 => Err(format!("{msg}: {n}")),
            Value::Duration(_) => Ok(()),
            _ => Err(bad_type(v)),
        }
    }

    /// A positive `int64`.
    pub fn positive_i64(v: &Value, msg: &str) -> R {
        match v {
            Value::Int64(n) if *n <= 0 => Err(format!("{msg}: {n}")),
            Value::Int64(_) => Ok(()),
            _ => Err(bad_type(v)),
        }
    }

    /// A decimal in `[0, 1]`: `negative` and `large` are Go's messages.
    pub fn unit_dec(v: &Value, negative: &str, large: &str) -> R {
        match v {
            Value::Dec(d) if d.is_negative() => Err(format!("{negative}: {d}")),
            Value::Dec(d) if *d > LegacyDec::one() => Err(format!("{large}: {d}")),
            Value::Dec(_) => Ok(()),
            _ => Err(bad_type(v)),
        }
    }

    /// `sdk.ValidateDenom` of a string parameter (staking `BondDenom`).
    pub fn bond_denom(v: &Value) -> R {
        match v {
            Value::Str(s) if s.trim().is_empty() => Err("bond denom cannot be blank".to_owned()),
            Value::Str(s) => denom(s),
            _ => Err(bad_type(v)),
        }
    }

    /// `sdk.ValidateDenom`.
    pub fn denom(s: &str) -> R {
        if sdk::coins::is_valid_denom(s) {
            Ok(())
        } else {
            Err(format!("invalid denom: {s}"))
        }
    }

    /// bank `validateSendEnabledParams`: valid denoms, each at most once.
    pub fn send_enabled(v: &Value) -> R {
        let Value::SendEnabled(list) = v else {
            return Err(bad_type(v));
        };
        let mut seen = std::collections::BTreeSet::new();
        for e in list.iter().flatten() {
            if !seen.insert(e.denom.as_str()) {
                return Err(format!(
                    "duplicate send enabled parameter found: '{}'",
                    e.denom
                ));
            }
            denom(&e.denom)?;
        }
        Ok(())
    }

    /// ibc-go 02-client `validateClients`: no blank client type.
    pub fn clients(v: &Value) -> R {
        let Value::Strings(list) = v else {
            return Err(bad_type(v));
        };
        for (i, c) in list.iter().flatten().enumerate() {
            if c.trim().is_empty() {
                return Err(format!("client type {i} cannot be blank"));
            }
        }
        Ok(())
    }

    /// Type check only, for the validators that check nothing else.
    pub fn of_kind(v: &Value, k: Kind) -> R {
        if v.kind() == k {
            Ok(())
        } else {
            Err(bad_type(v))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_encode_like_go_amino() {
        let dec = LegacyDec::from_str_go("0.02").unwrap_or_else(|_| LegacyDec::zero());
        let cases = [
            (Value::Uint64(256), "\"256\""),
            (Value::Uint32(100), "100"),
            (Value::Int64(-1), "\"-1\""),
            (Value::Duration(60_000_000_000), "\"60000000000\""),
            (Value::Bool(true), "true"),
            (Value::Str("a<b".to_owned()), "\"a\\u003cb\""),
            (Value::Dec(dec), "\"0.020000000000000000\""),
            (
                Value::Coin {
                    denom: "utia".to_owned(),
                    amount: Int::from(1000_i64),
                },
                r#"{"denom":"utia","amount":"1000"}"#,
            ),
            (Value::Strings(None), "null"),
            (Value::Strings(Some(Vec::new())), "[]"),
            (
                Value::Strings(Some(vec!["07-tendermint".to_owned()])),
                r#"["07-tendermint"]"#,
            ),
            (Value::SendEnabled(Some(Vec::new())), "[]"),
            (
                Value::SendEnabled(Some(vec![
                    SendEnabled {
                        denom: "utia".to_owned(),
                        enabled: false,
                    },
                    SendEnabled {
                        denom: String::new(),
                        enabled: true,
                    },
                ])),
                r#"[{"denom":"utia"},{"enabled":true}]"#,
            ),
        ];
        for (v, want) in cases {
            assert_eq!(v.amino_json(), want, "{v:?}");
        }
    }

    const TABLE: &[Param] = &[
        param("A", Kind::Uint64, |v| validate::nonzero_u64(v, "bad a")),
        param("B", Kind::Bool, validate::any),
    ];
    const SPACE: Subspace = Subspace {
        name: "test",
        table: TABLE,
    };

    fn with_ctx(f: impl FnOnce(&mut Context<'_>)) {
        let mut ms = sdk::MultiStore::new();
        ms.mount(crate::STORE_KEY, sdk::Kind::Iavl).unwrap_or(());
        ms.mount(crate::TSTORE_KEY, sdk::Kind::Transient)
            .unwrap_or(());
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        f(&mut ctx);
    }

    #[test]
    fn set_param_set_validates_and_checks_the_key_table() {
        with_ctx(|ctx| {
            assert!(
                SPACE
                    .set_param_set(ctx, &[("A", Value::Uint64(7)), ("B", Value::Bool(false))])
                    .is_ok()
            );
            assert_eq!(
                crate::get_raw(ctx, "test", b"A").ok().flatten(),
                Some(b"\"7\"".to_vec())
            );
            assert_eq!(
                crate::get_raw(ctx, "test", b"B").ok().flatten(),
                Some(b"false".to_vec())
            );
            let invalid = SPACE.set_param_set(ctx, &[("A", Value::Uint64(0))]);
            assert!(invalid.is_err_and(|e| e.log.contains("bad a: 0")));
            assert!(SPACE.set(ctx, "C", &Value::Bool(true)).is_err());
            assert!(SPACE.set(ctx, "B", &Value::Uint64(1)).is_err());
        });
    }
}
