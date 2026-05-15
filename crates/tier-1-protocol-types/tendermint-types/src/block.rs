//! Block IDs, part-set headers, and block headers.

use crate::error::{Error, Result};
use crate::hash::{
    Address, Hash, address_from_bytes, optional_hash_from_bytes, optional_hash_to_vec,
};
use crate::proto;
use prost::Message;

/// Maximum CometBFT chain ID length.
pub const MAX_CHAIN_ID_LENGTH: usize = 50;

/// CometBFT consensus version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusVersion {
    /// Block protocol version.
    pub block: u64,
    /// Application protocol version.
    pub app: u64,
}

impl ConsensusVersion {
    fn from_proto(proto: proto::version::Consensus) -> Self {
        Self {
            block: proto.block,
            app: proto.app,
        }
    }

    fn to_proto(self) -> proto::version::Consensus {
        proto::version::Consensus {
            block: self.block,
            app: self.app,
        }
    }
}

/// CometBFT part-set header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartSetHeader {
    /// Number of parts.
    pub total: u32,
    /// Merkle root for all parts.
    pub hash: Option<Hash>,
}

impl PartSetHeader {
    /// Return true when this is the zero part-set header.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.total == 0 && self.hash.is_none()
    }

    /// Perform CometBFT stateless part-set header validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.total == 0 {
            if self.hash.is_some() {
                return Err(Error::InvalidPartSetHeader);
            }
            return Ok(());
        }
        if self.hash.is_none() {
            return Err(Error::InvalidPartSetHeader);
        }
        Ok(())
    }

    /// Convert protobuf into a domain part-set header.
    pub fn from_proto(proto: proto::types::PartSetHeader) -> Result<Self> {
        let header = Self {
            total: proto.total,
            hash: optional_hash_from_bytes(&proto.hash)?,
        };
        header.validate_basic()?;
        Ok(header)
    }

    /// Convert this part-set header to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::PartSetHeader {
        proto::types::PartSetHeader {
            total: self.total,
            hash: optional_hash_to_vec(self.hash.as_ref()),
        }
    }
}

/// CometBFT block ID.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockId {
    /// Block hash.
    pub hash: Option<Hash>,
    /// Block part-set header.
    pub part_set_header: PartSetHeader,
}

impl BlockId {
    /// Return true when this is the nil block ID.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.hash.is_none() && self.part_set_header.is_zero()
    }

    /// Return true when this is a complete non-nil block ID.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.hash.is_some() && self.part_set_header.total > 0 && self.part_set_header.hash.is_some()
    }

    /// Perform CometBFT stateless block-ID validation.
    pub fn validate_basic(&self) -> Result<()> {
        self.part_set_header.validate_basic()?;
        if self.hash.is_none() && !self.part_set_header.is_zero() {
            return Err(Error::InvalidBlockId);
        }
        Ok(())
    }

    /// Convert protobuf into a domain block ID.
    pub fn from_proto(proto: proto::types::BlockId) -> Result<Self> {
        let block_id = Self {
            hash: optional_hash_from_bytes(&proto.hash)?,
            part_set_header: proto
                .part_set_header
                .map(PartSetHeader::from_proto)
                .transpose()?
                .unwrap_or_default(),
        };
        block_id.validate_basic()?;
        Ok(block_id)
    }

    /// Convert this block ID to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::BlockId {
        proto::types::BlockId {
            hash: optional_hash_to_vec(self.hash.as_ref()),
            part_set_header: Some(self.part_set_header.to_proto()),
        }
    }
}

/// CometBFT block header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Consensus version.
    pub version: ConsensusVersion,
    /// Chain ID.
    pub chain_id: String,
    /// Block height.
    pub height: i64,
    /// Block timestamp.
    pub time: proto::google::Timestamp,
    /// Previous block ID.
    pub last_block_id: BlockId,
    /// Last commit hash.
    pub last_commit_hash: Option<Hash>,
    /// Data hash.
    pub data_hash: Option<Hash>,
    /// Current validator-set hash.
    pub validators_hash: Option<Hash>,
    /// Next validator-set hash.
    pub next_validators_hash: Option<Hash>,
    /// Consensus params hash.
    pub consensus_hash: Option<Hash>,
    /// Application hash.
    pub app_hash: Vec<u8>,
    /// Last results hash.
    pub last_results_hash: Option<Hash>,
    /// Evidence hash.
    pub evidence_hash: Option<Hash>,
    /// Proposer address.
    pub proposer_address: Address,
}

impl Header {
    /// Perform CometBFT stateless header validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.chain_id.len() > MAX_CHAIN_ID_LENGTH {
            return Err(Error::InvalidChainIdLength {
                actual: self.chain_id.len(),
                max: MAX_CHAIN_ID_LENGTH,
            });
        }
        if self.height <= 0 {
            return Err(Error::InvalidHeight);
        }
        self.last_block_id.validate_basic()?;
        Ok(())
    }

    /// Return the CometBFT header hash.
    #[must_use]
    pub fn hash(&self) -> Option<Hash> {
        self.validators_hash.as_ref()?;
        let fields = vec![
            self.version.to_proto().encode_to_vec(),
            cdc_encode_string(&self.chain_id),
            cdc_encode_i64(self.height),
            self.time.encode_to_vec(),
            self.last_block_id.to_proto().encode_to_vec(),
            cdc_encode_bytes(self.last_commit_hash.as_ref()),
            cdc_encode_bytes(self.data_hash.as_ref()),
            cdc_encode_bytes(self.validators_hash.as_ref()),
            cdc_encode_bytes(self.next_validators_hash.as_ref()),
            cdc_encode_bytes(self.consensus_hash.as_ref()),
            cdc_encode_raw_bytes(&self.app_hash),
            cdc_encode_bytes(self.last_results_hash.as_ref()),
            cdc_encode_bytes(self.evidence_hash.as_ref()),
            cdc_encode_raw_bytes(&self.proposer_address),
        ];
        Some(merkle::root_from_byte_slices(&fields))
    }

    /// Convert protobuf into a domain header.
    pub fn from_proto(proto: proto::types::Header) -> Result<Self> {
        let header = Self {
            version: ConsensusVersion::from_proto(proto.version.unwrap_or_default()),
            chain_id: proto.chain_id,
            height: proto.height,
            time: proto.time.ok_or(Error::InvalidTimestamp)?,
            last_block_id: proto
                .last_block_id
                .map(BlockId::from_proto)
                .transpose()?
                .unwrap_or_default(),
            last_commit_hash: optional_hash_from_bytes(&proto.last_commit_hash)?,
            data_hash: optional_hash_from_bytes(&proto.data_hash)?,
            validators_hash: optional_hash_from_bytes(&proto.validators_hash)?,
            next_validators_hash: optional_hash_from_bytes(&proto.next_validators_hash)?,
            consensus_hash: optional_hash_from_bytes(&proto.consensus_hash)?,
            app_hash: proto.app_hash,
            last_results_hash: optional_hash_from_bytes(&proto.last_results_hash)?,
            evidence_hash: optional_hash_from_bytes(&proto.evidence_hash)?,
            proposer_address: address_from_bytes(&proto.proposer_address)?,
        };
        header.validate_basic()?;
        Ok(header)
    }

    /// Convert this header to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::Header {
        proto::types::Header {
            version: Some(self.version.to_proto()),
            chain_id: self.chain_id.clone(),
            height: self.height,
            time: Some(self.time),
            last_block_id: Some(self.last_block_id.to_proto()),
            last_commit_hash: optional_hash_to_vec(self.last_commit_hash.as_ref()),
            data_hash: optional_hash_to_vec(self.data_hash.as_ref()),
            validators_hash: optional_hash_to_vec(self.validators_hash.as_ref()),
            next_validators_hash: optional_hash_to_vec(self.next_validators_hash.as_ref()),
            consensus_hash: optional_hash_to_vec(self.consensus_hash.as_ref()),
            app_hash: self.app_hash.clone(),
            last_results_hash: optional_hash_to_vec(self.last_results_hash.as_ref()),
            evidence_hash: optional_hash_to_vec(self.evidence_hash.as_ref()),
            proposer_address: self.proposer_address.to_vec(),
        }
    }

    /// Decode a protobuf header.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let proto =
            proto::types::Header::decode(bytes).map_err(|e| Error::Decode(e.to_string()))?;
        Self::from_proto(proto)
    }

    /// Encode this header to protobuf bytes.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>> {
        Ok(self.to_proto().encode_to_vec())
    }
}

#[derive(Clone, PartialEq, Message)]
struct StringValue {
    #[prost(string, tag = "1")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct Int64Value {
    #[prost(int64, tag = "1")]
    value: i64,
}

#[derive(Clone, PartialEq, Message)]
struct BytesValue {
    #[prost(bytes = "vec", tag = "1")]
    value: Vec<u8>,
}

fn cdc_encode_string(value: &str) -> Vec<u8> {
    if value.is_empty() {
        return Vec::new();
    }
    StringValue {
        value: value.to_owned(),
    }
    .encode_to_vec()
}

fn cdc_encode_i64(value: i64) -> Vec<u8> {
    Int64Value { value }.encode_to_vec()
}

fn cdc_encode_bytes(value: Option<&Hash>) -> Vec<u8> {
    value.map_or_else(Vec::new, |value| cdc_encode_raw_bytes(value))
}

fn cdc_encode_raw_bytes(value: &[u8]) -> Vec<u8> {
    if value.is_empty() {
        return Vec::new();
    }
    BytesValue {
        value: value.to_vec(),
    }
    .encode_to_vec()
}
