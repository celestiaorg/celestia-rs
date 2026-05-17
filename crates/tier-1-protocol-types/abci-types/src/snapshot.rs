//! State-sync snapshot types.

use crate::{Error, Result, proto};

/// State-sync snapshot metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Snapshot height.
    pub height: u64,
    /// Application-specific snapshot format.
    pub format: u32,
    /// Number of chunks in the snapshot.
    pub chunks: u32,
    /// Opaque snapshot hash bytes.
    pub hash: Vec<u8>,
    /// Opaque application metadata.
    pub metadata: Vec<u8>,
}

impl Snapshot {
    /// Perform stateless snapshot validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height == 0 || self.format == 0 || self.chunks == 0 {
            return Err(Error::InvalidSnapshot);
        }
        Ok(())
    }

    /// Convert protobuf into a domain snapshot.
    pub fn from_proto(proto: proto::abci::Snapshot) -> Result<Self> {
        let snapshot = Self {
            height: proto.height,
            format: proto.format,
            chunks: proto.chunks,
            hash: proto.hash.to_vec(),
            metadata: proto.metadata.to_vec(),
        };
        snapshot.validate_basic()?;
        Ok(snapshot)
    }

    /// Convert this snapshot to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Snapshot {
        proto::abci::Snapshot {
            height: self.height,
            format: self.format,
            chunks: self.chunks,
            hash: self.hash.clone().into(),
            metadata: self.metadata.clone().into(),
        }
    }
}
