//! `google.protobuf.Any` with proto3-JSON (jsonpb) serde support.
//!
//! The JSON form inlines the packed message's fields next to an `@type`
//! key, which requires knowing every message type: see [`crate::registry`].

use serde::de::Error as _;
use serde::ser::Error as _;

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, Eq, Hash, ::prost::Message)]
pub struct Any {
    /// Type URL, e.g. `/cosmos.bank.v1beta1.MsgSend`.
    #[prost(string, tag = "1")]
    pub type_url: ::prost::alloc::string::String,
    /// Protobuf encoding of the packed message.
    #[prost(bytes = "vec", tag = "2")]
    pub value: ::prost::alloc::vec::Vec<u8>,
}

impl Any {
    /// Packs a message.
    pub fn pack<M: ::prost::Name>(msg: &M) -> Self {
        Self {
            type_url: M::type_url(),
            value: msg.encode_to_vec(),
        }
    }

    /// Unpacks into `M` if the type URL matches.
    pub fn unpack<M: ::prost::Name + Default>(&self) -> Option<Result<M, ::prost::DecodeError>> {
        (self.type_url == M::type_url()).then(|| M::decode(self.value.as_slice()))
    }
}

impl ::prost::Name for Any {
    const NAME: &'static str = "Any";
    const PACKAGE: &'static str = "google.protobuf";
    fn full_name() -> String {
        "google.protobuf.Any".into()
    }
    fn type_url() -> String {
        "/google.protobuf.Any".into()
    }
}

impl serde::Serialize for Any {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let json = crate::registry::any_value_to_json(&self.type_url, &self.value)
            .ok_or_else(|| S::Error::custom(format!("unknown Any type {}", self.type_url)))?
            .map_err(S::Error::custom)?;
        let mut map = match json {
            serde_json::Value::Object(m) => m,
            other => {
                let mut m = serde_json::Map::new();
                m.insert("value".into(), other);
                m
            }
        };
        let mut out = serde_json::Map::new();
        out.insert(
            "@type".into(),
            serde_json::Value::String(self.type_url.clone()),
        );
        out.append(&mut map);
        serde_json::Value::Object(out).serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for Any {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let type_url = match map.remove("@type") {
            Some(serde_json::Value::String(t)) => t,
            _ => return Err(D::Error::custom("Any without @type")),
        };
        let value = crate::registry::json_to_any_value(&type_url, serde_json::Value::Object(map))
            .ok_or_else(|| D::Error::custom(format!("unknown Any type {type_url}")))?
            .map_err(D::Error::custom)?;
        Ok(Self { type_url, value })
    }
}
