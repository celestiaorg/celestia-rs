//! Verified DA data, indexed in commit order independently of consensus execution.
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use kvdb::{KvBackend, RedbBackend, RedbOptions, WriteBatch};
use nodestore::{BlockStore, StateStore};
use prost::Message;

use super::{Committed, HeaderWire};

const HOUR_NS: i128 = 3600 * 1_000_000_000;
const DEFAULT_RETENTION_HOURS: u64 = 169;
const BATCH_SIZE: i64 = 64;
const ARCHIVE_BATCH_BYTES: usize = 16 << 20;

#[derive(Default)]
struct PruneProgress {
    requested: AtomicI64,
    safe: AtomicI64,
}

pub(crate) struct Indexer {
    pub(crate) store: Arc<Store>,
    pruning: Arc<PruneProgress>,
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
        retention_hours: Option<u64>,
    ) -> Result<Self> {
        let dir = home.join("data/da-bridge");
        std::fs::create_dir_all(&dir)?;
        let window = (!archival)
            .then(|| i128::from(retention_hours.unwrap_or(DEFAULT_RETENTION_HOURS)) * HOUR_NS);
        let store = Arc::new(Store::open_index(&dir, chain, window, blocks.clone())?);
        store.check_source(&blocks)?;
        let pruning = Arc::new(PruneProgress::default());
        let archive = pruning.clone();
        let writer = store.clone();
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("da-index".into())
            .spawn(move || {
                loop {
                    if stopped.try_recv() != Err(mpsc::TryRecvError::Empty) {
                        break;
                    }
                    match writer.advance(&blocks, &states, &archive) {
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
            pruning,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(crate) fn prune_limit(&self) -> impl Fn(i64) -> i64 + Send + Sync + 'static {
        let progress = self.pruning.clone();
        move |requested| {
            progress.requested.fetch_max(requested, Ordering::Release);
            progress.safe.load(Ordering::Acquire)
        }
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
    window: Option<i128>,
    blocks: BlockStore,
    target: AtomicI64,
    chain: String,
}

pub(super) struct IndexedHeader {
    pub(super) header: ctypes::Header,
    pub(super) commit: ctypes::Commit,
    validators: ctypes::ValidatorSet,
    dah: square::DataAvailabilityHeader,
}

impl IndexedHeader {
    fn decode(raw: &[u8]) -> Result<Self> {
        let wire = HeaderWire::decode(raw)?;
        Ok(Self {
            header: ctypes::Header::from_proto(&wire.header.context("missing indexed header")?)?,
            commit: ctypes::Commit::from_proto(&wire.commit.context("missing indexed commit")?)?,
            validators: ctypes::ValidatorSet::from_proto(
                &wire.validators.context("missing indexed validators")?,
            )?,
            dah: square::DataAvailabilityHeader::from_proto_bytes(&wire.dah)?,
        })
    }

    pub(super) fn json(&self) -> rpc::json::J {
        super::header_json(&self.header, &self.commit, &self.validators, &self.dah)
    }
}

fn at(prefix: u8, height: i64) -> Vec<u8> {
    [[prefix].as_slice(), &height.to_be_bytes()].concat()
}
fn by_hash(hash: &[u8]) -> Vec<u8> {
    [b"H", hash].concat()
}

impl Store {
    fn open(path: &Path, chain: &str, window: Option<i128>, blocks: BlockStore) -> Result<Self> {
        let db = RedbBackend::open(
            path,
            RedbOptions {
                sync_writes: true,
                cache_size: 32 << 20,
                ..Default::default()
            },
        )?;
        let mode = [u8::from(window.is_none())];
        if let Some(existing) = db.get(b"chain")? {
            ensure!(
                existing == chain.as_bytes(),
                "DA index belongs to another chain"
            );
            let previous = db.get(b"archival")?.context("missing retention mode")?;
            ensure!(previous == [0] || previous == [1], "invalid retention mode");
            if previous != mode {
                ensure!(
                    window.is_some(),
                    "a pruned index cannot be converted to archival"
                );
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
            window,
            blocks,
            target: AtomicI64::new(0),
            chain: chain.to_owned(),
        })
    }

    fn open_index(
        dir: &Path,
        chain: &str,
        window: Option<i128>,
        blocks: BlockStore,
    ) -> Result<Self> {
        let path = dir.join("index-v2.redb");
        let legacy = dir.join("index.redb");
        if !path.exists() && legacy.exists() {
            let original = RedbBackend::open(legacy, RedbOptions::default())?;
            let temp = tempfile::NamedTempFile::new_in(dir)?;
            let next = Self::open(temp.path(), chain, window, blocks.clone())?;
            next.import_legacy(&original)?;
            next.check_source(&blocks)?;
            drop(next);
            temp.persist_noclobber(&path)?;
            std::fs::File::open(dir)?.sync_all()?;
        }
        Self::open(&path, chain, window, blocks)
    }

    fn import_legacy(&self, original: &RedbBackend) -> Result<()> {
        ensure!(
            original.get(b"chain")?.as_deref() == Some(self.chain.as_bytes()),
            "DA index belongs to another chain"
        );
        ensure!(
            self.window.is_some() || original.get(b"archival")?.as_deref() == Some(&[1]),
            "a pruned index cannot be converted to archival"
        );
        let Some(head) = original.get(b"head")? else {
            return Ok(());
        };
        let head = i64::from_be_bytes(head.as_slice().try_into()?);
        let mut height = i64::from_be_bytes(
            original
                .get(b"tail")?
                .context("missing legacy DA tail")?
                .as_slice()
                .try_into()?,
        );
        ensure!(
            height > 0 && height <= head,
            "invalid legacy DA height range"
        );
        let base = self.blocks.base();
        let cutoff = self.cutoff();
        let mut tail = None;
        while height <= head {
            let mut batch = WriteBatch::new();
            let end = height.saturating_add(BATCH_SIZE - 1).min(head);
            for h in height..=end {
                let raw = original
                    .get(&at(b'h', h))?
                    .context("missing legacy DA header")?;
                let header = decode_header(&raw)?;
                if tail.is_none() && h != head && header.time.unix_nanos() < cutoff {
                    continue;
                }
                let hash = header.hash().context("missing legacy DA header hash")?;
                batch.put(at(b'h', h), raw);
                batch.put(by_hash(&hash), h.to_be_bytes());
                if h < base {
                    let ods = original
                        .get(&at(b's', h))?
                        .context("missing legacy DA square")?;
                    batch.put(at(b's', h), ods);
                }
                if tail.is_none() {
                    batch.put(b"tail", h.to_be_bytes());
                    tail = Some(h);
                }
                batch.put(b"head", h.to_be_bytes());
                batch.put(b"tip", hash);
            }
            if !batch.is_empty() {
                self.db.write(batch)?;
            }
            if end == head {
                break;
            }
            height = end + 1;
        }
        Ok(())
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

    fn advance(
        &self,
        blocks: &BlockStore,
        states: &StateStore,
        pruning: &PruneProgress,
    ) -> Result<bool> {
        self.target
            .store(blocks.height(), std::sync::atomic::Ordering::Release);
        let head = self.head()?;
        if self.archive_pending(pruning, head)? {
            return Ok(true);
        }
        let next = if head == 0 {
            blocks.base().max(1)
        } else {
            head.checked_add(1).context("DA height overflow")?
        };
        if blocks.height() < next || states.load()?.is_none_or(|s| s.last_block_height < next) {
            if self.window.is_some() && head > 0 {
                let mut batch = WriteBatch::new();
                let old = self.tail()?;
                let tail = self.prune(&mut batch, old, head)?;
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

    /// Persist the data still needed after the requested consensus prune. The
    /// executor only reads the watermark, so it never waits on DA writes.
    fn archive_pending(&self, progress: &PruneProgress, head: i64) -> Result<bool> {
        let start = progress
            .safe
            .load(Ordering::Relaxed)
            .max(self.blocks.base())
            .max(1);
        let limit = progress
            .requested
            .load(Ordering::Acquire)
            .min(head.saturating_add(1))
            .min(start.saturating_add(BATCH_SIZE));
        if start >= limit {
            return Ok(false);
        }
        let tail = self.tail()?;
        let mut batch = WriteBatch::new();
        let mut bytes = 0;
        let mut end = limit;
        for height in start.max(tail)..limit {
            if bytes >= ARCHIVE_BATCH_BYTES {
                end = height;
                break;
            }
            let header = decode_header(&self.header_wire(height)?)?;
            let block = self
                .blocks
                .load_block(height)?
                .context("block disappeared before DA archival")?;
            ensure!(
                block.header.hash() == header.hash(),
                "DA header and consensus block disagree"
            );
            let raw = block.data.to_proto().encode_to_vec();
            bytes += raw.len();
            batch.put(at(b't', height), raw);
        }
        if !batch.is_empty() {
            self.db.write(batch)?;
        }
        progress.safe.store(end, Ordering::Release);
        Ok(true)
    }

    fn cutoff(&self) -> i128 {
        self.window.map_or(i128::MIN, |window| {
            light::verifier::now().unix_nanos() - window
        })
    }

    fn prune(&self, batch: &mut WriteBatch, mut tail: i64, h: i64) -> Result<i64> {
        let cutoff = self.cutoff();
        let limit = tail.saturating_add(BATCH_SIZE).min(h);
        while self.window.is_some() && tail < limit {
            let raw = self.header_wire(tail)?;
            let old = decode_header(&raw)?;
            if old.time.unix_nanos() >= cutoff {
                break;
            }
            batch.delete(at(b'h', tail));
            batch.delete(at(b's', tail));
            batch.delete(at(b't', tail));
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
        tail = self.prune(&mut batch, tail, h)?;
        let hash = &block.commit.block_id.hash;
        batch.put(at(b'h', h), block.header_wire()?);
        batch.put(by_hash(hash), h.to_be_bytes());
        batch.put(b"tip", hash.clone());
        batch.put(b"tail", tail.to_be_bytes());
        batch.put(b"head", h.to_be_bytes());
        self.db.write(batch)?;
        Ok(())
    }

    pub(super) fn header(&self, height: i64) -> Result<IndexedHeader> {
        let indexed = IndexedHeader::decode(&self.header_wire(height)?)?;
        let header = &indexed.header;
        let commit = &indexed.commit;
        // Indexing already verified the commit signatures and reconstructed the EDS.
        // The header binds the stored validator set and DAH without the block body.
        ensure!(
            header.chain_id == self.chain && header.height == height && commit.height == height,
            "unexpected indexed header chain or height"
        );
        ensure!(
            header.hash().as_ref() == Some(&commit.block_id.hash)
                && header.validators_hash == indexed.validators.hash()
                && header.data_hash == indexed.dah.hash(),
            "indexed header commitments disagree"
        );
        if height >= self.blocks.base()
            && let Some(meta) = self.blocks.load_block_meta(height)?
        {
            ensure!(
                meta.block_id.hash == commit.block_id.hash,
                "DA header and consensus block disagree"
            );
        } else {
            ensure!(
                self.by_hash(&commit.block_id.hash)? == Some(height),
                "DA header and indexed block hash disagree"
            );
        }
        Ok(indexed)
    }

    pub(super) fn block(&self, height: i64) -> Result<Committed> {
        let IndexedHeader {
            header,
            commit,
            validators,
            dah,
        } = IndexedHeader::decode(&self.header_wire(height)?)?;
        let consensus = if height >= self.blocks.base() {
            self.blocks.load_block(height)?
        } else {
            None
        };
        let eds = if let Some(block) = consensus {
            ensure!(
                block.header.hash().as_ref() == Some(&commit.block_id.hash),
                "DA header and consensus block disagree"
            );
            let txs: Vec<_> = block.data.txs.into_iter().map(|tx| tx.0).collect();
            rebuild(&header, &dah, &txs)?
        } else if let Some(raw) = self.db.get(&at(b't', height))? {
            let data = ctypes::proto::types::Data::decode(raw.as_slice())?;
            rebuild(&header, &dah, &data.txs)?
        } else {
            let raw = self
                .db
                .get(&at(b's', height))?
                .context("original data square unavailable")?;
            extend_legacy(&raw, &dah, &header)?
        };
        Ok(Committed {
            header,
            commit,
            validators,
            eds,
            dah,
        })
    }
}

fn decode_header(raw: &[u8]) -> Result<ctypes::Header> {
    let wire = HeaderWire::decode(raw)?;
    Ok(ctypes::Header::from_proto(
        &wire.header.context("missing indexed header")?,
    )?)
}

fn rebuild(
    header: &ctypes::Header,
    dah: &square::DataAvailabilityHeader,
    txs: &[Vec<u8>],
) -> Result<square::ExtendedDataSquare> {
    let (eds, actual) =
        app::proposal::verified_block_eds(header.version.app, txs, &header.data_hash)
            .map_err(anyhow::Error::msg)?;
    ensure!(
        &actual == dah,
        "reconstructed square does not match indexed DA header"
    );
    Ok(eds)
}

fn extend_legacy(
    raw: &[u8],
    dah: &square::DataAvailabilityHeader,
    header: &ctypes::Header,
) -> Result<square::ExtendedDataSquare> {
    let width = dah.row_roots.len() / 2;
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
        &square::new_data_availability_header(&eds)? == dah && header.data_hash == dah.hash(),
        "stored square does not match committed root"
    );
    Ok(eds)
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
            last_commit_hash: ctypes::Commit::default().hash(),
            evidence_hash: ctypes::EvidenceData::default().hash(),
            ..Default::default()
        };
        let mut commit = ctypes::Commit {
            height,
            signatures: vec![ctypes::CommitSig::absent()],
            ..Default::default()
        };
        commit.block_id.hash = header.hash().context("empty hash")?;
        Ok(Committed {
            header,
            commit,
            validators,
            eds,
            dah,
        })
    }

    #[tokio::test]
    async fn header_rpcs_return_identical_json_without_block_bodies() -> Result<()> {
        use crate::bridge::{Bridge, Query, header_json, header_rpc, ws::Endpoint as _};
        use rpc::json::J;
        use serde_json::json;
        use tokio::sync::Semaphore;

        let temp = tempfile::tempdir()?;
        let blocks = BlockStore::new(Arc::new(kvdb::MemBackend::default()))?;
        let store = Store::open(&temp.path().join("index.redb"), "controls", None, blocks)?;
        let mut expected = Vec::new();
        let mut previous = ctypes::BlockId::default();
        for height in 1..=3 {
            let mut block = block(height, height)?;
            block.header.last_block_id = previous;
            block.commit.block_id.hash = block.header.hash().context("empty hash")?;
            store.save(&block)?;
            expected.push(header_json(
                &block.header,
                &block.commit,
                &block.validators,
                &block.dah,
            ));
            previous = block.commit.block_id;
        }
        let bridge = Arc::new(Bridge {
            index: Arc::new(store),
            #[cfg(feature = "da-fibre")]
            fibre: None,
            #[cfg(feature = "bridge-p2p")]
            peer_control: None,
            app: None,
            wallet: None,
            waiters: Arc::new(Semaphore::new(1)),
            worker: Arc::new(Semaphore::new(1)),
        });
        // Only the indexed headers exist. Any attempt to reconstruct the EDS must fail.
        assert_eq!(
            bridge.read(Query::Share(2, 0, 0)).unwrap_err().to_string(),
            "original data square unavailable"
        );
        assert_eq!(bridge.read(Query::Head)?, expected[2]);
        assert_eq!(bridge.read(Query::Hash(previous.hash))?, expected[2]);
        assert_eq!(bridge.read(Query::Header(2))?, expected[1]);
        assert_eq!(bridge.read(Query::Tail)?, expected[0]);
        assert_eq!(
            header_rpc::handle(bridge.clone(), "header.WaitForHeight", &json!([2])).await?,
            expected[1]
        );
        assert_eq!(
            bridge.clone().event(2, None).await?,
            Some(expected[1].encode())
        );

        let mut from = serde_json::from_str::<serde_json::Value>(&expected[0].encode())?;
        let method = "header.GetRangeByHeight";
        let range = header_rpc::handle(bridge.clone(), method, &json!([from, 4])).await?;
        assert_eq!(range, J::Arr(expected[1..].to_vec()));
        from["commit"]["block_id"]["hash"] = json!(hex::encode([0; 32]));
        let error = header_rpc::handle(bridge, method, &json!([from, 4]))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "discontinuous header range");
        Ok(())
    }

    #[test]
    fn indexed_header_checks_data_root_and_retained_consensus_header() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blocks = BlockStore::new(Arc::new(kvdb::MemBackend::default()))?;
        let store = Store::open(
            &temp.path().join("index.redb"),
            "controls",
            None,
            blocks.clone(),
        )?;
        let mut indexed = block(1, 1)?;
        let mut consensus = ctypes::Block::new(
            indexed.header.clone(),
            ctypes::Data::new(Vec::new(), 1, Some(indexed.header.data_hash.clone())),
            ctypes::EvidenceData::default(),
            Some(ctypes::Commit::default()),
        );
        let parts = consensus.make_part_set(65_536)?;
        blocks.save_block(&mut consensus, &parts, &indexed.commit)?;
        store.save(&indexed)?;
        assert_eq!(store.header(1)?.commit, indexed.commit);

        indexed.dah.row_roots[0][0] ^= 1;
        store.save(&indexed)?;
        assert!(store.header(1).is_err());
        // This entry has consistent commitments but names a different consensus block.
        store.save(&block(1, 2)?)?;
        assert!(store.header(1).is_err());
        Ok(())
    }

    #[test]
    fn rolling_and_archival_indexes_reopen_with_consistent_height_and_hash() -> Result<()> {
        let first = block(10, 1)?;
        let next = block(11, 169 * 3600 + 2)?;
        for archival in [false, true] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("index.redb");
            let blocks = BlockStore::new(Arc::new(kvdb::MemBackend::default()))?;
            let window = (!archival).then_some(169 * HOUR_NS);
            let store = Store::open(&path, "controls", window, blocks.clone())?;
            store.save(&first)?;
            drop(store);
            let store = Store::open(&path, "controls", window, blocks.clone())?;
            assert_eq!(store.head()?, 10);
            assert_eq!(store.by_hash(&first.commit.block_id.hash)?, Some(10));
            store.save(&next)?;
            drop(store);
            let store = Store::open(&path, "controls", window, blocks.clone())?;
            assert_eq!(store.head()?, 11);
            assert_eq!(store.tail()?, if archival { 10 } else { 11 });
            assert_eq!(
                store.by_hash(&first.commit.block_id.hash)?.is_some(),
                archival
            );
            assert_eq!(store.header_wire(10).is_ok(), archival);
            assert!(store.db.get(&at(b's', 10))?.is_none());
            assert!(store.db.get(&at(b't', 10))?.is_none());
            assert_eq!(store.by_hash(&next.commit.block_id.hash)?, Some(11));
            drop(store);
            assert!(Store::open(&path, "different", window, blocks.clone()).is_err());
            assert_eq!(
                Store::open(&path, "controls", archival.then_some(HOUR_NS), blocks).is_ok(),
                archival
            );
        }
        Ok(())
    }

    #[test]
    fn reads_from_consensus_then_archives_before_pruning() -> Result<()> {
        let blocks = BlockStore::new(Arc::new(kvdb::MemBackend::default()))?;
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("index-v2.redb");
        let store = Store::open(&path, "controls", None, blocks.clone())?;
        let now = light::verifier::now()
            .unix_nanos()
            .div_euclid(1_000_000_000);
        for h in 1..=3 {
            let mut committed = block(h, i64::try_from(now)? - 7200 + h)?;
            let txs = vec![vec![0x0a, 0x00]];
            let (width, root) = app::proposal::block_data_root_with_max(11, &txs, 64)
                .map_err(anyhow::Error::msg)?;
            (committed.eds, committed.dah) =
                app::proposal::verified_block_eds(11, &txs, &root).map_err(anyhow::Error::msg)?;
            committed.header.data_hash = root.clone();
            committed.commit.block_id.hash = committed.header.hash().context("empty hash")?;
            let mut consensus = ctypes::Block::new(
                committed.header.clone(),
                ctypes::Data::new(txs.into_iter().map(ctypes::Tx).collect(), width, Some(root)),
                ctypes::EvidenceData::default(),
                Some(ctypes::Commit::default()),
            );
            let parts = consensus.make_part_set(65_536)?;
            blocks.save_block(&mut consensus, &parts, &committed.commit)?;
            store.save(&committed)?;
        }
        let expected = store.block(1)?.dah;
        assert!(store.db.get(&at(b't', 1))?.is_none());
        let progress = PruneProgress {
            requested: AtomicI64::new(2),
            safe: AtomicI64::new(0),
        };
        assert!(store.archive_pending(&progress, store.head()?)?);
        assert_eq!(progress.safe.load(Ordering::Acquire), 2);
        assert!(store.db.get(&at(b't', 1))?.is_some());
        assert!(store.db.get(&at(b't', 2))?.is_none());

        let state = nodestore::state::make_genesis_state(&nodestore::GenesisDoc {
            genesis_time: ctypes::Time::from_unix(i64::try_from(now)?, 0),
            chain_id: "controls".into(),
            initial_height: 1,
            consensus_params: ctypes::ConsensusParams::default(),
            validators: Vec::new(),
            app_hash: Vec::new(),
            app_state: Vec::new(),
        })?;
        blocks.prune_blocks(progress.safe.load(Ordering::Acquire), &state)?;
        assert!(blocks.load_block(1)?.is_none());
        drop(store);
        let store = Store::open(&path, "controls", None, blocks.clone())?;
        assert_eq!(store.block(1)?.dah, expected);
        drop(store);

        let store = Store::open(&path, "controls", Some(HOUR_NS), blocks)?;
        let mut batch = WriteBatch::new();
        let tail = store.prune(&mut batch, store.tail()?, store.head()?)?;
        batch.put(b"tail", tail.to_be_bytes());
        store.db.write(batch)?;
        assert_eq!(tail, 3);
        assert!(store.db.get(&at(b't', 1))?.is_none());
        Ok(())
    }
}
