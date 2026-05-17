//! Cosmos protobuf Any domain type.

use crate::error::{Error, Result};
use crate::proto;

/// Cosmos protobuf `Any`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Any {
    /// Type URL, for example `/cosmos.crypto.secp256k1.PubKey`.
    pub type_url: String,
    /// Raw protobuf value bytes.
    pub value: Vec<u8>,
}

impl Any {
    /// Perform Cosmos SDK stateless `Any` validation.
    pub fn validate_basic(&self) -> Result<()> {
        if !is_valid_type_url(&self.type_url) {
            return Err(Error::InvalidAny);
        }
        Ok(())
    }

    /// Convert protobuf into a domain `Any`.
    pub fn from_proto(proto: proto::google::Any) -> Result<Self> {
        let any = Self {
            type_url: proto.type_url,
            value: proto.value,
        };
        any.validate_basic()?;
        Ok(any)
    }

    /// Convert this value to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::google::Any {
        proto::google::Any {
            type_url: self.type_url.clone(),
            value: self.value.clone(),
        }
    }
}

fn is_valid_type_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('/') else {
        return false;
    };
    !rest.is_empty()
        && rest
            .split('.')
            .all(|segment| !segment.is_empty() && segment.chars().all(is_type_url_char))
}

fn is_type_url_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '_'
}
