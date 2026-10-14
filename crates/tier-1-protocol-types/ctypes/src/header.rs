//! Block `Header` (`types/block.go`).

use crate::block_id::{BlockId, validate_hash};
use crate::error::{Error, Result};
use crate::merkle_proof::hash_from_byte_slices;
use crate::proto::{types as pb, version as pbv};
use crate::time::Time;
use prost::Message;

/// `version.BlockProtocol`.
pub const BLOCK_PROTOCOL: u64 = 11;
/// `MaxChainIDLen`.
pub const MAX_CHAIN_ID_LEN: usize = 50;
/// `crypto.AddressSize`.
pub const ADDRESS_SIZE: usize = 20;
/// `MaxHeaderBytes`.
pub const MAX_HEADER_BYTES: i64 = 623;

/// `cmtversion.Consensus`: block and app protocol versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Consensus {
    /// Block protocol (must be [`BLOCK_PROTOCOL`]).
    pub block: u64,
    /// Application version (celestia-app's app version).
    pub app: u64,
}

impl Consensus {
    /// Protobuf form.
    #[must_use]
    pub fn to_proto(self) -> pbv::Consensus {
        pbv::Consensus {
            block: self.block,
            app: self.app,
        }
    }

    /// From protobuf (absent = zero).
    #[must_use]
    pub fn from_proto(p: Option<&pbv::Consensus>) -> Self {
        p.map(|p| Self {
            block: p.block,
            app: p.app,
        })
        .unwrap_or_default()
    }
}

/// A block header. Celestia semantics: `version.app` is the app version,
/// `data_hash` is the data availability root (`DataRootHash` from
/// `PrepareProposal`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Header {
    /// Consensus versions.
    pub version: Consensus,
    /// Chain id.
    pub chain_id: String,
    /// Height.
    pub height: i64,
    /// Block time.
    pub time: Time,
    /// Previous block id.
    pub last_block_id: BlockId,
    /// Hash of the last commit.
    pub last_commit_hash: Vec<u8>,
    /// Data hash (celestia: data root).
    pub data_hash: Vec<u8>,
    /// Validators of this block.
    pub validators_hash: Vec<u8>,
    /// Validators of the next block.
    pub next_validators_hash: Vec<u8>,
    /// Consensus params hash.
    pub consensus_hash: Vec<u8>,
    /// App hash after the previous block.
    pub app_hash: Vec<u8>,
    /// Results hash of the previous block.
    pub last_results_hash: Vec<u8>,
    /// Evidence hash.
    pub evidence_hash: Vec<u8>,
    /// Proposer address.
    pub proposer_address: Vec<u8>,
}

/// `cdcEncode(string)`.
fn cdc_string(s: &str) -> Vec<u8> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    prost::encoding::string::encode(1, &s.to_owned(), &mut out);
    out
}

/// `cdcEncode(int64)`.
fn cdc_i64(v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    if v != 0 {
        prost::encoding::int64::encode(1, &v, &mut out);
    }
    out
}

/// `cdcEncode(HexBytes)`.
pub(crate) fn cdc_bytes(b: &[u8]) -> Vec<u8> {
    if b.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    prost::encoding::bytes::encode(1, &b.to_vec(), &mut out);
    out
}

impl Header {
    /// `Header.Populate`.
    #[allow(clippy::too_many_arguments)]
    pub fn populate(
        &mut self,
        version: Consensus,
        chain_id: String,
        timestamp: Time,
        last_block_id: BlockId,
        val_hash: Vec<u8>,
        next_val_hash: Vec<u8>,
        consensus_hash: Vec<u8>,
        app_hash: Vec<u8>,
        last_results_hash: Vec<u8>,
        proposer_address: Vec<u8>,
    ) {
        self.version = version;
        self.chain_id = chain_id;
        self.time = timestamp;
        self.last_block_id = last_block_id;
        self.validators_hash = val_hash;
        self.next_validators_hash = next_val_hash;
        self.consensus_hash = consensus_hash;
        self.app_hash = app_hash;
        self.last_results_hash = last_results_hash;
        self.proposer_address = proposer_address;
    }

    /// `Header.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.version.block != BLOCK_PROTOCOL {
            return Err(Error::msg(format!(
                "block protocol is incorrect: got: {}, want: {BLOCK_PROTOCOL} ",
                self.version.block
            )));
        }
        if self.chain_id.len() > MAX_CHAIN_ID_LEN {
            return Err(Error::msg(format!(
                "chainID is too long; got: {}, max: {MAX_CHAIN_ID_LEN}",
                self.chain_id.len()
            )));
        }
        if self.height < 0 {
            return Err(Error::msg("negative Height"));
        } else if self.height == 0 {
            return Err(Error::msg("zero Height"));
        }
        self.last_block_id
            .validate_basic()
            .map_err(|e| e.wrap("wrong LastBlockID: "))?;
        validate_hash(&self.last_commit_hash).map_err(|e| e.wrap("wrong LastCommitHash: "))?;
        validate_hash(&self.data_hash).map_err(|e| e.wrap("wrong DataHash: "))?;
        validate_hash(&self.evidence_hash).map_err(|e| e.wrap("wrong EvidenceHash: "))?;
        if self.proposer_address.len() != ADDRESS_SIZE {
            return Err(Error::msg(format!(
                "invalid ProposerAddress length; got: {}, expected: {ADDRESS_SIZE}",
                self.proposer_address.len()
            )));
        }
        validate_hash(&self.validators_hash).map_err(|e| e.wrap("wrong ValidatorsHash: "))?;
        validate_hash(&self.next_validators_hash)
            .map_err(|e| e.wrap("wrong NextValidatorsHash: "))?;
        validate_hash(&self.consensus_hash).map_err(|e| e.wrap("wrong ConsensusHash: "))?;
        validate_hash(&self.last_results_hash).map_err(|e| e.wrap("wrong LastResultsHash: "))?;
        Ok(())
    }

    /// `Header.Hash`: Merkle root of the field encodings; `None` (Go nil)
    /// when `validators_hash` is empty.
    #[must_use]
    pub fn hash(&self) -> Option<Vec<u8>> {
        if self.validators_hash.is_empty() {
            return None;
        }
        let fields: [Vec<u8>; 14] = [
            self.version.to_proto().encode_to_vec(),
            cdc_string(&self.chain_id),
            cdc_i64(self.height),
            self.time.to_proto().encode_to_vec(),
            self.last_block_id.to_proto().encode_to_vec(),
            cdc_bytes(&self.last_commit_hash),
            cdc_bytes(&self.data_hash),
            cdc_bytes(&self.validators_hash),
            cdc_bytes(&self.next_validators_hash),
            cdc_bytes(&self.consensus_hash),
            cdc_bytes(&self.app_hash),
            cdc_bytes(&self.last_results_hash),
            cdc_bytes(&self.evidence_hash),
            cdc_bytes(&self.proposer_address),
        ];
        Some(hash_from_byte_slices(&fields))
    }

    /// `Header.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Header {
        pb::Header {
            version: Some(self.version.to_proto()),
            chain_id: self.chain_id.clone(),
            height: self.height,
            time: Some(self.time.to_proto()),
            last_block_id: Some(self.last_block_id.to_proto()),
            last_commit_hash: self.last_commit_hash.clone(),
            data_hash: self.data_hash.clone(),
            validators_hash: self.validators_hash.clone(),
            next_validators_hash: self.next_validators_hash.clone(),
            consensus_hash: self.consensus_hash.clone(),
            app_hash: self.app_hash.clone(),
            last_results_hash: self.last_results_hash.clone(),
            evidence_hash: self.evidence_hash.clone(),
            proposer_address: self.proposer_address.clone(),
        }
    }

    /// Converts without `ValidateBasic` (timestamps are still range-checked,
    /// as gogoproto does while unmarshaling).
    pub fn from_proto_unvalidated(p: &pb::Header) -> Result<Self> {
        Ok(Self {
            version: Consensus::from_proto(p.version.as_ref()),
            chain_id: p.chain_id.clone(),
            height: p.height,
            time: Time::from_proto(p.time.as_ref())?,
            last_block_id: p
                .last_block_id
                .as_ref()
                .map(BlockId::from_proto_unvalidated)
                .unwrap_or_default(),
            last_commit_hash: p.last_commit_hash.clone(),
            data_hash: p.data_hash.clone(),
            validators_hash: p.validators_hash.clone(),
            next_validators_hash: p.next_validators_hash.clone(),
            consensus_hash: p.consensus_hash.clone(),
            app_hash: p.app_hash.clone(),
            last_results_hash: p.last_results_hash.clone(),
            evidence_hash: p.evidence_hash.clone(),
            proposer_address: p.proposer_address.clone(),
        })
    }

    /// `HeaderFromProto` (validates the last block id, then the header).
    pub fn from_proto(p: &pb::Header) -> Result<Self> {
        let default_id = pb::BlockId::default();
        BlockId::from_proto(p.last_block_id.as_ref().unwrap_or(&default_id))?;
        let h = Self::from_proto_unvalidated(p)?;
        h.validate_basic()?;
        Ok(h)
    }
}
