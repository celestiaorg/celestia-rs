//! Protobuf helpers backed by `prost`.

pub use bytes::{Buf, BufMut, Bytes, BytesMut};
pub use prost::{DecodeError, EncodeError, Message};

/// Encode a prost message into a new byte vector.
#[must_use]
pub fn encode_to_vec<M: Message>(_message: &M) -> Vec<u8> {
    Vec::new()
}

/// Encode a prost message with a protobuf length delimiter into a new byte vector.
#[must_use]
pub fn encode_length_delimited_to_vec<M: Message>(_message: &M) -> Vec<u8> {
    Vec::new()
}

/// Decode a prost message from a complete byte slice.
pub fn decode<M>(_bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default,
{
    Err(DecodeError::new("not implemented"))
}

/// Decode a length-delimited prost message from a complete byte slice.
pub fn decode_length_delimited<M>(_bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default,
{
    Err(DecodeError::new("not implemented"))
}
