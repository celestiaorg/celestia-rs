//! `BlockMeta` (`types/block_meta.go`).

use crate::block::Block;
use crate::block_id::BlockId;
use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::header::Header;
use crate::part_set::PartSetHeader;
use crate::proto::types as pb;

/// Block metadata stored by the block store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockMeta {
    /// Block id.
    pub block_id: BlockId,
    /// Encoded block size.
    pub block_size: i64,
    /// Header.
    pub header: Header,
    /// Number of txs.
    pub num_txs: i64,
}

impl BlockMeta {
    /// `NewBlockMeta(block, blockParts)`.
    pub fn new(block: &mut Block, parts_header: PartSetHeader) -> Self {
        let hash = block.hash().unwrap_or_default();
        Self {
            block_id: BlockId {
                hash,
                part_set_header: parts_header,
            },
            block_size: i64::try_from(block.size()).unwrap_or(i64::MAX),
            header: block.header.clone(),
            num_txs: i64::try_from(block.data.txs.len()).unwrap_or(i64::MAX),
        }
    }

    /// `BlockMeta.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::BlockMeta {
        pb::BlockMeta {
            block_id: Some(self.block_id.to_proto()),
            block_size: self.block_size,
            header: Some(self.header.to_proto()),
            num_txs: self.num_txs,
        }
    }

    /// `BlockMetaFromTrustedProto`.
    pub fn from_trusted_proto(p: &pb::BlockMeta) -> Result<Self> {
        let block_id = BlockId::from_proto(&p.block_id.clone().unwrap_or_default())?;
        let header = Header::from_proto(&p.header.clone().unwrap_or_default())?;
        Ok(Self {
            block_id,
            block_size: p.block_size,
            header,
            num_txs: p.num_txs,
        })
    }

    /// `BlockMetaFromProto` (validates).
    pub fn from_proto(p: &pb::BlockMeta) -> Result<Self> {
        let bm = Self::from_trusted_proto(p)?;
        bm.validate_basic()?;
        Ok(bm)
    }

    /// `BlockMeta.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        self.block_id.validate_basic()?;
        let hh = self.header.hash().unwrap_or_default();
        if self.block_id.hash != hh {
            return Err(Error::msg(format!(
                "expected BlockID#Hash and Header#Hash to be the same, got {} != {}",
                hex_upper(&self.block_id.hash),
                hex_upper(&hh)
            )));
        }
        Ok(())
    }
}
