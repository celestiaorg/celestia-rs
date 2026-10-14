//! `SignedHeader` and `LightBlock` (`types/light.go`).

use crate::commit::Commit;
use crate::error::{Error, Result};
use crate::gofmt::{hex_upper, quote};
use crate::header::Header;
use crate::proto::types as pb;
use crate::validator_set::ValidatorSet;

/// A header with the commit that signs it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SignedHeader {
    /// Header (Go pointer, may be nil).
    pub header: Option<Header>,
    /// Commit (Go pointer, may be nil).
    pub commit: Option<Commit>,
}

impl SignedHeader {
    /// `SignedHeader.IsEmpty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.header.is_none() && self.commit.is_none()
    }

    /// `SignedHeader.ValidateBasic(chainID)`.
    pub fn validate_basic(&self, chain_id: &str) -> Result<()> {
        let Some(header) = &self.header else {
            return Err(Error::msg("missing header"));
        };
        let Some(commit) = &self.commit else {
            return Err(Error::msg("missing commit"));
        };
        header
            .validate_basic()
            .map_err(|e| e.wrap("invalid header: "))?;
        commit
            .validate_basic()
            .map_err(|e| e.wrap("invalid commit: "))?;
        if header.chain_id != chain_id {
            return Err(Error::msg(format!(
                "header belongs to another chain {}, not {}",
                quote(&header.chain_id),
                quote(chain_id)
            )));
        }
        if commit.height != header.height {
            return Err(Error::msg(format!(
                "header and commit height mismatch: {} vs {}",
                header.height, commit.height
            )));
        }
        let hhash = header.hash().unwrap_or_default();
        if hhash != commit.block_id.hash {
            return Err(Error::msg(format!(
                "commit signs block {}, header is block {}",
                hex_upper(&commit.block_id.hash),
                hex_upper(&hhash)
            )));
        }
        Ok(())
    }

    /// `SignedHeader.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::SignedHeader {
        pb::SignedHeader {
            header: self.header.as_ref().map(Header::to_proto),
            commit: self.commit.as_ref().map(Commit::to_proto),
        }
    }

    /// `SignedHeaderFromProto`.
    pub fn from_proto(p: &pb::SignedHeader) -> Result<Self> {
        Ok(Self {
            header: p.header.as_ref().map(Header::from_proto).transpose()?,
            commit: p.commit.as_ref().map(Commit::from_proto).transpose()?,
        })
    }
}

/// A signed header plus the validator set that signed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LightBlock {
    /// Signed header (Go pointer, may be nil).
    pub signed_header: Option<SignedHeader>,
    /// Validator set (Go pointer, may be nil).
    pub validator_set: Option<ValidatorSet>,
}

impl LightBlock {
    /// The header, if present.
    #[must_use]
    pub fn header(&self) -> Option<&Header> {
        self.signed_header.as_ref()?.header.as_ref()
    }

    /// `LightBlock.ValidateBasic(chainID)`.
    pub fn validate_basic(&self, chain_id: &str) -> Result<()> {
        let Some(sh) = &self.signed_header else {
            return Err(Error::msg("missing signed header"));
        };
        let Some(vals) = &self.validator_set else {
            return Err(Error::msg("missing validator set"));
        };
        sh.validate_basic(chain_id)
            .map_err(|e| e.wrap("invalid signed header: "))?;
        vals.validate_basic()
            .map_err(|e| e.wrap("invalid validator set: "))?;
        let vh = vals.hash();
        let header_vh = sh
            .header
            .as_ref()
            .map(|h| h.validators_hash.clone())
            .unwrap_or_default();
        if header_vh != vh {
            return Err(Error::msg(format!(
                "expected validator hash of header to match validator set hash ({} != {})",
                hex_upper(&header_vh),
                hex_upper(&vh)
            )));
        }
        Ok(())
    }

    /// `LightBlock.ToProto`.
    pub fn to_proto(&self) -> Result<pb::LightBlock> {
        Ok(pb::LightBlock {
            signed_header: self.signed_header.as_ref().map(SignedHeader::to_proto),
            validator_set: self
                .validator_set
                .as_ref()
                .map(ValidatorSet::to_proto)
                .transpose()?,
        })
    }

    /// `LightBlockFromProto`.
    pub fn from_proto(p: &pb::LightBlock) -> Result<Self> {
        Ok(Self {
            signed_header: p
                .signed_header
                .as_ref()
                .map(SignedHeader::from_proto)
                .transpose()?,
            validator_set: p
                .validator_set
                .as_ref()
                .map(ValidatorSet::from_proto)
                .transpose()?,
        })
    }
}
