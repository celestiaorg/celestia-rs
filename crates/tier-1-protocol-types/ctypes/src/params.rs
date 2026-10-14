//! `ConsensusParams` (`types/params.go`). celestia-core v0.42 has no
//! celestia-specific consensus params (timeouts come from the app's
//! `TimeoutInfo`); block size limits are the celestia ones (128 MiB max,
//! 64 KiB parts).

use crate::error::{Error, Result};
use crate::gofmt::duration_string;
use crate::part_set::MAX_BLOCK_SIZE_BYTES;
use crate::proto::types as pb;
use crate::pubkey::{ED25519_KEY_TYPE, SECP256K1_KEY_TYPE};
use crate::time::{duration_from_proto, duration_to_proto};
use prost::Message;

/// Block limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockParams {
    /// Max block bytes (-1 = `MaxBlockSizeBytes`).
    pub max_bytes: i64,
    /// Max gas (-1 = unlimited).
    pub max_gas: i64,
}

/// Evidence limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvidenceParams {
    /// Max age in blocks.
    pub max_age_num_blocks: i64,
    /// Max age as a Go `time.Duration` (nanoseconds).
    pub max_age_duration: i64,
    /// Max evidence bytes per block.
    pub max_bytes: i64,
}

/// Allowed validator key types (ABCI names).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidatorParams {
    /// Key type names.
    pub pub_key_types: Vec<String>,
}

/// App version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VersionParams {
    /// App version.
    pub app: u64,
}

/// ABCI params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AbciParams {
    /// First height with vote extensions (0 = disabled).
    pub vote_extensions_enable_height: i64,
}

impl AbciParams {
    /// `VoteExtensionsEnabled(h)`; Go panics for `h < 1`.
    pub fn vote_extensions_enabled(&self, h: i64) -> Result<bool> {
        if h < 1 {
            return Err(Error::msg(format!(
                "cannot check if vote extensions enabled for height {h} (< 1)"
            )));
        }
        if self.vote_extensions_enable_height == 0 {
            return Ok(false);
        }
        Ok(self.vote_extensions_enable_height <= h)
    }
}

/// Consensus-critical parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConsensusParams {
    /// Block.
    pub block: BlockParams,
    /// Evidence.
    pub evidence: EvidenceParams,
    /// Validator.
    pub validator: ValidatorParams,
    /// Version.
    pub version: VersionParams,
    /// ABCI.
    pub abci: AbciParams,
}

/// `IsValidPubkeyType`.
#[must_use]
pub fn is_valid_pubkey_type(params: &ValidatorParams, t: &str) -> bool {
    params.pub_key_types.iter().any(|p| p == t)
}

impl ConsensusParams {
    /// `DefaultConsensusParams`.
    #[must_use]
    pub fn default_params() -> Self {
        Self {
            block: BlockParams {
                max_bytes: 22_020_096,
                max_gas: -1,
            },
            evidence: EvidenceParams {
                max_age_num_blocks: 100_000,
                max_age_duration: 48 * 3600 * 1_000_000_000,
                max_bytes: 1_048_576,
            },
            validator: ValidatorParams {
                pub_key_types: vec![ED25519_KEY_TYPE.to_owned()],
            },
            version: VersionParams { app: 0 },
            abci: AbciParams {
                vote_extensions_enable_height: 0,
            },
        }
    }

    /// `ConsensusParams.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        let b = &self.block;
        if b.max_bytes == 0 {
            return Err(Error::msg("block.MaxBytes cannot be 0"));
        }
        if b.max_bytes < -1 {
            return Err(Error::msg(format!(
                "block.MaxBytes must be -1 or greater than 0. Got {}",
                b.max_bytes
            )));
        }
        if b.max_bytes > MAX_BLOCK_SIZE_BYTES {
            return Err(Error::msg(format!(
                "block.MaxBytes is too big. {} > {MAX_BLOCK_SIZE_BYTES}",
                b.max_bytes
            )));
        }
        if b.max_gas < -1 {
            return Err(Error::msg(format!(
                "block.MaxGas must be greater or equal to -1. Got {}",
                b.max_gas
            )));
        }
        let e = &self.evidence;
        if e.max_age_num_blocks <= 0 {
            return Err(Error::msg(format!(
                "evidence.MaxAgeNumBlocks must be greater than 0. Got {}",
                e.max_age_num_blocks
            )));
        }
        if e.max_age_duration <= 0 {
            return Err(Error::msg(format!(
                "evidence.MaxAgeDuration must be grater than 0 if provided, Got {}",
                duration_string(e.max_age_duration)
            )));
        }
        let max_bytes = if b.max_bytes == -1 {
            MAX_BLOCK_SIZE_BYTES
        } else {
            b.max_bytes
        };
        if e.max_bytes > max_bytes {
            return Err(Error::msg(format!(
                "evidence.MaxBytesEvidence is greater than upper bound, {} > {}",
                e.max_bytes, b.max_bytes
            )));
        }
        if e.max_bytes < 0 {
            return Err(Error::msg(format!(
                "evidence.MaxBytes must be non negative. Got: {}",
                e.max_bytes
            )));
        }
        if self.abci.vote_extensions_enable_height < 0 {
            return Err(Error::msg(format!(
                "ABCI.VoteExtensionsEnableHeight cannot be negative. Got: {}",
                self.abci.vote_extensions_enable_height
            )));
        }
        if self.validator.pub_key_types.is_empty() {
            return Err(Error::msg(
                "len(Validator.PubKeyTypes) must be greater than 0",
            ));
        }
        for (i, kt) in self.validator.pub_key_types.iter().enumerate() {
            if kt != ED25519_KEY_TYPE && kt != SECP256K1_KEY_TYPE {
                return Err(Error::msg(format!(
                    "params.Validator.PubKeyTypes[{i}], {kt}, is an unknown pubkey type"
                )));
            }
        }
        Ok(())
    }

    /// `ConsensusParams.ValidateUpdate(updated, h)` (vote extension enable
    /// height rules).
    pub fn validate_update(&self, updated: Option<&pb::ConsensusParams>, h: i64) -> Result<()> {
        let Some(upd) = updated.and_then(|u| u.abci.as_ref()) else {
            return Ok(());
        };
        let new = upd.vote_extensions_enable_height;
        let cur = self.abci.vote_extensions_enable_height;
        if new < 0 {
            return Err(Error::msg("VoteExtensionsEnableHeight must be positive"));
        }
        if cur <= 0 && new == 0 {
            return Ok(());
        }
        if cur == new {
            return Ok(());
        }
        if cur > 0 && new == 0 {
            if cur <= h {
                return Err(Error::msg(format!(
                    "vote extensions cannot be disabled once enabledold enable height: {cur}, current height {h}"
                )));
            }
            return Ok(());
        }
        if new <= h {
            return Err(Error::msg(format!(
                "vote extensions cannot be updated to a past or current height, enable height: {new}, current height {h}"
            )));
        }
        if cur <= 0 {
            return Ok(());
        }
        if cur <= h {
            return Err(Error::msg(format!(
                "vote extensions cannot be modified once enabledenable height: {cur}, current height {h}"
            )));
        }
        Ok(())
    }

    /// `ConsensusParams.Hash`: SHA-256 of `HashedParams{max_bytes, max_gas}`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        let hp = pb::HashedParams {
            block_max_bytes: self.block.max_bytes,
            block_max_gas: self.block.max_gas,
        };
        hash::sha256(&hp.encode_to_vec()).to_vec()
    }

    /// `ConsensusParams.Update(params2)`: copies the non-nil sections.
    /// Errors only when an evidence duration does not decode.
    pub fn update(&self, p2: Option<&pb::ConsensusParams>) -> Result<Self> {
        let mut res = self.clone();
        let Some(p2) = p2 else { return Ok(res) };
        if let Some(b) = &p2.block {
            res.block.max_bytes = b.max_bytes;
            res.block.max_gas = b.max_gas;
        }
        if let Some(e) = &p2.evidence {
            res.evidence.max_age_num_blocks = e.max_age_num_blocks;
            res.evidence.max_age_duration = duration_from_proto(e.max_age_duration.as_ref())?;
            res.evidence.max_bytes = e.max_bytes;
        }
        if let Some(v) = &p2.validator {
            res.validator.pub_key_types.clone_from(&v.pub_key_types);
        }
        if let Some(v) = &p2.version {
            res.version.app = v.app;
        }
        if let Some(a) = &p2.abci {
            res.abci.vote_extensions_enable_height = a.vote_extensions_enable_height;
        }
        Ok(res)
    }

    /// `ConsensusParams.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::ConsensusParams {
        pb::ConsensusParams {
            block: Some(pb::BlockParams {
                max_bytes: self.block.max_bytes,
                max_gas: self.block.max_gas,
            }),
            evidence: Some(pb::EvidenceParams {
                max_age_num_blocks: self.evidence.max_age_num_blocks,
                max_age_duration: Some(duration_to_proto(self.evidence.max_age_duration)),
                max_bytes: self.evidence.max_bytes,
            }),
            validator: Some(pb::ValidatorParams {
                pub_key_types: self.validator.pub_key_types.clone(),
            }),
            version: Some(pb::VersionParams {
                app: self.version.app,
            }),
            abci: Some(pb::AbciParams {
                vote_extensions_enable_height: self.abci.vote_extensions_enable_height,
            }),
        }
    }

    /// `ConsensusParamsFromProto`. Go dereferences the block, evidence,
    /// validator and version sections unconditionally (panicking when nil);
    /// absent sections are treated as zero here.
    pub fn from_proto(p: &pb::ConsensusParams) -> Result<Self> {
        let b = p.block.unwrap_or_default();
        let e = p.evidence.unwrap_or_default();
        Ok(Self {
            block: BlockParams {
                max_bytes: b.max_bytes,
                max_gas: b.max_gas,
            },
            evidence: EvidenceParams {
                max_age_num_blocks: e.max_age_num_blocks,
                max_age_duration: duration_from_proto(e.max_age_duration.as_ref())?,
                max_bytes: e.max_bytes,
            },
            validator: ValidatorParams {
                pub_key_types: p
                    .validator
                    .as_ref()
                    .map(|v| v.pub_key_types.clone())
                    .unwrap_or_default(),
            },
            version: VersionParams {
                app: p.version.as_ref().map_or(0, |v| v.app),
            },
            abci: AbciParams {
                vote_extensions_enable_height: p
                    .abci
                    .as_ref()
                    .map_or(0, |a| a.vote_extensions_enable_height),
            },
        })
    }
}
