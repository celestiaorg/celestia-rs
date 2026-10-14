//! `BlockID` and `ValidateHash`.

use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::part_set::PartSetHeader;
use crate::proto::types as pb;
use prost::Message;

/// `tmhash.Size`.
pub const HASH_SIZE: usize = 32;

/// `ValidateHash`: empty or exactly 32 bytes.
pub fn validate_hash(h: &[u8]) -> Result<()> {
    if !h.is_empty() && h.len() != HASH_SIZE {
        return Err(Error::msg(format!(
            "expected size to be {HASH_SIZE} bytes, got {} bytes",
            h.len()
        )));
    }
    Ok(())
}

/// A block identifier: header hash plus part-set header.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct BlockId {
    /// Block (header) hash; empty for nil.
    pub hash: Vec<u8>,
    /// Part-set header.
    pub part_set_header: PartSetHeader,
}

impl BlockId {
    /// `BlockID.Equals`.
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self == other
    }

    /// `BlockID.Key`: `string(hash) + string(proto(part_set_header))`.
    #[must_use]
    pub fn key(&self) -> Vec<u8> {
        let mut k = self.hash.clone();
        k.extend(self.part_set_header.to_proto().encode_to_vec());
        k
    }

    /// `BlockID.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if validate_hash(&self.hash).is_err() {
            return Err(Error::msg("wrong Hash"));
        }
        self.part_set_header
            .validate_basic()
            .map_err(|e| e.wrap("wrong PartSetHeader: "))
    }

    /// `BlockID.IsZero`: the nil block.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.hash.is_empty() && self.part_set_header.is_zero()
    }

    /// `BlockID.IsComplete`.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.hash.len() == HASH_SIZE
            && self.part_set_header.total > 0
            && self.part_set_header.hash.len() == HASH_SIZE
    }

    /// `BlockID.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::BlockId {
        pb::BlockId {
            hash: self.hash.clone(),
            part_set_header: Some(self.part_set_header.to_proto()),
        }
    }

    /// Converts without validation (Go's in-memory conversion).
    #[must_use]
    pub fn from_proto_unvalidated(p: &pb::BlockId) -> Self {
        Self {
            hash: p.hash.clone(),
            part_set_header: p
                .part_set_header
                .as_ref()
                .map(PartSetHeader::from_proto_unvalidated)
                .unwrap_or_default(),
        }
    }

    /// `BlockIDFromProto` (validates).
    pub fn from_proto(p: &pb::BlockId) -> Result<Self> {
        let psh = p
            .part_set_header
            .as_ref()
            .map(PartSetHeader::from_proto)
            .transpose()?
            .unwrap_or_default();
        let id = Self {
            hash: p.hash.clone(),
            part_set_header: psh,
        };
        id.validate_basic()?;
        Ok(id)
    }
}

impl core::fmt::Display for BlockId {
    /// `BlockID.String`: `HASH:total:FINGERPRINT`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", hex_upper(&self.hash), self.part_set_header)
    }
}

/// `ProtoBlockIDIsNil`.
#[must_use]
pub fn proto_block_id_is_nil(id: &pb::BlockId) -> bool {
    id.hash.is_empty()
        && id
            .part_set_header
            .as_ref()
            .is_none_or(|p| p.total == 0 && p.hash.is_empty())
}
