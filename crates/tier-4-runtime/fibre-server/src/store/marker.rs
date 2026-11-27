//! Shard markers (Go `store_marker.go`): the metadata value under each
//! `/shard/` key, naming the payload's backend and encoded size.
//!
//! ```text
//! u8 version (= 1) | u8 backend tag | u64be size
//! ```
//!
//! An empty marker is the legacy form: a local payload whose size is read
//! from the file.

use super::StoreError;

const SHARD_MARKER_VERSION: u8 = 1;
const SHARD_MARKER_SIZE: usize = 10;

/// Backend that holds a shard payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendTag {
    /// Flat files under `<store>/shards/`.
    Local = 0x01,
    /// Go's S3-compatible object backend (not supported here).
    Object = 0x02,
}

/// Encodes a versioned marker.
pub(crate) fn encode_marker(backend: BackendTag, size: i64) -> Vec<u8> {
    let mut marker = Vec::with_capacity(SHARD_MARKER_SIZE);
    marker.push(SHARD_MARKER_VERSION);
    marker.push(backend as u8);
    marker.extend_from_slice(&size.to_be_bytes());
    marker
}

/// Decodes a marker into its backend and size; size 0 means legacy (read
/// the size from the local file).
pub(crate) fn decode_marker(data: &[u8]) -> Result<(BackendTag, i64), StoreError> {
    if data.is_empty() {
        return Ok((BackendTag::Local, 0));
    }
    if data.len() != SHARD_MARKER_SIZE {
        return Err(StoreError::Integrity(format!(
            "shard marker length {}, want {SHARD_MARKER_SIZE}",
            data.len()
        )));
    }
    if data[0] != SHARD_MARKER_VERSION {
        return Err(StoreError::Integrity(format!(
            "unsupported shard marker version {}",
            data[0]
        )));
    }
    let backend = match data[1] {
        0x01 => BackendTag::Local,
        0x02 => BackendTag::Object,
        tag => {
            return Err(StoreError::Integrity(format!(
                "unsupported shard backend tag 0x{tag:02x}"
            )));
        }
    };
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&data[2..]);
    let size = u64::from_be_bytes(raw);
    match i64::try_from(size) {
        Ok(size) if size > 0 => Ok((backend, size)),
        _ => Err(StoreError::Integrity(format!("invalid shard size {size}"))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn codec() {
        let marker = encode_marker(BackendTag::Local, 42);
        assert_eq!(marker, [1, 1, 0, 0, 0, 0, 0, 0, 0, 42]);
        assert_eq!(decode_marker(&marker).unwrap(), (BackendTag::Local, 42));
        assert_eq!(decode_marker(&[]).unwrap(), (BackendTag::Local, 0));
        let object = encode_marker(BackendTag::Object, 42);
        assert_eq!(decode_marker(&object).unwrap(), (BackendTag::Object, 42));
    }

    #[test]
    fn rejects_invalid_data() {
        let valid = encode_marker(BackendTag::Local, 1);
        let mut overflow = valid.clone();
        overflow[2..].copy_from_slice(&(u64::MAX / 2 + 1).to_be_bytes());
        let mut zero_backend = valid.clone();
        zero_backend[1] = 0;
        let mut unknown_backend = valid.clone();
        unknown_backend[1] = 3;
        let mut overlong = valid.clone();
        overlong.push(0);
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("truncated", valid[..9].to_vec()),
            ("overlong", overlong),
            ("zero version", [&[0][..], &valid[1..]].concat()),
            ("unsupported version", [&[2][..], &valid[1..]].concat()),
            ("zero backend", zero_backend),
            ("unknown backend", unknown_backend),
            ("zero size", vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
            ("size overflow", overflow),
        ];
        for (name, data) in cases {
            let err = decode_marker(&data).unwrap_err();
            assert!(err.is_integrity(), "{name}: {err}");
        }
    }
}
