//! The go-jsonrpc envelope is shared by HTTP calls and socket notifications.
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Deserialize)]
pub(super) struct Request {
    jsonrpc: String,
    #[serde(default, deserialize_with = "present_id")]
    pub(super) id: Option<Value>,
    #[serde(default)]
    pub(super) method: Value,
    #[serde(default)]
    pub(super) params: Value,
}

// Missing IDs identify notifications. An explicit null is a JSON-RPC call ID.
fn present_id<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(de).map(Some)
}

impl Request {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, (i32, String)> {
        let request: Self = serde_json::from_slice(bytes).map_err(|err| {
            if err.is_data() {
                (-32600, "invalid request".into())
            } else {
                (-32700, "parse error".into())
            }
        })?;
        if request.jsonrpc != "2.0"
            || request
                .id
                .as_ref()
                .is_some_and(|id| !(id.is_null() || id.is_number() || id.is_string()))
        {
            return Err((-32600, "invalid request".into()));
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_and_call_ids_are_distinct() {
        assert!(
            Request::parse(br#"{"jsonrpc":"2.0","method":"xrpc.cancel"}"#)
                .unwrap()
                .id
                .is_none()
        );
        assert_eq!(
            Request::parse(br#"{"jsonrpc":"2.0","method":"header.LocalHead","id":null}"#)
                .unwrap()
                .id,
            Some(Value::Null)
        );
        assert_eq!(
            Request::parse(br#"{"jsonrpc":"2.0","id":[]}"#)
                .err()
                .unwrap()
                .0,
            -32600
        );
        assert_eq!(Request::parse(b"{").err().unwrap().0, -32700);
    }
}
