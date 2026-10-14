//! Cosmos SDK commit multi-store primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m2/multistore.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use encoding::varint;
use iavl::{NodeCache, NodeDb};
use ics23::commitment_proof::Proof as RawCommitmentProof;
use ics23::{ExistenceProof, HashOp, InnerOp, LeafOp, LengthOp};
use kvdb::{KvBackend, WriteBatch};
use prost::Message;

pub use iavl::{Hash, Version};
pub use kvdb;

/// IAVL-backed key/value store mounted in a multi-store.
pub type KvStore = kvstore::Store;

/// Commitment information for one committed store version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitId {
    /// Committed store version.
    pub version: Version,
    /// Committed root hash.
    pub hash: Hash,
}

impl CommitId {
    /// Return the Cosmos SDK zero commit ID.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            version: 0,
            hash: hash::sha256(&[]),
        }
    }

    /// Return true when this is the zero commit ID.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::zero()
    }
}

/// Store-specific commit information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    /// Mounted store name.
    pub name: String,
    /// Commit ID for this store.
    pub commit_id: CommitId,
}

/// Multi-store commit information for one app height.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitInfo {
    /// Multi-store commit version.
    pub version: Version,
    /// Store commit infos sorted by store name.
    pub store_infos: Vec<StoreInfo>,
    /// Optional commit timestamp from the Cosmos SDK protobuf shape.
    pub timestamp: Option<prost_types::Timestamp>,
}

impl CommitInfo {
    /// Construct validated commit info and sort stores by name.
    pub fn new(version: Version, store_infos: Vec<StoreInfo>) -> Result<Self> {
        if version <= 0 {
            return Err(Error::InvalidVersion { version });
        }
        let store_infos = sorted_committed_store_infos(store_infos)?;
        Ok(Self {
            version,
            store_infos,
            timestamp: Some(go_zero_timestamp()),
        })
    }

    /// Return the Cosmos SDK app hash for this commit info.
    pub fn hash(&self) -> Result<Hash> {
        commit_info_hash(&self.store_infos)
    }

    /// Return this commit info as a commit ID.
    pub fn commit_id(&self) -> Result<CommitId> {
        Ok(CommitId {
            version: self.version,
            hash: self.hash()?,
        })
    }

    /// Return the SDK simple Merkle proof op for `store_name`.
    pub fn proof_op(&self, store_name: &str) -> Result<ics23_proofs::ProofOp> {
        proof_op_from_store_infos(&self.store_infos, store_name)
    }

    /// Return store info by name.
    #[must_use]
    pub fn store_info(&self, store_name: &str) -> Option<&StoreInfo> {
        self.store_infos
            .iter()
            .find(|store_info| store_info.name == store_name)
    }

    /// Encode this value as Cosmos SDK `CommitInfo` protobuf bytes.
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>> {
        if self.version <= 0 {
            return Err(Error::InvalidVersion {
                version: self.version,
            });
        }
        let store_infos = sorted_committed_store_infos(self.store_infos.clone())?;
        Ok(RawCommitInfo {
            version: self.version,
            store_infos: store_infos.into_iter().map(RawStoreInfo::from).collect(),
            timestamp: self.timestamp,
        }
        .encode_to_vec())
    }

    /// Decode Cosmos SDK `CommitInfo` protobuf bytes.
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self> {
        let raw = RawCommitInfo::decode(bytes)
            .map_err(|_| Error::MalformedProto("commit info decode failed"))?;
        if raw.version <= 0 {
            return Err(Error::InvalidVersion {
                version: raw.version,
            });
        }
        let mut store_infos = Vec::with_capacity(raw.store_infos.len());
        for raw_store_info in raw.store_infos {
            store_infos.push(StoreInfo::try_from(raw_store_info)?);
        }
        let mut commit_info = Self::new(raw.version, store_infos)?;
        commit_info.timestamp = raw.timestamp;
        Ok(commit_info)
    }
}

/// Mounted store kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// IAVL-backed persistent key/value store.
    Iavl,
}

/// When committed versions are deleted (Cosmos SDK pruning semantics).
///
/// After the commit of every `interval`-th version, all versions except the
/// latest, the `keep_recent` before it and multiples of `keep_every` are
/// deleted. `interval == 0` disables pruning (archive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruningOptions {
    /// Versions kept below the latest one.
    pub keep_recent: u64,
    /// Additionally keep every version divisible by this (0 = none).
    pub keep_every: u64,
    /// Prune after every `interval` commits (0 = never prune).
    pub interval: u64,
}

impl PruningOptions {
    /// Keep every version.
    #[must_use]
    pub const fn nothing() -> Self {
        Self {
            keep_recent: 0,
            keep_every: 0,
            interval: 0,
        }
    }

    /// Keep only the latest version (in-memory default).
    #[must_use]
    pub const fn latest_only() -> Self {
        Self {
            keep_recent: 0,
            keep_every: 0,
            interval: 1,
        }
    }

    /// Validator default (ADR 0002): the latest 100 versions, pruned after
    /// every commit. Pruning every commit spreads the deletes evenly; with
    /// the SDK's `interval = 10` each tenth commit deletes ten versions at
    /// once (2–4 s commit spikes at heavy load in the ADR 0002
    /// measurements).
    #[must_use]
    pub const fn validator() -> Self {
        Self {
            keep_recent: 100,
            keep_every: 0,
            interval: 1,
        }
    }

    /// True when `version` survives pruning at `latest`.
    #[must_use]
    pub fn retains(&self, version: Version, latest: Version) -> bool {
        if self.interval == 0 || version >= latest {
            return true;
        }
        let keep_recent = Version::try_from(self.keep_recent).unwrap_or(Version::MAX);
        let keep_every = Version::try_from(self.keep_every).unwrap_or(0);
        version >= latest.saturating_sub(keep_recent)
            || (keep_every > 0 && version % keep_every == 0)
    }

    /// True when pruning runs after committing `version`.
    #[must_use]
    pub fn due(&self, version: Version) -> bool {
        self.interval > 0 && u64::try_from(version).is_ok_and(|v| v % self.interval == 0)
    }
}

/// Options of a storage-backed multi-store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreOptions {
    /// Pruning policy.
    pub pruning: PruningOptions,
    /// Decoded IAVL nodes cached across all stores (roughly 200–300 B of
    /// memory each).
    pub node_cache_size: usize,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            pruning: PruningOptions::validator(),
            node_cache_size: 1_000_000,
        }
    }
}

/// Backend key of the latest committed version (Go: `s/latest`).
pub const LATEST_KEY: &[u8] = b"s/latest";
const METADATA_PREFIX: &[u8] = b"m/";

fn commit_info_key(version: Version) -> Vec<u8> {
    format!("s/{version}").into_bytes()
}

fn store_prefix(name: &str) -> Vec<u8> {
    format!("s/k:{name}/").into_bytes()
}

fn metadata_key(key: &[u8]) -> Vec<u8> {
    [METADATA_PREFIX, key].concat()
}

fn db_err(e: kvdb::Error) -> Error {
    Error::Db(e.0)
}

/// Cosmos SDK commit multi-store, in memory ([`Store::new`]) or backed by a
/// [`KvBackend`] ([`Store::open`]).
///
/// A storage-backed store writes all IAVL stores of a height, the commit
/// info, the latest-version pointer and staged metadata in **one** backend
/// batch, so a crash leaves either the previous or the new height on disk.
#[derive(Debug, Clone)]
pub struct Store {
    initial_version: Version,
    version: Version,
    stores: BTreeMap<String, KvStore>,
    last_commit_info: Option<CommitInfo>,
    history: BTreeMap<Version, CommitInfo>,
    db: Option<Arc<dyn KvBackend>>,
    cache: Arc<NodeCache>,
    pruning: PruningOptions,
    /// Metadata staged for the next commit batch (storage) or kept (memory).
    metadata: BTreeMap<Vec<u8>, Vec<u8>>,
    pending_metadata: Vec<(Vec<u8>, Vec<u8>)>,
    /// Block time recorded in the next commit info (Go `SetCommitHeader`).
    commit_timestamp: Option<prost_types::Timestamp>,
}

impl Store {
    /// Create an empty in-memory multi-store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            initial_version: 1,
            version: 0,
            stores: BTreeMap::new(),
            last_commit_info: None,
            history: BTreeMap::new(),
            db: None,
            cache: NodeCache::new(1),
            pruning: PruningOptions::latest_only(),
            metadata: BTreeMap::new(),
            pending_metadata: Vec::new(),
            commit_timestamp: None,
        }
    }

    /// Create an empty in-memory multi-store with a custom first commit version.
    pub fn with_initial_version(initial_version: Version) -> Result<Self> {
        if initial_version <= 0 {
            return Err(Error::InvalidVersion {
                version: initial_version,
            });
        }
        Ok(Self {
            initial_version,
            ..Self::new()
        })
    }

    /// Create a storage-backed multi-store over `db`. Mount the stores, then
    /// call [`Self::load_latest_version`].
    #[must_use]
    pub fn open(db: Arc<dyn KvBackend>, options: StoreOptions) -> Self {
        Self {
            db: Some(db),
            cache: NodeCache::new(options.node_cache_size),
            pruning: options.pruning,
            ..Self::new()
        }
    }

    /// The backend of a storage-backed store.
    #[must_use]
    pub fn backend(&self) -> Option<&Arc<dyn KvBackend>> {
        self.db.as_ref()
    }

    /// The shared node cache (diagnostics).
    #[must_use]
    pub fn node_cache(&self) -> &Arc<NodeCache> {
        &self.cache
    }

    /// The pruning policy.
    #[must_use]
    pub fn pruning(&self) -> PruningOptions {
        self.pruning
    }

    /// Replace the pruning policy.
    pub fn set_pruning(&mut self, pruning: PruningOptions) {
        self.pruning = pruning;
    }

    fn new_kv_store(&self, name: &str) -> Result<KvStore> {
        match &self.db {
            None => Ok(KvStore::with_initial_version(self.initial_version)?),
            Some(db) => {
                let ndb = NodeDb::new(Arc::clone(db), store_prefix(name), Arc::clone(&self.cache));
                let mut tree = iavl::Tree::open(ndb)?;
                tree.set_initial_version(self.initial_version)?;
                Ok(KvStore::from_tree(tree))
            }
        }
    }

    /// Mount a new store under `name`.
    pub fn mount_store(&mut self, name: impl Into<String>, kind: StoreKind) -> Result<()> {
        let name = name.into();
        validate_store_name(&name)?;
        if self.stores.contains_key(&name) {
            return Err(Error::DuplicateStoreName(name));
        }
        match kind {
            StoreKind::Iavl => {
                let store = self.new_kv_store(&name)?;
                self.stores.insert(name, store);
            }
        }
        Ok(())
    }

    /// Load the latest committed version from storage (Go
    /// `LoadLatestVersion`) and return its commit ID. Every mounted store is
    /// positioned at that version and checked against the stored commit
    /// info; stores missing from it (added later) start empty at the next
    /// version. A no-op returning the zero commit ID for in-memory stores
    /// and fresh databases.
    pub fn load_latest_version(&mut self) -> Result<CommitId> {
        let Some(db) = &self.db else {
            return self.last_commit_id();
        };
        let latest = match db.get(LATEST_KEY).map_err(db_err)? {
            None => 0,
            Some(raw) => Version::from_be_bytes(
                raw.as_slice()
                    .try_into()
                    .map_err(|_| Error::InvalidCommitInfo("latest version encoding"))?,
            ),
        };
        if latest == 0 {
            for (name, store) in &self.stores {
                if !store.tree().available_versions().is_empty() {
                    return Err(Error::Corrupted(format!(
                        "store {name} has versions but no commit info"
                    )));
                }
            }
            return self.last_commit_id();
        }
        let info = self.read_commit_info(latest)?;
        for (name, store) in &mut self.stores {
            let tree = store.tree_mut();
            if let Some(store_info) = info.store_info(name) {
                tree.load_version(latest)?;
                if tree.root_hash() != store_info.commit_id.hash {
                    return Err(Error::Corrupted(format!(
                        "store {name} root does not match commit info at {latest}"
                    )));
                }
            } else {
                if !tree.available_versions().is_empty() {
                    return Err(Error::Corrupted(format!(
                        "store {name} has versions but is not in commit info"
                    )));
                }
                tree.set_initial_version(latest + 1)?;
            }
        }
        for store_info in &info.store_infos {
            if !self.stores.contains_key(&store_info.name) {
                return Err(Error::UnknownStore(store_info.name.clone()));
            }
        }
        self.version = latest;
        self.last_commit_info = Some(info);
        self.last_commit_id()
    }

    fn read_commit_info(&self, version: Version) -> Result<CommitInfo> {
        let Some(db) = &self.db else {
            return self
                .history
                .get(&version)
                .cloned()
                .ok_or(Error::VersionNotFound { version });
        };
        let raw = db
            .get(&commit_info_key(version))
            .map_err(db_err)?
            .ok_or(Error::VersionNotFound { version })?;
        CommitInfo::from_proto_bytes(&raw)
    }

    /// Return true when a store is mounted under `name`.
    #[must_use]
    pub fn has_store(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// Return mounted store names in lexicographic order.
    #[must_use]
    pub fn store_names(&self) -> Vec<&str> {
        self.stores.keys().map(String::as_str).collect()
    }

    /// Return an immutable mounted store by name.
    pub fn get_store(&self, name: &str) -> Result<&KvStore> {
        self.stores
            .get(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return a mutable mounted store by name.
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore> {
        self.stores
            .get_mut(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return the latest committed multi-store version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Return the latest multi-store commit ID.
    pub fn last_commit_id(&self) -> Result<CommitId> {
        self.last_commit_info
            .as_ref()
            .map_or_else(|| Ok(CommitId::zero()), CommitInfo::commit_id)
    }

    /// Return the latest multi-store commit info.
    #[must_use]
    pub fn last_commit_info(&self) -> Option<&CommitInfo> {
        self.last_commit_info.as_ref()
    }

    /// Return the app hash for current mounted store working roots.
    pub fn working_hash(&self) -> Result<Hash> {
        commit_info_hash(&self.working_store_infos())
    }

    /// Stage `value` under application metadata `key`; it is written in the
    /// next commit's batch (storage) and readable immediately.
    pub fn set_metadata(&mut self, key: &[u8], value: Vec<u8>) {
        self.metadata.insert(key.to_vec(), value.clone());
        self.pending_metadata.push((key.to_vec(), value));
    }

    /// Application metadata written by [`Self::set_metadata`].
    pub fn get_metadata(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(value) = self.metadata.get(key) {
            return Ok(Some(value.clone()));
        }
        match &self.db {
            Some(db) => db.get(&metadata_key(key)).map_err(db_err),
            None => Ok(None),
        }
    }

    /// Commit every mounted store and return the multi-store commit ID.
    pub fn commit(&mut self) -> Result<CommitId> {
        let next_version = if self.version == 0 && self.initial_version > 1 {
            self.initial_version
        } else {
            self.version.checked_add(1).ok_or(Error::InvalidVersion {
                version: self.version,
            })?
        };
        if next_version <= 0 {
            return Err(Error::InvalidVersion {
                version: next_version,
            });
        }

        let mut batch = WriteBatch::new();
        let mut store_infos = Vec::with_capacity(self.stores.len());
        for (name, store) in &mut self.stores {
            let commit = store.tree_mut().save_version_to(&mut batch)?;
            store_infos.push(StoreInfo {
                name: name.clone(),
                commit_id: CommitId {
                    version: next_version,
                    hash: commit.root_hash,
                },
            });
        }

        let mut commit_info = CommitInfo::new(next_version, store_infos)?;
        if let Some(ts) = self.commit_timestamp.take() {
            commit_info.timestamp = Some(ts);
        }
        let commit_id = commit_info.commit_id()?;
        if let Some(db) = &self.db {
            batch.put(commit_info_key(next_version), commit_info.to_proto_bytes()?);
            batch.put(LATEST_KEY, next_version.to_be_bytes().to_vec());
            for (key, value) in self.pending_metadata.drain(..) {
                batch.put(metadata_key(&key), value);
            }
            db.write(batch).map_err(db_err)?;
        } else {
            self.pending_metadata.clear();
            self.history.insert(next_version, commit_info.clone());
        }
        self.version = next_version;
        self.last_commit_info = Some(commit_info);
        if self.pruning.due(next_version) {
            self.prune()?;
        }
        Ok(commit_id)
    }

    /// Delete every version the pruning policy no longer retains, in one
    /// backend batch. Returns the deleted versions.
    pub fn prune(&mut self) -> Result<Vec<Version>> {
        let latest = self.version;
        let pruning = self.pruning;
        let mut batch = WriteBatch::new();
        let mut deleted = BTreeSet::new();
        for store in self.stores.values_mut() {
            deleted.extend(
                store
                    .tree_mut()
                    .prune(|v| pruning.retains(v, latest), &mut batch)?,
            );
        }
        if let Some(db) = &self.db {
            for version in &deleted {
                batch.delete(commit_info_key(*version));
            }
            if !batch.is_empty() {
                db.write(batch).map_err(db_err)?;
            }
        }
        Ok(deleted.into_iter().collect())
    }

    /// Go `rootmulti.Store.RollbackToVersion`: deletes every version above
    /// `target` in every store and makes `target` the latest commit, in one
    /// backend batch. Uncommitted changes are discarded.
    pub fn rollback_to_version(&mut self, target: Version) -> Result<CommitId> {
        if target <= 0 {
            return Err(Error::InvalidVersion { version: target });
        }
        let info = self.read_commit_info(target)?;
        let mut batch = WriteBatch::new();
        for store in self.stores.values_mut() {
            let tree = store.tree_mut();
            if tree.available_versions().contains(&target) {
                tree.rollback_to(target, &mut batch)?;
            } else {
                // A store added after `target`: it did not exist yet.
                tree.rollback_to(0, &mut batch)?;
            }
        }
        if let Some(db) = &self.db {
            for v in target + 1..=self.version {
                batch.delete(commit_info_key(v));
            }
            batch.put(LATEST_KEY, target.to_be_bytes().to_vec());
            db.write(batch).map_err(db_err)?;
        } else {
            self.history.retain(|v, _| *v <= target);
        }
        self.version = target;
        self.last_commit_info = Some(info);
        self.last_commit_id()
    }

    /// Committed versions available in every store, ascending.
    #[must_use]
    pub fn available_versions(&self) -> Vec<Version> {
        let mut stores = self.stores.values();
        let Some(first) = stores.next() else {
            return Vec::new();
        };
        let mut versions: BTreeSet<Version> =
            first.tree().available_versions().into_iter().collect();
        for store in stores {
            let other: BTreeSet<Version> = store.tree().available_versions().into_iter().collect();
            versions.retain(|v| other.contains(v));
        }
        versions.into_iter().collect()
    }

    /// Sets the block time recorded in the next commit's commit info (Go
    /// `rootmulti.Store.SetCommitHeader`). Not part of the app hash; read
    /// back by historical queries as the block time.
    pub fn set_commit_timestamp(&mut self, timestamp: prost_types::Timestamp) {
        self.commit_timestamp = Some(timestamp);
    }

    /// A read-only view of committed `version` (Go
    /// `CacheMultiStoreWithVersion`): every store positioned at `version`;
    /// stores that did not exist yet at `version` are empty. Fails when the
    /// version is unknown or pruned. The view must never be committed.
    pub fn view_at(&self, version: Version) -> Result<Store> {
        let info = self.read_commit_info(version)?;
        let mut stores = BTreeMap::new();
        for (name, store) in &self.stores {
            let tree = store.tree();
            let view = if tree.version_exists(version) {
                tree.at_version(version)?
            } else if info.store_info(name).is_some() {
                return Err(Error::VersionNotFound { version });
            } else {
                iavl::Tree::new()
            };
            stores.insert(name.clone(), KvStore::from_tree(view));
        }
        Ok(Store {
            initial_version: self.initial_version,
            version,
            stores,
            last_commit_info: Some(info),
            history: BTreeMap::new(),
            db: self.db.clone(),
            cache: Arc::clone(&self.cache),
            pruning: PruningOptions::nothing(),
            metadata: BTreeMap::new(),
            pending_metadata: Vec::new(),
            commit_timestamp: None,
        })
    }

    /// Return historical commit info by version.
    pub fn get_commit_info(&self, version: Version) -> Result<CommitInfo> {
        self.read_commit_info(version)
    }

    /// Export every IAVL store at `version`, sorted by store name (the SDK
    /// snapshot order).
    pub fn export(&self, version: Version) -> Result<Vec<(String, iavl::Exporter)>> {
        if version <= 0 || version > self.version {
            return Err(Error::InvalidVersion { version });
        }
        self.stores
            .iter()
            .map(|(name, store)| Ok((name.clone(), store.tree().export(version)?)))
            .collect()
    }

    /// Start restoring a snapshot taken at `version` into this (empty)
    /// store. See [`Restorer`].
    pub fn restore(&mut self, version: Version) -> Result<Restorer<'_>> {
        if version <= 0 {
            return Err(Error::InvalidVersion { version });
        }
        if self.version != 0 {
            return Err(Error::InvalidVersion {
                version: self.version,
            });
        }
        if let Some(db) = &self.db {
            // Remove leftovers of an interrupted restore.
            let mut batch = WriteBatch::new();
            for name in self.stores.keys() {
                for item in db
                    .prefix_range(&store_prefix(name), false)
                    .map_err(db_err)?
                {
                    batch.delete(item.map_err(db_err)?.0);
                }
            }
            if !batch.is_empty() {
                db.write(batch).map_err(db_err)?;
            }
        }
        Ok(Restorer {
            store: self,
            version,
            current: None,
            imported: BTreeMap::new(),
            batch: WriteBatch::new(),
        })
    }

    /// Create a clone-backed cache multi-store.
    #[must_use]
    pub fn cache_multi_store(&self) -> CacheMultiStore {
        CacheMultiStore {
            stores: self.stores.clone(),
        }
    }

    /// Apply a cache multi-store to this parent store.
    pub fn apply_cache(&mut self, cache: CacheMultiStore) -> Result<()> {
        cache.write(self)
    }

    fn working_store_infos(&self) -> Vec<StoreInfo> {
        self.stores
            .iter()
            .map(|(name, store)| StoreInfo {
                name: name.clone(),
                commit_id: CommitId {
                    version: store.version(),
                    hash: store.root_hash(),
                },
            })
            .collect()
    }
}

/// Imports a snapshot stream store by store (Go `rootmulti.Store.Restore`).
///
/// Nodes are written while importing; the commit info and the
/// latest-version pointer are written by [`Restorer::finish`] in the same
/// batch as the store roots, so an interrupted restore leaves the store
/// empty (the leftovers are wiped by the next [`Store::restore`]).
#[derive(Debug)]
pub struct Restorer<'a> {
    store: &'a mut Store,
    version: Version,
    current: Option<(String, iavl::Importer)>,
    imported: BTreeMap<String, iavl::Tree>,
    batch: WriteBatch,
}

impl Restorer<'_> {
    fn finish_current(&mut self) -> Result<()> {
        if let Some((name, importer)) = self.current.take() {
            let tree = importer.commit(&mut self.batch)?;
            self.imported.insert(name, tree);
        }
        Ok(())
    }

    /// Begin the nodes of store `name`.
    pub fn start_store(&mut self, name: &str) -> Result<()> {
        self.finish_current()?;
        if !self.store.stores.contains_key(name) {
            return Err(Error::UnknownStore(name.to_owned()));
        }
        if self.imported.contains_key(name) {
            return Err(Error::DuplicateStoreName(name.to_owned()));
        }
        let ndb = self
            .store
            .stores
            .get(name)
            .and_then(|s| s.tree().node_db().cloned());
        self.current = Some((name.to_owned(), iavl::Importer::new(ndb, self.version)?));
        Ok(())
    }

    /// Add the next node of the current store.
    pub fn add_node(&mut self, node: iavl::ExportNode) -> Result<()> {
        match &mut self.current {
            Some((_, importer)) => Ok(importer.add(node)?),
            None => Err(Error::InvalidCommitInfo("snapshot node before store item")),
        }
    }

    /// Finish the restore: every mounted store must have been imported.
    /// Writes roots + commit info atomically and loads the restored version.
    pub fn finish(mut self) -> Result<CommitId> {
        self.finish_current()?;
        let mut store_infos = Vec::new();
        for name in self.store.stores.keys() {
            let tree = self
                .imported
                .get(name)
                .ok_or_else(|| Error::UnknownStore(name.clone()))?;
            store_infos.push(StoreInfo {
                name: name.clone(),
                commit_id: CommitId {
                    version: self.version,
                    hash: tree.root_hash(),
                },
            });
        }
        let info = CommitInfo::new(self.version, store_infos)?;
        if let Some(db) = &self.store.db {
            let mut batch = std::mem::take(&mut self.batch);
            batch.put(commit_info_key(self.version), info.to_proto_bytes()?);
            batch.put(LATEST_KEY, self.version.to_be_bytes().to_vec());
            for (key, value) in self.store.pending_metadata.drain(..) {
                batch.put(metadata_key(&key), value);
            }
            db.write(batch).map_err(db_err)?;
        } else {
            self.store.history.insert(self.version, info.clone());
        }
        for (name, tree) in std::mem::take(&mut self.imported) {
            if let Some(store) = self.store.stores.get_mut(&name) {
                *store = KvStore::from_tree(tree);
            }
        }
        self.store.version = self.version;
        self.store.last_commit_info = Some(info);
        self.store.last_commit_id()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// Clone-backed write-back cache multi-store.
#[derive(Debug, Clone)]
pub struct CacheMultiStore {
    stores: BTreeMap<String, KvStore>,
}

impl CacheMultiStore {
    /// Return true when a store is mounted under `name`.
    #[must_use]
    pub fn has_store(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// Return cached store names in lexicographic order.
    #[must_use]
    pub fn store_names(&self) -> Vec<&str> {
        self.stores.keys().map(String::as_str).collect()
    }

    /// Return an immutable cached store by name.
    pub fn get_store(&self, name: &str) -> Result<&KvStore> {
        self.stores
            .get(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return a mutable cached store by name.
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore> {
        self.stores
            .get_mut(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Flush cached store states into `parent`.
    pub fn write(self, parent: &mut Store) -> Result<()> {
        for (name, store) in self.stores {
            let parent_store = parent
                .stores
                .get_mut(&name)
                .ok_or_else(|| Error::UnknownStore(name.clone()))?;
            *parent_store = store;
        }
        Ok(())
    }

    /// Drop this cache without mutating the parent.
    pub fn discard(self) {}
}

/// Errors returned by multi-store helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A store name was empty.
    EmptyStoreName,
    /// A store name was mounted or decoded more than once.
    DuplicateStoreName(String),
    /// A store name is not mounted or not present in commit info.
    UnknownStore(String),
    /// A version was not positive where a committed version was required.
    InvalidVersion {
        /// Invalid version.
        version: Version,
    },
    /// Historical commit info was not found.
    VersionNotFound {
        /// Requested version.
        version: Version,
    },
    /// Commit info was structurally invalid.
    InvalidCommitInfo(&'static str),
    /// Protobuf bytes could not be decoded.
    MalformedProto(&'static str),
    /// Underlying key/value store error.
    Kv(kvstore::Error),
    /// Underlying IAVL error.
    Iavl(iavl::Error),
    /// Underlying simple Merkle error.
    Merkle(merkle::Error),
    /// Underlying ICS-23 proof error.
    Proof(ics23_proofs::Error),
    /// Storage backend error.
    Db(String),
    /// Persisted state is inconsistent.
    Corrupted(String),
}

impl From<kvstore::Error> for Error {
    fn from(value: kvstore::Error) -> Self {
        Self::Kv(value)
    }
}

impl From<iavl::Error> for Error {
    fn from(value: iavl::Error) -> Self {
        Self::Iavl(value)
    }
}

impl From<merkle::Error> for Error {
    fn from(value: merkle::Error) -> Self {
        Self::Merkle(value)
    }
}

impl From<ics23_proofs::Error> for Error {
    fn from(value: ics23_proofs::Error) -> Self {
        Self::Proof(value)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyStoreName => f.write_str("empty store name"),
            Self::DuplicateStoreName(name) => write!(f, "duplicate store name: {name}"),
            Self::UnknownStore(name) => write!(f, "unknown store: {name}"),
            Self::InvalidVersion { version } => write!(f, "invalid version: {version}"),
            Self::VersionNotFound { version } => write!(f, "version not found: {version}"),
            Self::InvalidCommitInfo(message) => write!(f, "invalid commit info: {message}"),
            Self::MalformedProto(message) => write!(f, "malformed protobuf: {message}"),
            Self::Kv(err) => write!(f, "kvstore: {err}"),
            Self::Iavl(err) => write!(f, "iavl: {err}"),
            Self::Merkle(err) => write!(f, "merkle: {err}"),
            Self::Proof(err) => write!(f, "proof: {err}"),
            Self::Db(err) => write!(f, "storage: {err}"),
            Self::Corrupted(err) => write!(f, "corrupted state: {err}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result alias for multi-store helpers.
pub type Result<T> = core::result::Result<T, Error>;

/// Return the Cosmos SDK app hash for store commit infos.
pub fn commit_info_hash(store_infos: &[StoreInfo]) -> Result<Hash> {
    let store_infos = sorted_store_infos(store_infos.to_vec())?;
    if store_infos.is_empty() {
        return Ok(hash::sha256(&[]));
    }
    let leaves = simple_map_leaves(&store_infos);
    Ok(merkle::root_from_byte_slices(&leaves))
}

/// Return the SDK simple Merkle proof op for `store_name`.
pub fn proof_op_from_store_infos(
    store_infos: &[StoreInfo],
    store_name: &str,
) -> Result<ics23_proofs::ProofOp> {
    validate_store_name(store_name)?;
    let store_infos = sorted_store_infos(store_infos.to_vec())?;
    let leaves = simple_map_leaves(&store_infos);
    let (_, proofs) = merkle::proofs_from_byte_slices(&leaves);
    let Some((index, store_info)) = store_infos
        .iter()
        .enumerate()
        .find(|(_, store_info)| store_info.name == store_name)
    else {
        return Err(Error::UnknownStore(store_name.to_owned()));
    };
    let proof = proofs
        .get(index)
        .ok_or(Error::InvalidCommitInfo("missing simple Merkle proof"))?;
    let commitment_proof = ics23::CommitmentProof {
        proof: Some(RawCommitmentProof::Exist(ExistenceProof {
            key: store_info.name.as_bytes().to_vec(),
            value: store_info.commit_id.hash.to_vec(),
            leaf: Some(LeafOp {
                hash: HashOp::Sha256.into(),
                prehash_key: HashOp::NoHash.into(),
                prehash_value: HashOp::Sha256.into(),
                length: LengthOp::VarProto.into(),
                prefix: vec![0],
            }),
            path: inner_ops_from_simple_proof(proof)?,
        })),
    };
    Ok(ics23_proofs::ProofOp {
        field_type: ics23_proofs::PROOF_OP_SIMPLE_MERKLE_COMMITMENT.to_owned(),
        key: store_info.name.as_bytes().to_vec(),
        data: commitment_proof.encode_to_vec(),
    })
}

fn sorted_store_infos(mut store_infos: Vec<StoreInfo>) -> Result<Vec<StoreInfo>> {
    let mut names = BTreeSet::new();
    for store_info in &store_infos {
        validate_store_name(&store_info.name)?;
        if !names.insert(store_info.name.clone()) {
            return Err(Error::DuplicateStoreName(store_info.name.clone()));
        }
    }
    store_infos.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(store_infos)
}

fn sorted_committed_store_infos(store_infos: Vec<StoreInfo>) -> Result<Vec<StoreInfo>> {
    let store_infos = sorted_store_infos(store_infos)?;
    for store_info in &store_infos {
        if store_info.commit_id.version <= 0 {
            return Err(Error::InvalidVersion {
                version: store_info.commit_id.version,
            });
        }
    }
    Ok(store_infos)
}

fn validate_store_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::EmptyStoreName);
    }
    Ok(())
}

fn go_zero_timestamp() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: -62_135_596_800,
        nanos: 0,
    }
}

fn simple_map_leaves(store_infos: &[StoreInfo]) -> Vec<Vec<u8>> {
    store_infos
        .iter()
        .map(|store_info| {
            kv_pair_bytes(
                store_info.name.as_bytes(),
                &hash::sha256(&store_info.commit_id.hash),
            )
        })
        .collect()
}

fn kv_pair_bytes(key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        varint::encoded_uvarint_len(key.len() as u64)
            + key.len()
            + varint::encoded_uvarint_len(value.len() as u64)
            + value.len(),
    );
    bytes.extend(varint::encode_uvarint(key.len() as u64));
    bytes.extend(key);
    bytes.extend(varint::encode_uvarint(value.len() as u64));
    bytes.extend(value);
    bytes
}

fn inner_ops_from_simple_proof(proof: &merkle::Proof) -> Result<Vec<InnerOp>> {
    proof.validate_basic()?;
    let path = simple_proof_path(proof.index, proof.total);
    if path.len() != proof.aunts.len() {
        return Err(Error::Merkle(merkle::Error::UnexpectedAuntCount));
    }
    Ok(proof
        .aunts
        .iter()
        .zip(path)
        .map(|(aunt, aunt_is_right)| {
            if aunt_is_right {
                InnerOp {
                    hash: HashOp::Sha256.into(),
                    prefix: vec![1],
                    suffix: aunt.to_vec(),
                }
            } else {
                let mut prefix = Vec::with_capacity(1 + aunt.len());
                prefix.push(1);
                prefix.extend(aunt);
                InnerOp {
                    hash: HashOp::Sha256.into(),
                    prefix,
                    suffix: Vec::new(),
                }
            }
        })
        .collect())
}

fn simple_proof_path(index: u64, total: u64) -> Vec<bool> {
    if total < 2 {
        return Vec::new();
    }
    let left_count = split_point(total);
    if index < left_count {
        let mut path = simple_proof_path(index, left_count);
        path.push(true);
        path
    } else {
        let mut path = simple_proof_path(index - left_count, total - left_count);
        path.push(false);
        path
    }
}

fn split_point(length: u64) -> u64 {
    debug_assert!(length > 1);
    let bit_len = u64::BITS - (length - 1).leading_zeros();
    1u64 << (bit_len - 1)
}

#[derive(Clone, PartialEq, Message)]
struct RawCommitInfo {
    #[prost(int64, tag = "1")]
    version: Version,
    #[prost(message, repeated, tag = "2")]
    store_infos: Vec<RawStoreInfo>,
    #[prost(message, optional, tag = "3")]
    timestamp: Option<prost_types::Timestamp>,
}

#[derive(Clone, PartialEq, Message)]
struct RawStoreInfo {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    commit_id: Option<RawCommitId>,
}

impl From<StoreInfo> for RawStoreInfo {
    fn from(value: StoreInfo) -> Self {
        Self {
            name: value.name,
            commit_id: Some(RawCommitId::from(value.commit_id)),
        }
    }
}

impl TryFrom<RawStoreInfo> for StoreInfo {
    type Error = Error;

    fn try_from(value: RawStoreInfo) -> Result<Self> {
        Ok(Self {
            name: value.name,
            commit_id: CommitId::try_from(
                value
                    .commit_id
                    .ok_or(Error::InvalidCommitInfo("missing store commit ID"))?,
            )?,
        })
    }
}

#[derive(Clone, PartialEq, Message)]
struct RawCommitId {
    #[prost(int64, tag = "1")]
    version: Version,
    #[prost(bytes = "vec", tag = "2")]
    hash: Vec<u8>,
}

impl From<CommitId> for RawCommitId {
    fn from(value: CommitId) -> Self {
        Self {
            version: value.version,
            hash: value.hash.to_vec(),
        }
    }
}

impl TryFrom<RawCommitId> for CommitId {
    type Error = Error;

    fn try_from(value: RawCommitId) -> Result<Self> {
        if value.version <= 0 {
            return Err(Error::InvalidVersion {
                version: value.version,
            });
        }
        let hash = value
            .hash
            .try_into()
            .map_err(|_| Error::InvalidCommitInfo("commit hash must be 32 bytes"))?;
        Ok(Self {
            version: value.version,
            hash,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn store_info_lookup_cache_helpers_and_defaults_work() {
        let store_info = StoreInfo {
            name: "auth".to_owned(),
            commit_id: CommitId {
                version: 1,
                hash: hash::sha256(b"auth"),
            },
        };
        let info = CommitInfo::new(1, vec![store_info]).expect("commit info");
        assert!(info.store_info("auth").is_some());
        assert!(info.store_info("bank").is_none());

        let mut store = Store::default();
        assert!(!store.has_store("bank"));
        store
            .mount_store("bank", StoreKind::Iavl)
            .expect("mount bank");
        assert!(store.has_store("bank"));

        let cache = store.cache_multi_store();
        assert!(cache.has_store("bank"));
        assert_eq!(cache.store_names(), vec!["bank"]);
        assert!(cache.get_store("bank").is_ok());
        assert!(matches!(
            cache.get_store("missing"),
            Err(Error::UnknownStore(name)) if name == "missing"
        ));

        let parent_without_bank = &mut Store::new();
        assert_eq!(
            cache.write(parent_without_bank),
            Err(Error::UnknownStore("bank".to_owned()))
        );
    }

    #[test]
    fn custom_initial_version_reaches_mounted_iavl_stores() {
        let mut custom = Store::with_initial_version(7).expect("custom store");
        custom
            .mount_store("bank", StoreKind::Iavl)
            .expect("mount bank");
        custom
            .get_store_mut("bank")
            .expect("bank store")
            .set(b"k".to_vec(), b"v".to_vec())
            .expect("set bank key");
        let custom_commit = custom.commit().expect("custom commit");
        assert_eq!(custom_commit.version, 7);

        let custom_info = custom.last_commit_info().expect("custom commit info");
        assert_eq!(custom_info.store_infos[0].commit_id.version, 7);

        let mut default = Store::new();
        default
            .mount_store("bank", StoreKind::Iavl)
            .expect("mount bank");
        default
            .get_store_mut("bank")
            .expect("bank store")
            .set(b"k".to_vec(), b"v".to_vec())
            .expect("set bank key");
        let default_commit = default.commit().expect("default commit");
        assert_eq!(default_commit.version, 1);
        let default_info = default.last_commit_info().expect("default commit info");
        assert_ne!(
            custom_info.store_infos[0].commit_id.hash,
            default_info.store_infos[0].commit_id.hash
        );
    }

    #[test]
    fn proto_error_paths_are_typed() {
        let invalid = CommitInfo {
            version: 0,
            store_infos: Vec::new(),
            timestamp: None,
        };
        assert_eq!(
            invalid.to_proto_bytes(),
            Err(Error::InvalidVersion { version: 0 })
        );

        let invalid_version = RawCommitInfo {
            version: 0,
            store_infos: Vec::new(),
            timestamp: None,
        }
        .encode_to_vec();
        assert_eq!(
            CommitInfo::from_proto_bytes(&invalid_version),
            Err(Error::InvalidVersion { version: 0 })
        );

        let missing_commit_id = RawCommitInfo {
            version: 1,
            store_infos: vec![RawStoreInfo {
                name: "bank".to_owned(),
                commit_id: None,
            }],
            timestamp: None,
        }
        .encode_to_vec();
        assert_eq!(
            CommitInfo::from_proto_bytes(&missing_commit_id),
            Err(Error::InvalidCommitInfo("missing store commit ID"))
        );

        let invalid_commit_id_version = RawCommitInfo {
            version: 1,
            store_infos: vec![RawStoreInfo {
                name: "bank".to_owned(),
                commit_id: Some(RawCommitId {
                    version: 0,
                    hash: hash::sha256(b"bank").to_vec(),
                }),
            }],
            timestamp: None,
        }
        .encode_to_vec();
        assert_eq!(
            CommitInfo::from_proto_bytes(&invalid_commit_id_version),
            Err(Error::InvalidVersion { version: 0 })
        );
    }

    #[test]
    fn commit_rejects_impossible_next_versions() {
        let mut overflow = Store {
            version: Version::MAX,
            ..Store::new()
        };
        assert_eq!(
            overflow.commit(),
            Err(Error::InvalidVersion {
                version: Version::MAX
            })
        );

        let mut negative = Store {
            version: -2,
            ..Store::new()
        };
        assert_eq!(
            negative.commit(),
            Err(Error::InvalidVersion { version: -1 })
        );
    }

    #[test]
    fn simple_proof_generation_covers_left_and_malformed_paths() {
        let auth_hash = hash::sha256(b"auth");
        let bank_hash = hash::sha256(b"bank");
        let store_infos = vec![
            StoreInfo {
                name: "bank".to_owned(),
                commit_id: CommitId {
                    version: 1,
                    hash: bank_hash,
                },
            },
            StoreInfo {
                name: "auth".to_owned(),
                commit_id: CommitId {
                    version: 1,
                    hash: auth_hash,
                },
            },
        ];
        let root = commit_info_hash(&store_infos).expect("commit info hash");
        let op = proof_op_from_store_infos(&store_infos, "auth").expect("auth proof op");
        let commitment = ics23_proofs::CommitmentOp::from_proof_op(op).expect("commitment op");
        assert_eq!(
            commitment.run(&[&auth_hash]).expect("run proof op"),
            vec![root]
        );

        let malformed = merkle::Proof {
            total: 2,
            index: 0,
            leaf_hash: hash::sha256(b"leaf"),
            aunts: Vec::new(),
        };
        assert_eq!(
            inner_ops_from_simple_proof(&malformed),
            Err(Error::Merkle(merkle::Error::UnexpectedAuntCount))
        );
    }

    #[test]
    fn error_display_and_from_impls_are_covered() {
        let errors = [
            Error::EmptyStoreName,
            Error::DuplicateStoreName("bank".to_owned()),
            Error::UnknownStore("bank".to_owned()),
            Error::InvalidVersion { version: 0 },
            Error::VersionNotFound { version: 9 },
            Error::InvalidCommitInfo("bad info"),
            Error::MalformedProto("bad proto"),
            Error::from(kvstore::Error::EmptyKey),
            Error::from(iavl::Error::EmptyKey),
            Error::from(merkle::Error::InvalidTotal),
            Error::from(ics23_proofs::Error::EmptyKey),
        ];

        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
