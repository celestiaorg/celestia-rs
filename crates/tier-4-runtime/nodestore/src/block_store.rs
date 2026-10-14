//! `store.BlockStore` (`store/store.go`) on a [`KvBackend`].
//!
//! Keys follow Go's layout (`H:<h>` block meta, `P:<h>:<i>` parts,
//! `C:<h>` block commit (the `LastCommit` of block `h+1`), `SC:<h>` seen
//! commit, `EC:<h>` extended commit, `BH:<hash>` height by hash,
//! `blockStore` base/height). Values are the same protobufs. Every save is
//! one atomic [`KvBackend::write`] that also carries the new base/height.

use std::sync::{Arc, Mutex};

use ctypes::proto::{store as pbstore, types as pb};
use ctypes::{Block, BlockMeta, Commit, ExtendedCommit, Part, PartSet};
use kvdb::{KvBackend, WriteBatch};
use prost::Message;

use crate::state::{State, sub_times};
use crate::{Error, Result, ctypes_err, kv_err};

const BLOCK_STORE_KEY: &[u8] = b"blockStore";

fn meta_key(h: i64) -> Vec<u8> {
    format!("H:{h}").into_bytes()
}
fn part_key(h: i64, i: usize) -> Vec<u8> {
    format!("P:{h}:{i}").into_bytes()
}
fn commit_key(h: i64) -> Vec<u8> {
    format!("C:{h}").into_bytes()
}
fn seen_commit_key(h: i64) -> Vec<u8> {
    format!("SC:{h}").into_bytes()
}
fn ext_commit_key(h: i64) -> Vec<u8> {
    format!("EC:{h}").into_bytes()
}
fn hash_key(hash: &[u8]) -> Vec<u8> {
    format!("BH:{}", hex::encode(hash)).into_bytes()
}
/// `calcTxHashKey` (celestia-core's committed-tx info, `TH:<hex>`).
fn tx_hash_key(hash: &[u8]) -> Vec<u8> {
    format!("TH:{}", hex::encode(hash)).into_bytes()
}

#[derive(Debug, Default, Clone, Copy)]
struct BaseHeight {
    base: i64,
    height: i64,
}

/// Block store handle (cheap to clone; clones share the database and the
/// cached base/height).
#[derive(Debug, Clone)]
pub struct BlockStore {
    db: Arc<dyn KvBackend>,
    bh: Arc<Mutex<BaseHeight>>,
}

fn decode<M: Message + Default>(bz: &[u8], what: &str) -> Result<M> {
    M::decode(bz).map_err(|e| Error::new(format!("{what}: data has been corrupted: {e}")))
}

/// `evidence.IsEvidenceExpired`.
#[must_use]
pub fn is_evidence_expired(
    height_now: i64,
    time_now: ctypes::Time,
    ev_height: i64,
    ev_time: ctypes::Time,
    params: &ctypes::params::EvidenceParams,
) -> bool {
    let age_duration = sub_times(time_now, ev_time);
    let age_blocks = height_now - ev_height;
    age_blocks > params.max_age_num_blocks && age_duration > params.max_age_duration
}

impl BlockStore {
    /// `NewBlockStore`.
    pub fn new(db: Arc<dyn KvBackend>) -> Result<Self> {
        let bh = match db.get(BLOCK_STORE_KEY).map_err(kv_err)? {
            Some(bz) if !bz.is_empty() => {
                let s: pbstore::BlockStoreState = decode(&bz, "block store state")?;
                BaseHeight {
                    base: s.base,
                    height: s.height,
                }
            }
            _ => BaseHeight::default(),
        };
        Ok(Self {
            db,
            bh: Arc::new(Mutex::new(bh)),
        })
    }

    /// The underlying database.
    #[must_use]
    pub fn db(&self) -> Arc<dyn KvBackend> {
        self.db.clone()
    }

    fn bh(&self) -> BaseHeight {
        self.bh.lock().map(|g| *g).unwrap_or_default()
    }

    /// `Base`: first stored height (0 when empty).
    #[must_use]
    pub fn base(&self) -> i64 {
        self.bh().base
    }

    /// `Height`: last stored height (0 when empty).
    #[must_use]
    pub fn height(&self) -> i64 {
        self.bh().height
    }

    /// `Size`.
    #[must_use]
    pub fn size(&self) -> i64 {
        let bh = self.bh();
        if bh.height == 0 {
            0
        } else {
            bh.height - bh.base + 1
        }
    }

    /// `IsEmpty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let bh = self.bh();
        bh.base == 0 && bh.height == 0
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.db.get(key).map_err(kv_err)
    }

    /// `LoadBlockMeta`.
    pub fn load_block_meta(&self, height: i64) -> Result<Option<BlockMeta>> {
        let Some(bz) = self.get(&meta_key(height))? else {
            return Ok(None);
        };
        let p: pb::BlockMeta = decode(&bz, "block meta")?;
        BlockMeta::from_proto(&p).map(Some).map_err(ctypes_err)
    }

    /// `LoadBaseMeta`.
    pub fn load_base_meta(&self) -> Result<Option<BlockMeta>> {
        let base = self.base();
        if base == 0 {
            return Ok(None);
        }
        self.load_block_meta(base)
    }

    /// `LoadBlockPart`.
    pub fn load_block_part(&self, height: i64, index: usize) -> Result<Option<Part>> {
        let Some(bz) = self.get(&part_key(height, index))? else {
            return Ok(None);
        };
        let p: pb::Part = decode(&bz, "block part")?;
        Part::from_proto(&p).map(Some).map_err(ctypes_err)
    }

    /// `LoadPartSet`.
    pub fn load_part_set(&self, height: i64) -> Result<Option<(PartSet, BlockMeta)>> {
        let Some(meta) = self.load_block_meta(height)? else {
            return Ok(None);
        };
        let mut ps = PartSet::from_header(
            &meta.block_id.part_set_header,
            ctypes::BLOCK_PART_SIZE_BYTES,
        );
        for i in 0..meta.block_id.part_set_header.total as usize {
            let Some(part) = self.load_block_part(height, i)? else {
                return Ok(None);
            };
            ps.add_part(&part).map_err(ctypes_err)?;
        }
        Ok(Some((ps, meta)))
    }

    /// `LoadBlock`.
    pub fn load_block(&self, height: i64) -> Result<Option<Block>> {
        let Some(meta) = self.load_block_meta(height)? else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        for i in 0..meta.block_id.part_set_header.total as usize {
            let Some(part) = self.load_block_part(height, i)? else {
                return Ok(None);
            };
            buf.extend_from_slice(&part.bytes);
        }
        Block::decode(&buf).map(Some).map_err(ctypes_err)
    }

    /// `LoadBlockMetaByHash`.
    pub fn load_block_meta_by_hash(&self, hash: &[u8]) -> Result<Option<BlockMeta>> {
        let Some(bz) = self.get(&hash_key(hash))? else {
            return Ok(None);
        };
        let h: i64 = String::from_utf8_lossy(&bz)
            .parse()
            .map_err(|e| Error::new(format!("failed to extract height from {bz:?}: {e}")))?;
        self.load_block_meta(h)
    }

    /// `LoadBlockByHash`.
    pub fn load_block_by_hash(&self, hash: &[u8]) -> Result<Option<Block>> {
        match self.load_block_meta_by_hash(hash)? {
            Some(m) => self.load_block(m.header.height),
            None => Ok(None),
        }
    }

    fn load_commit(&self, key: &[u8]) -> Result<Option<Commit>> {
        let Some(bz) = self.get(key)? else {
            return Ok(None);
        };
        let p: pb::Commit = decode(&bz, "commit")?;
        Commit::from_proto(&p).map(Some).map_err(ctypes_err)
    }

    /// `LoadBlockCommit`: the commit for `height` included in block
    /// `height+1`.
    pub fn load_block_commit(&self, height: i64) -> Result<Option<Commit>> {
        self.load_commit(&commit_key(height))
    }

    /// `LoadSeenCommit`: the locally seen +2/3 precommits for `height`.
    pub fn load_seen_commit(&self, height: i64) -> Result<Option<Commit>> {
        self.load_commit(&seen_commit_key(height))
    }

    /// `SaveTxInfo`: per-tx execution info of a committed block (keyed by
    /// the celestia tx hash), read by RPC `tx_status`. Only failed txs keep
    /// their log.
    pub fn save_tx_info(
        &self,
        block: &Block,
        results: &[ctypes::proto::abci::ExecTxResult],
    ) -> Result<()> {
        if results.len() != block.data.txs.len() {
            return Err(Error::new(
                "tx execution results length mismatch with block txs length",
            ));
        }
        let mut batch = WriteBatch::new();
        for (i, (tx, r)) in block.data.txs.iter().zip(results).enumerate() {
            let info = pbstore::TxInfo {
                height: block.header.height,
                index: u32::try_from(i).unwrap_or(u32::MAX),
                code: r.code,
                error: if r.code == 0 {
                    String::new()
                } else {
                    r.log.clone()
                },
                codespace: r.codespace.clone(),
                gas_wanted: r.gas_wanted,
                gas_used: r.gas_used,
                signers: r.signers.clone(),
            };
            batch.put(tx_hash_key(&tx.hash()), info.encode_to_vec());
        }
        self.db.write(batch).map_err(kv_err)
    }

    /// `LoadTxInfo`.
    pub fn load_tx_info(&self, tx_hash: &[u8]) -> Result<Option<pbstore::TxInfo>> {
        match self.get(&tx_hash_key(tx_hash))? {
            Some(bz) if !bz.is_empty() => decode(&bz, "tx info").map(Some),
            _ => Ok(None),
        }
    }

    /// `LoadBlockExtendedCommit`.
    pub fn load_block_extended_commit(&self, height: i64) -> Result<Option<ExtendedCommit>> {
        let Some(bz) = self.get(&ext_commit_key(height))? else {
            return Ok(None);
        };
        let p: pb::ExtendedCommit = decode(&bz, "extended commit")?;
        ExtendedCommit::from_proto(&p).map(Some).map_err(ctypes_err)
    }

    fn save_block_to_batch(
        &self,
        block: &mut Block,
        parts: &PartSet,
        seen_commit: &Commit,
        batch: &mut WriteBatch,
    ) -> Result<()> {
        let height = block.header.height;
        let hash = block.hash().unwrap_or_default();
        let bh = self.bh();
        if bh.base > 0 && height != bh.height + 1 {
            return Err(Error::new(format!(
                "BlockStore can only save contiguous blocks. Wanted {}, got {height}",
                bh.height + 1
            )));
        }
        if !parts.is_complete() {
            return Err(Error::new(
                "BlockStore can only save complete block part sets",
            ));
        }
        if height != seen_commit.height {
            return Err(Error::new(format!(
                "BlockStore cannot save seen commit of a different height (block: {height}, commit: {})",
                seen_commit.height
            )));
        }
        for i in 0..parts.total() as usize {
            let part = parts
                .get_part(i)
                .ok_or_else(|| Error::new(format!("missing block part {i}")))?;
            batch.put(part_key(height, i), part.to_proto().encode_to_vec());
        }
        let meta = BlockMeta::new(block, parts.header());
        batch.put(meta_key(height), meta.to_proto().encode_to_vec());
        batch.put(hash_key(&hash), height.to_string().into_bytes());
        let lc = block
            .last_commit
            .as_ref()
            .ok_or_else(|| Error::new("block without LastCommit"))?;
        batch.put(commit_key(height - 1), lc.to_proto().encode_to_vec());
        batch.put(
            seen_commit_key(height),
            seen_commit.to_proto().encode_to_vec(),
        );
        Ok(())
    }

    fn write_with_state(&self, mut batch: WriteBatch, bh: BaseHeight) -> Result<()> {
        batch.put(
            BLOCK_STORE_KEY.to_vec(),
            pbstore::BlockStoreState {
                base: bh.base,
                height: bh.height,
            }
            .encode_to_vec(),
        );
        self.db.write(batch).map_err(kv_err)?;
        if let Ok(mut g) = self.bh.lock() {
            *g = bh;
        }
        Ok(())
    }

    /// `SaveBlock`.
    pub fn save_block(
        &self,
        block: &mut Block,
        parts: &PartSet,
        seen_commit: &Commit,
    ) -> Result<()> {
        let mut batch = WriteBatch::new();
        self.save_block_to_batch(block, parts, seen_commit, &mut batch)?;
        let mut bh = self.bh();
        bh.height = block.header.height;
        if bh.base == 0 {
            bh.base = block.header.height;
        }
        self.write_with_state(batch, bh)
    }

    /// `SaveBlockWithExtendedCommit`.
    pub fn save_block_with_extended_commit(
        &self,
        block: &mut Block,
        parts: &PartSet,
        seen_ext_commit: &ExtendedCommit,
    ) -> Result<()> {
        seen_ext_commit
            .ensure_extensions(true)
            .map_err(|e| Error::new(format!("problems saving block with extensions: {e}")))?;
        let mut batch = WriteBatch::new();
        self.save_block_to_batch(block, parts, &seen_ext_commit.to_commit(), &mut batch)?;
        let height = block.header.height;
        batch.put(
            ext_commit_key(height),
            seen_ext_commit.to_proto().encode_to_vec(),
        );
        let mut bh = self.bh();
        bh.height = height;
        if bh.base == 0 {
            bh.base = height;
        }
        self.write_with_state(batch, bh)
    }

    /// `SaveSeenCommit` (state sync bootstrap).
    pub fn save_seen_commit(&self, height: i64, seen_commit: &Commit) -> Result<()> {
        let mut batch = WriteBatch::new();
        batch.put(
            seen_commit_key(height),
            seen_commit.to_proto().encode_to_vec(),
        );
        self.db.write(batch).map_err(kv_err)
    }

    /// `PruneBlocks`: removes blocks below `height`, keeping headers and
    /// commits still inside the evidence window. Returns the number pruned
    /// and the evidence retain point.
    pub fn prune_blocks(&self, height: i64, state: &State) -> Result<(u64, i64)> {
        if height <= 0 {
            return Err(Error::new("height must be greater than 0"));
        }
        let bh = self.bh();
        if height > bh.height {
            return Err(Error::new(format!(
                "cannot prune beyond the latest height {}",
                bh.height
            )));
        }
        if height < bh.base {
            return Err(Error::new(format!(
                "cannot prune to height {height}, it is lower than base height {}",
                bh.base
            )));
        }
        let mut pruned = 0u64;
        let mut batch = WriteBatch::new();
        let mut evidence_point = height;
        for h in bh.base..height {
            let Some(meta) = self.load_block_meta(h)? else {
                continue;
            };
            if let Ok(Some(block)) = self.load_block(h) {
                for tx in &block.data.txs {
                    batch.delete(tx_hash_key(&tx.hash()));
                }
            }
            if evidence_point == height
                && !is_evidence_expired(
                    state.last_block_height,
                    state.last_block_time,
                    h,
                    meta.header.time,
                    &state.consensus_params.evidence,
                )
            {
                evidence_point = h;
            }
            if h < evidence_point {
                batch.delete(meta_key(h));
                batch.delete(commit_key(h));
                batch.delete(ext_commit_key(h));
            }
            batch.delete(hash_key(&meta.block_id.hash));
            batch.delete(seen_commit_key(h));
            for p in 0..meta.block_id.part_set_header.total as usize {
                batch.delete(part_key(h, p));
            }
            pruned += 1;
            if pruned % 1000 == 0 {
                let b = std::mem::take(&mut batch);
                self.write_with_state(
                    b,
                    BaseHeight {
                        base: h,
                        height: bh.height,
                    },
                )?;
            }
        }
        self.write_with_state(
            batch,
            BaseHeight {
                base: height,
                height: bh.height,
            },
        )?;
        Ok((pruned, evidence_point))
    }

    /// `DeleteLatestBlock` (rollback).
    pub fn delete_latest_block(&self) -> Result<()> {
        let bh = self.bh();
        let target = bh.height;
        let mut batch = WriteBatch::new();
        if let Some(meta) = self.load_block_meta(target)? {
            batch.delete(hash_key(&meta.block_id.hash));
            for p in 0..meta.block_id.part_set_header.total as usize {
                batch.delete(part_key(target, p));
            }
        }
        batch.delete(meta_key(target));
        batch.delete(seen_commit_key(target));
        batch.delete(ext_commit_key(target));
        batch.delete(commit_key(target - 1));
        let mut nbh = bh;
        nbh.height = target - 1;
        if nbh.height < nbh.base {
            nbh = BaseHeight::default();
        }
        self.write_with_state(batch, nbh)
    }
}
