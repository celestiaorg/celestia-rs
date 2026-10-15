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
    typed_event_from_json(T::full_name(), serde_json::to_value(msg)?)
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
            let s = serde_json::to_string(&v)?
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace('&', "\\u0026");
            // `json.Marshal` writes each invalid UTF-8 byte of a Go string
            // as `\ufffd` (strings here are Go forms, see `gostr`).
            attrs.push((k, gostr::json_escape_invalid(&s).into_owned()));
        }
    }
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(event(full_name, attrs))
}

#[cfg(test)]
mod tests {
    use super::*;

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
