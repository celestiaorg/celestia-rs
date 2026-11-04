//! JSON <-> bytes conversion for every message, keyed by `Any` type URL.

thread_local! {
    static JSON_CANONICAL_DEPTH: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

struct CanonicalDepthGuard<'a> {
    depth: &'a core::cell::Cell<u32>,
    previous: u32,
}

impl Drop for CanonicalDepthGuard<'_> {
    fn drop(&mut self) {
        self.depth.set(self.previous);
    }
}

fn canonicalize_json_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => *s = gostr::canonical(s).into_owned(),
        serde_json::Value::Array(items) => items.iter_mut().for_each(canonicalize_json_strings),
        serde_json::Value::Object(map) => {
            let old = core::mem::take(map);
            for (key, mut value) in old {
                canonicalize_json_strings(&mut value);
                map.insert(gostr::canonical(&key).into_owned(), value);
            }
        }
        _ => {}
    }
}

fn json_to_bytes<M>(mut value: serde_json::Value) -> Result<Vec<u8>, String>
where
    M: ::prost::Message + serde::de::DeserializeOwned,
{
    let m: M = JSON_CANONICAL_DEPTH.with(|depth| {
        let outermost = depth.get() == 0;
        if outermost {
            // JSON text is UTF-8. Convert its strings to the lossless
            // internal Go form once for the whole tree. A nested Any
            // dispatches through this function again while depth is set.
            canonicalize_json_strings(&mut value);
        }
        let previous = depth.get();
        depth.set(previous + 1);
        let _guard = CanonicalDepthGuard { depth, previous };
        serde_json::from_value(value).map_err(|e| e.to_string())
    })?;
    Ok(m.encode_to_vec())
}

fn bytes_to_json<M>(bytes: &[u8]) -> Result<serde_json::Value, String>
where
    M: ::prost::Message + Default + serde::Serialize,
{
    let m = M::decode(bytes).map_err(|e| e.to_string())?;
    serde_json::to_value(&m).map_err(|e| e.to_string())
}

include!("gen/registry.rs");

/// Convert strings in a protobuf-derived JSON value from their internal Go
/// form to the UTF-8 text Go's JSON encoders write.
pub fn go_form_json_to_text(value: &mut serde_json::Value) {
    fn text(s: &str) -> String {
        let lossy = gostr::lossy(s);
        String::from_utf8(gostr::to_go_bytes(&lossy).into_owned())
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }
    match value {
        serde_json::Value::String(s) => *s = text(s),
        serde_json::Value::Array(items) => items.iter_mut().for_each(go_form_json_to_text),
        serde_json::Value::Object(map) => {
            let old = core::mem::take(map);
            for (key, mut value) in old {
                go_form_json_to_text(&mut value);
                map.insert(text(&key), value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::evidence::v1beta1::{Equivocation, GenesisState};
    use crate::packetforward::v1::GenesisState as PacketforwardGenesis;
    use prost::Message as _;

    const EQUIVOCATION: &str = "/cosmos.evidence.v1beta1.Equivocation";

    #[test]
    fn any_json_preserves_reserved_unicode_as_utf8() {
        let value = serde_json::json!({
            "height": "1",
            "time": "1970-01-01T00:00:01Z",
            "power": "1",
            "consensus_address": "\u{10ffff}",
        });
        let bytes = json_to_any_value(EQUIVOCATION, value)
            .expect("registered")
            .expect("valid JSON");
        assert_eq!(
            bytes,
            [8, 1, 18, 2, 8, 1, 24, 1, 34, 4, 0xf4, 0x8f, 0xbf, 0xbf]
        );

        let decoded = Equivocation::decode(bytes.as_slice()).expect("valid protobuf");
        assert_eq!(
            gostr::to_go_bytes(&decoded.consensus_address).as_ref(),
            "\u{10ffff}".as_bytes()
        );
    }

    #[test]
    fn protobuf_json_go_forms_become_json_text() {
        let message = Equivocation {
            height: 1,
            time: Some(pbjson_types::Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            power: 1,
            consensus_address: gostr::from_go_bytes("\u{10ffff}".as_bytes()).into_owned(),
        };
        let mut json = any_value_to_json(EQUIVOCATION, &message.encode_to_vec())
            .expect("registered")
            .expect("valid protobuf");
        go_form_json_to_text(&mut json);
        assert_eq!(json["consensus_address"], "\u{10ffff}");
    }

    #[test]
    fn protobuf_json_text_distinguishes_invalid_bytes_from_reserved_unicode() {
        let message = Equivocation {
            height: 1,
            time: Some(pbjson_types::Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            power: 1,
            consensus_address: gostr::from_go_bytes(b"bad \x80 \xf4\x8f\xbe\x80").into_owned(),
        };
        let mut json = any_value_to_json(EQUIVOCATION, &message.encode_to_vec())
            .expect("registered")
            .expect("valid protobuf");
        go_form_json_to_text(&mut json);
        assert_eq!(json["consensus_address"], "bad \u{fffd} \u{10ff80}");
    }

    #[test]
    fn nested_any_strings_are_canonicalized_once() {
        let bytes = json_to_any_value(
            "/cosmos.evidence.v1beta1.GenesisState",
            serde_json::json!({
                "evidence": [{
                    "@type": EQUIVOCATION,
                    "height": "1",
                    "time": "1970-01-01T00:00:01Z",
                    "power": "1",
                    "consensus_address": "\u{10ffff}",
                }]
            }),
        )
        .expect("registered")
        .expect("valid JSON");
        let genesis = GenesisState::decode(bytes.as_slice()).expect("valid genesis protobuf");
        let any = genesis.evidence.first().expect("one evidence item");
        assert_eq!(
            any.value,
            [8, 1, 18, 2, 8, 1, 24, 1, 34, 4, 0xf4, 0x8f, 0xbf, 0xbf]
        );
        let decoded = Equivocation::decode(any.value.as_slice()).expect("valid evidence protobuf");
        assert_eq!(
            gostr::to_go_bytes(&decoded.consensus_address).as_ref(),
            "\u{10ffff}".as_bytes()
        );
    }

    #[test]
    fn map_named_at_type_is_not_treated_as_an_any() {
        let bytes = json_to_any_value(
            "/packetforward.v1.GenesisState",
            serde_json::json!({
                "in_flight_packets": {
                    "@type": {"original_sender_address": "\u{10ffff}"}
                }
            }),
        )
        .expect("registered")
        .expect("valid JSON");
        let genesis =
            PacketforwardGenesis::decode(bytes.as_slice()).expect("valid genesis protobuf");
        let packet = genesis.in_flight_packets.get("@type").expect("map entry");
        assert_eq!(
            gostr::to_go_bytes(&packet.original_sender_address).as_ref(),
            "\u{10ffff}".as_bytes()
        );
    }
}
