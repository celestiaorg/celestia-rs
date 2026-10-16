//! ABCI events (`sdk.EventManager`).

use celestia_core_proto::abci::{Event, EventAttribute};

/// Builds an event with `index = true` attributes, as `sdk.NewEvent` +
/// `sdk.NewAttribute` produce (the SDK marks all attributes indexed by
/// default in `ToABCIEvents`).
pub fn event<K: Into<String>, V: Into<String>>(
    ty: impl Into<String>,
    attrs: impl IntoIterator<Item = (K, V)>,
) -> Event {
    Event {
        r#type: ty.into(),
        attributes: attrs
            .into_iter()
            .map(|(k, v)| EventAttribute {
                key: k.into(),
                value: v.into(),
                index: true,
            })
            .collect(),
    }
}

/// Collects events emitted during execution.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventManager {
    events: Vec<Event>,
}

impl EventManager {
    /// Emits one event.
    pub fn emit(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Emits several events.
    pub fn emit_all(&mut self, es: impl IntoIterator<Item = Event>) {
        self.events.extend(es);
    }

    /// Number of events so far (a checkpoint for [`Self::truncate`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when no events were emitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Drops events after a checkpoint (used when a branch is discarded).
    pub fn truncate(&mut self, len: usize) {
        self.events.truncate(len);
    }

    /// Takes all events.
    pub fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Borrows the events.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Appends an attribute to every event emitted since checkpoint `from`
    /// (e.g. authz's `authz_msg_index`).
    pub fn append_attribute_since(&mut self, from: usize, key: &str, value: &str) {
        for e in self.events.iter_mut().skip(from) {
            e.attributes.push(EventAttribute {
                key: key.into(),
                value: value.into(),
                index: true,
            });
        }
    }
}

/// `sdk.TypedEventToEvent`: the event type is the proto full name and each
/// top-level field of the message's proto3 JSON (original field names,
/// defaults emitted) becomes an attribute whose value is the field's compact
/// JSON, in sorted key order.
///
/// Go's `jsonpb` escapes `<`, `>` and `&` in strings as `\u003c` etc., and
/// each invalid UTF-8 byte of a Go string as `\ufffd`; this helper applies
/// the same escaping so values match byte-for-byte.
pub fn typed_event<T: prost::Name + serde::Serialize>(msg: &T) -> Result<Event, serde_json::Error> {
    // Serialized directly (not through `serde_json::Value`, whose maps
    // sort their keys): nested messages keep jsonpb's field order.
    let s = serde_json::to_string(msg)?;
    let Some(members) = object_members(&s) else {
        return typed_event_from_json(T::full_name(), serde_json::from_str(&s)?);
    };
    let mut attrs: Vec<(String, String)> = members
        .into_iter()
        .map(|(k, v)| (k.to_owned(), attr_value(&go_timestamps(v))))
        .collect();
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(event(T::full_name(), attrs))
}

/// The members of the compact JSON object `s` (keys without escapes, raw
/// values), or `None` when `s` is not such an object.
fn object_members(s: &str) -> Option<Vec<(&str, &str)>> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') || bytes.last() != Some(&b'}') {
        return None;
    }
    let mut out = Vec::new();
    let mut pos = 1;
    while pos < bytes.len() - 1 {
        // Key.
        if bytes[pos] != b'"' {
            return None;
        }
        let ks = pos + 1;
        let ke = ks + bytes[ks..].iter().position(|&c| c == b'"' || c == b'\\')?;
        if bytes[ke] != b'"' || bytes.get(ke + 1) != Some(&b':') {
            return None;
        }
        // Value: up to the next top-level ',' or the closing '}'.
        let vs = ke + 2;
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        let mut end = vs;
        while end < bytes.len() - 1 {
            let c = bytes[end];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {}
                }
            }
            end += 1;
        }
        out.push((&s[ks..ke], &s[vs..end]));
        pos = end + 1;
    }
    Some(out)
}

/// pbjson writes a `Timestamp` as RFC 3339 with a `+00:00` offset; Go's
/// jsonpb ends it with `Z` (same 0 / 3 / 6 / 9 fractional digits).
fn go_timestamps(v: &str) -> std::borrow::Cow<'_, str> {
    const SUFFIX: &str = "+00:00\"";
    if !v.contains(SUFFIX) {
        return v.into();
    }
    let is_ts = |t: &[u8]| {
        // YYYY-MM-DDTHH:MM:SS[.fraction]
        t.len() >= 19
            && t.iter().enumerate().take(19).all(|(i, &c)| match i {
                4 | 7 => c == b'-',
                10 => c == b'T',
                13 | 16 => c == b':',
                _ => c.is_ascii_digit(),
            })
            && (t.len() == 19
                || (t[19] == b'.' && t.len() > 20 && t[20..].iter().all(u8::is_ascii_digit)))
    };
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(p) = rest.find(SUFFIX) {
        let head = &rest[..p];
        let start = head.rfind('"').map_or(0, |q| q + 1);
        out.push_str(head);
        if head.len() > start && is_ts(&head.as_bytes()[start..]) {
            out.push_str("Z\"");
        } else {
            out.push_str(SUFFIX);
        }
        rest = &rest[p + SUFFIX.len()..];
    }
    out.push_str(rest);
    out.into()
}

/// An attribute value: Go's jsonpb escaping of `<`, `>`, `&` and of each
/// invalid UTF-8 byte.
fn attr_value(json: &str) -> String {
    let s = json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    gostr::json_escape_invalid(&s).into_owned()
}

/// [`typed_event`] from an already-built proto3-JSON object, for messages
/// whose gogoproto `customtype` fields (e.g. `LegacyDec` stored as its
/// scaled-integer string) need their JSON form patched in first.
pub fn typed_event_from_json(
    full_name: impl Into<String>,
    value: serde_json::Value,
) -> Result<Event, serde_json::Error> {
    let mut attrs: Vec<(String, String)> = Vec::new();
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            // `json.Marshal` writes each invalid UTF-8 byte of a Go string
            // as `\ufffd` (strings here are Go forms, see `gostr`).
            attrs.push((k, attr_value(&serde_json::to_string(&v)?)));
        }
    }
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(event(full_name, attrs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nested messages keep jsonpb's field order and timestamps end in
    /// `Z` (devnet `fibre`, celestia-app v10.4.0).
    #[test]
    fn typed_event_nested_order_and_time() {
        use celestia_proto::celestia::fibre::v1::EventWithdrawFromEscrowRequest;
        let e = typed_event(&EventWithdrawFromEscrowRequest {
            signer: "s".into(),
            amount: Some(celestia_proto::cosmos::base::v1beta1::Coin {
                denom: "utia".into(),
                amount: "1000000".into(),
            }),
            requested_at: Some(pbjson_types::Timestamp {
                seconds: 1_791_969_985,
                nanos: 444_152_280,
            }),
            available_at: Some(pbjson_types::Timestamp {
                seconds: 1_791_970_045,
                nanos: 0,
            }),
        })
        .unwrap_or_default();
        let kv: Vec<_> = e
            .attributes
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str()))
            .collect();
        assert_eq!(
            kv,
            [
                ("amount", "{\"denom\":\"utia\",\"amount\":\"1000000\"}"),
                ("available_at", "\"2026-10-14T09:27:25Z\""),
                ("requested_at", "\"2026-10-14T09:26:25.444152280Z\""),
                ("signer", "\"s\""),
            ]
        );
    }

    #[test]
    fn typed_event_sorted_json_attrs() {
        let e = typed_event(&celestia_proto::celestia::blob::v1::EventPayForBlobs {
            signer: "a<b".into(),
            blob_sizes: vec![1, 2],
            namespaces: vec![vec![0]],
        })
        .unwrap_or_default();
        assert_eq!(e.r#type, "celestia.blob.v1.EventPayForBlobs");
        let kv: Vec<_> = e
            .attributes
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str()))
            .collect();
        assert_eq!(
            kv,
            [
                ("blob_sizes", "[1,2]"),
                ("namespaces", "[\"AA==\"]"),
                ("signer", "\"a\\u003cb\""),
            ]
        );
    }
}
