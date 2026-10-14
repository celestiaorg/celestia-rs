//! JSON <-> bytes conversion for every message, keyed by `Any` type URL.

fn json_to_bytes<M>(value: serde_json::Value) -> Result<Vec<u8>, String>
where
    M: ::prost::Message + serde::de::DeserializeOwned,
{
    let m: M = serde_json::from_value(value).map_err(|e| e.to_string())?;
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
