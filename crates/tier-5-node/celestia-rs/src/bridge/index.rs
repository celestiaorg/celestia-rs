//! Verified DA data, indexed in commit order independently of consensus execution.
use std::{
    path::Path,
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use kvdb::{KvBackend, RedbBackend, RedbOptions, WriteBatch};
use nodestore::{BlockStore, StateStore};
use prost::Message;

use super::{Committed, HeaderWire};

const STORAGE_WINDOW_NS: i128 = 169 * 3600 * 1_000_000_000;

pub(crate) struct Indexer {
    pub(crate) store: Arc<Store>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Indexer {
    pub(crate) fn start(
        home: &Path,
        chain: &str,
        blocks: BlockStore,
        states: StateStore,
        archival: bool,
    ) -> Result<Self> {
        let dir = home.join("data/da-bridge");
        std::fs::create_dir_all(&dir)?;
        let store = Arc::new(Store::open(&dir.join("index.redb"), chain, archival)?);
        store.check_source(&blocks)?;
        let writer = store.clone();
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("da-index".into())
            .spawn(move || {
                loop {
                    if stopped.try_recv() != Err(mpsc::TryRecvError::Empty) {
                        break;
                    }
                    match writer.advance(&blocks, &states) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "DA indexing paused at its last verified height");
                        }
                    }
                    if stopped.recv_timeout(Duration::from_millis(500))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            store,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}
impl Drop for Indexer {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct Store {
    db: RedbBackend,
    archival: bool,
    target: std::sync::atomic::AtomicI64,
    chain: String,
}

fn at(prefix: u8, height: i64) -> Vec<u8> {
    [[prefix].as_slice(), &height.to_be_bytes()].concat()
}
fn by_hash(hash: &[u8]) -> Vec<u8> {
    [b"H", hash].concat()
}

impl Store {
    fn open(path: &Path, chain: &str, archival: bool) -> Result<Self> {
        let db = RedbBackend::open(
            path,
            RedbOptions {
                sync_writes: true,
                cache_size: 32 << 20,
                ..Default::default()
            },
        )?;
        let mode = [u8::from(archival)];
        if let Some(existing) = db.get(b"chain")? {
            ensure!(
                existing == chain.as_bytes(),
                "DA index belongs to another chain"
            );
            let previous = db.get(b"archival")?.context("missing retention mode")?;
            ensure!(previous == [0] || previous == [1], "invalid retention mode");
            if previous != mode {
                ensure!(!archival, "a pruned index cannot be converted to archival");
                let mut batch = WriteBatch::new();
                batch.put(b"archival", mode);
                db.write(batch)?;
            }
        } else {
            let mut batch = WriteBatch::new();
            batch.put(b"chain", chain.as_bytes());
            batch.put(b"archival", mode);
            db.write(batch)?;
        }
        Ok(Self {
            db,
            archival,
            target: std::sync::atomic::AtomicI64::new(0),
            chain: chain.to_owned(),
        })
    }

    fn number(&self, key: &[u8]) -> Result<Option<i64>> {
        self.db
            .get(key)?
            .map(|v| Ok(i64::from_be_bytes(v.as_slice().try_into()?)))
            .transpose()
    }
    pub(super) fn target(&self) -> i64 {
        self.target.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(super) fn head(&self) -> Result<i64> {
        Ok(self.number(b"head")?.unwrap_or(0))
    }
    pub(super) fn tail(&self) -> Result<i64> {
        Ok(self.number(b"tail")?.unwrap_or(0))
    }
    pub(super) fn by_hash(&self, hash: &[u8]) -> Result<Option<i64>> {
        self.number(&by_hash(hash))
    }
    pub(super) fn header_wire(&self, height: i64) -> Result<Vec<u8>> {
        self.db
            .get(&at(b'h', height))?
            .context("DA header unavailable")
    }

    fn check_source(&self, blocks: &BlockStore) -> Result<()> {
        self.target
            .store(blocks.height(), std::sync::atomic::Ordering::Release);
        let head = self.head()?;
        if head == 0 {
            return Ok(());
        }
        ensure!(
            head <= blocks.height(),
            "DA index is ahead of the local consensus store"
        );
        let tip = self.db.get(b"tip")?.context("missing DA tip hash")?;
        if head >= blocks.base() {
            let meta = blocks
                .load_block_meta(head)?
                .context("missing consensus block at DA head")?;
            ensure!(
                meta.header.hash().as_ref() == Some(&tip),
                "DA index and consensus chain disagree"
            );
        } else {
            let next = blocks
                .load_block_meta(blocks.base())?
                .context("missing consensus base")?;
            ensure!(
                head.checked_add(1) == Some(next.header.height)
                    && next.header.last_block_id.hash == tip,
                "cannot link the DA index to the retained consensus blocks"
            );
        }
        Ok(())
    }

    fn advance(&self, blocks: &BlockStore, states: &StateStore) -> Result<bool> {
        self.target
            .store(blocks.height(), std::sync::atomic::Ordering::Release);
        let head = self.head()?;
        let next = if head == 0 {
            blocks.base().max(1)
        } else {
            head.checked_add(1).context("DA height overflow")?
        };
        if blocks.height() < next || states.load()?.is_none_or(|s| s.last_block_height < next) {
            if !self.archival && head > 0 {
                let mut batch = WriteBatch::new();
                let old = self.tail()?;
                let tail =
                    self.prune(&mut batch, old, head, light::verifier::now().unix_nanos())?;
                if tail != old {
                    batch.put(b"tail", tail.to_be_bytes());
                    self.db.write(batch)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        ensure!(
            next >= blocks.base(),
            "consensus pruned height {next} before it could be indexed"
        );
        let block = reconstruct(blocks, states, next)?;
        ensure!(block.header.chain_id == self.chain, "unexpected DA chain");
        if head != 0 {
            ensure!(
                self.db.get(b"tip")?.as_ref() == Some(&block.header.last_block_id.hash),
                "verified DA history is discontinuous"
            );
        }
        self.save(&block)?;
        Ok(true)
    }

    fn prune(&self, batch: &mut WriteBatch, mut tail: i64, h: i64, now: i128) -> Result<i64> {
        let cutoff = now - STORAGE_WINDOW_NS;
        let limit = tail.saturating_add(64).min(h);
        while !self.archival && tail < limit {
            let raw = self.header_wire(tail)?;
            let wire = HeaderWire::decode(raw.as_slice())?;
            let old = ctypes::Header::from_proto(&wire.header.context("missing indexed header")?)?;
            if old.time.unix_nanos() >= cutoff {
                break;
            }
            batch.delete(at(b'h', tail));
            batch.delete(at(b's', tail));
            batch.delete(by_hash(&old.hash().context("missing old header hash")?));
            tail += 1;
        }
        Ok(tail)
    }

    fn save(&self, block: &Committed) -> Result<()> {
        let h = block.header.height;
        let mut batch = WriteBatch::new();
        let mut tail = self.tail()?;
        if tail == 0 {
            tail = h;
        }
        tail = self.prune(&mut batch, tail, h, light::verifier::now().unix_nanos())?;
        let width = block.eds.original_width();
        let mut ods = Vec::with_capacity(width * width * shares::SHARE_SIZE);
        for row in 0..width {
            for col in 0..width {
                ods.extend_from_slice(block.eds.share(row, col)?);
            }
        }
        let hash = &block.commit.block_id.hash;
        batch.put(at(b'h', h), block.header_wire()?);
        batch.put(at(b's', h), ods);
        batch.put(by_hash(hash), h.to_be_bytes());
        batch.put(b"tip", hash.clone());
        batch.put(b"tail", tail.to_be_bytes());
        batch.put(b"head", h.to_be_bytes());
        self.db.write(batch)?;
        Ok(())
    }

    pub(super) fn block(&self, height: i64) -> Result<Committed> {
        let encoded = self.header_wire(height)?;
        let wire = HeaderWire::decode(encoded.as_slice())?;
        let header = ctypes::Header::from_proto(&wire.header.context("missing indexed header")?)?;
        let commit = ctypes::Commit::from_proto(&wire.commit.context("missing indexed commit")?)?;
        let validators = ctypes::ValidatorSet::from_proto(
            &wire.validators.context("missing indexed validators")?,
        )?;
        let dah = square::DataAvailabilityHeader::from_proto_bytes(&wire.dah)?;
        let width = dah.row_roots.len() / 2;
        let raw = self
            .db
            .get(&at(b's', height))?
            .context("original data square unavailable")?;
        ensure!(
            (1..=512).contains(&width)
                && width.is_power_of_two()
                && raw.len() == width * width * shares::SHARE_SIZE,
            "invalid stored square dimensions"
        );
        let shares = raw
            .as_chunks::<{ shares::SHARE_SIZE }>()
            .0
            .iter()
            .copied()
            .map(shares::Share::from_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        let eds = square::extend_shares(&shares)?;
        ensure!(
            square::new_data_availability_header(&eds)? == dah && header.data_hash == dah.hash(),
            "stored square does not match committed root"
        );
        Ok(Committed {
            header,
            commit,
            validators,
            eds,
            dah,
        })
    }
}

fn reconstruct(
    blocks: &nodestore::BlockStore,
    states: &nodestore::StateStore,
    height: i64,
) -> Result<Committed> {
    let block = blocks.load_block(height)?.context("block unavailable")?;
    let commit = match blocks.load_block_commit(height)? {
        Some(commit) => commit,
        None => blocks
            .load_seen_commit(height)?
            .context("commit unavailable")?,
    };
    let h = block.header;
    ensure!(
        h.height == height && h.hash().as_ref() == Some(&commit.block_id.hash),
        "block and commit disagree"
    );
    let validators = states.load_validators(height)?;
    ensure!(
        h.validators_hash == validators.hash(),
        "validator set does not match block"
    );
    ctypes::verify_commit_light(&h.chain_id, &validators, &commit.block_id, height, &commit)?;
    let txs = block
        .data
        .txs
        .into_iter()
        .map(|tx| tx.0)
        .collect::<Vec<_>>();
    let (eds, dah) = app::proposal::verified_block_eds(h.version.app, &txs, &h.data_hash)
        .map_err(anyhow::Error::msg)?;
    Ok(Committed {
        header: h,
        commit,
        validators,
        eds,
        dah,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(height: i64, seconds: i64) -> Result<Committed> {
        let eds = square::extend_shares(square::empty_square().as_shares())?;
        let dah = square::new_data_availability_header(&eds)?;
        let validators = ctypes::ValidatorSet::new(&[ctypes::Validator::new(
            ctypes::PubKey::Ed25519([0x31; 32]),
            1,
        )])?;
        let header = ctypes::Header {
            version: ctypes::Consensus { block: 11, app: 11 },
            chain_id: "controls".into(),
            height,
            time: ctypes::Time::from_unix(seconds, 0),
            validators_hash: validators.hash(),
            proposer_address: validators.validators[0].address.clone(),
            data_hash: dah.hash().to_vec(),
            ..Default::default()
        };
        let mut commit = ctypes::Commit::default();
        commit.block_id.hash = header.hash().context("empty hash")?;
        Ok(Committed {
            header,
            commit,
            validators,
            eds,
            dah,
        })
    }

    #[test]
    fn rolling_and_archival_indexes_reopen_with_consistent_height_and_hash() -> Result<()> {
        let first = block(10, 1)?;
        let next = block(11, 169 * 3600 + 2)?;
        for archival in [false, true] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("index.redb");
            let store = Store::open(&path, "controls", archival)?;
            store.save(&first)?;
            drop(store);
            let store = Store::open(&path, "controls", archival)?;
            assert_eq!(store.head()?, 10);
            assert_eq!(store.by_hash(&first.commit.block_id.hash)?, Some(10));
            store.save(&next)?;
            drop(store);
            let store = Store::open(&path, "controls", archival)?;
            assert_eq!(store.head()?, 11);
            assert_eq!(store.tail()?, if archival { 10 } else { 11 });
            assert_eq!(
                store.by_hash(&first.commit.block_id.hash)?.is_some(),
                archival
            );
            assert_eq!(store.header_wire(10).is_ok(), archival);
            assert_eq!(store.db.get(&at(b's', 10))?.is_some(), archival);
            assert_eq!(store.by_hash(&next.commit.block_id.hash)?, Some(11));
            drop(store);
            assert!(Store::open(&path, "different", archival).is_err());
            assert_eq!(Store::open(&path, "controls", !archival).is_ok(), archival);
        }
        Ok(())
    }
}
