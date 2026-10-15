//! Typed events (`EventManager.EmitTypedEvent`).
//!
//! Go converts a typed event with `sdk.TypedEventToEvent`: the event type is
//! the proto full name and every field becomes an attribute whose value is
//! the field's jsonpb encoding (`OrigName`, `EmitDefaults`), sorted by key.

use std::fmt::Write as _;

use celestia_proto::celestia_core_proto::abci::Event;

use crate::util::HexAddress;

/// A jsonpb-encoded field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonValue {
    /// `string` (and customtypes that marshal to a JSON string).
    Str(String),
    /// `uint32`/`int32`: a JSON number.
    Num(u64),
    /// `uint64`/`int64`: a quoted number.
    Quoted(u64),
    /// `bool`.
    Bool(bool),
    /// A nil nullable field.
    Null,
    /// `repeated string`.
    StrList(Vec<String>),
}

impl JsonValue {
    /// Nullable `HexAddress` customtype.
    #[must_use]
    pub fn opt_addr(a: Option<&HexAddress>) -> Self {
        a.map_or(Self::Null, |a| Self::Str(a.to_string()))
    }

    /// Non-nullable `HexAddress` customtype.
    #[must_use]
    pub fn addr(a: &HexAddress) -> Self {
        Self::Str(a.to_string())
    }

    fn encode(&self) -> String {
        match self {
            Self::Str(s) => go_json_string(s),
            Self::Num(n) => n.to_string(),
            Self::Quoted(n) => format!("\"{n}\""),
            Self::Bool(b) => b.to_string(),
            Self::Null => "null".into(),
            Self::StrList(v) => {
                let items: Vec<String> = v.iter().map(|s| go_json_string(s)).collect();
                format!("[{}]", items.join(","))
            }
        }
    }
}

/// `encoding/json` string encoding with Go's HTML-safe escaping of the Go
/// string `s` (Go form, `gostr`): invalid UTF-8 bytes become `\ufffd`.
#[must_use]
pub fn go_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for r in gostr::runes(s) {
        let Some(ch) = r else {
            out.push_str("\\ufffd");
            continue;
        };
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => gostr::push_char(&mut out, c),
        }
    }
    out.push('"');
    out
}

/// Builds the ABCI event for typed event `M` with the given fields.
#[must_use]
pub fn typed_event<M: prost::Name>(fields: Vec<(&str, JsonValue)>) -> Event {
    let mut fields = fields;
    fields.sort_by(|a, b| a.0.cmp(b.0));
    sdk::event::event(
        M::full_name(),
        fields.into_iter().map(|(k, v)| (k.to_string(), v.encode())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_escaping() {
        assert_eq!(
            go_json_string("a<b>&\"\n\u{1}"),
            r#""a\u003cb\u003e\u0026\"\n\u0001""#
        );
    }

    #[test]
    fn sorted_attributes() {
        let e = typed_event::<celestia_proto::hyperlane::core::v1::EventDispatch>(vec![
            ("sender", JsonValue::Str("x".into())),
            ("destination", JsonValue::Num(5)),
        ]);
        assert_eq!(e.r#type, "hyperlane.core.v1.EventDispatch");
        assert_eq!(e.attributes[0].key, "destination");
        assert_eq!(e.attributes[0].value, "5");
        assert_eq!(e.attributes[1].value, "\"x\"");
    }
}
