//! Protobuf helpers backed by `prost`.

pub use bytes::{Buf, BufMut, Bytes, BytesMut};
pub use prost::{DecodeError, EncodeError, Message};

/// Encode a prost message into a new byte vector.
#[must_use]
pub fn encode_to_vec<M: Message>(message: &M) -> Vec<u8> {
    message.encode_to_vec()
}

/// Encode a prost message with a protobuf length delimiter into a new byte vector.
#[must_use]
pub fn encode_length_delimited_to_vec<M: Message>(message: &M) -> Vec<u8> {
    message.encode_length_delimited_to_vec()
}

/// Decode a prost message from a complete byte slice.
pub fn decode<M>(bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default,
{
    M::decode(bytes)
}

/// Decode a length-delimited prost message from a complete byte slice.
pub fn decode_length_delimited<M>(bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default,
{
    M::decode_length_delimited(bytes)
}
