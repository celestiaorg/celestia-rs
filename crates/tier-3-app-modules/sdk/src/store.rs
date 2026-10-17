//! Layered multistore with Cosmos SDK cache semantics.
//!
//! The committed state is a [`multistore::Store`] of IAVL trees. On top of it
//! sits a stack of write-back cache layers, mirroring the SDK's nested
//! `CacheMultiStore`s: layer 0 is the block-level `finalizeBlockState`, and
//! each transaction/message branch pushes another layer. Writes are only
//! applied to IAVL by [`MultiStore::flush`], which — like `cachekv.Write` —
//! applies dirty keys per store in ascending key order. IAVL tree shape (and
//! therefore the app hash) depends on that order.

use std::collections::BTreeMap;
use std::ops::Bound;

use multistore::{CommitId, StoreKind};

/// Kind of a mounted store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// IAVL-backed, committed, part of the app hash.
    Iavl,
    /// Cleared at every [`MultiStore::flush`]; never committed.
    Transient,
    /// Kept in memory across blocks; never committed.
    Memory,
}

type Writes = BTreeMap<Vec<u8>, Option<Vec<u8>>>;

/// One cache layer: pending writes per store.
type Layer = BTreeMap<String, Writes>;

/// Live key/value pairs returned by [`MultiStore::range`].
pub type Entries = Vec<(Vec<u8>, Vec<u8>)>;

/// Layered multistore.
#[derive(Debug)]
pub struct MultiStore {
    root: multistore::Store,
    kinds: BTreeMap<String, Kind>,
    /// Backing data for transient and memory stores.
    volatile: BTreeMap<String, BTreeMap<Vec<u8>, Vec<u8>>>,
    /// Layer stacks: `[finalize, check]` (`finalizeBlockState`, `checkState`).
    stacks: [Vec<Layer>; 2],
    /// Index of the stack used by reads/writes (set by the borrowing context).
    active: usize,
    /// Query views ([`Self::view_at`]): stores that did not exist at the
    /// viewed version. Go keeps a nil store for them, so any access panics
    /// (`runtime error: invalid memory address or nil pointer
    /// dereference`).
    nil_stores: std::collections::BTreeSet<String>,
}

/// Which branch of committed state a context operates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stack {
    /// Block execution (`finalizeBlockState`); flushed into IAVL each block.
    Finalize = 0,
    /// Mempool validation (`checkState`); discarded at every commit.
    Check = 1,
}

/// Errors from the store layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Store name not mounted.
    #[error("unknown store {0}")]
    UnknownStore(String),
    /// Underlying multistore error.
    #[error("multistore: {0}")]
    Multistore(String),
    /// Empty key.
    #[error("empty key")]
    EmptyKey,
    /// Access to a store a query view does not have (Go: a nil store).
    #[error("runtime error: invalid memory address or nil pointer dereference")]
    NilStore,
}

impl From<multistore::Error> for StoreError {
    fn from(e: multistore::Error) -> Self {
        Self::Multistore(e.to_string())
    }
}

impl From<kvstore::Error> for StoreError {
    fn from(e: kvstore::Error) -> Self {
        Self::Multistore(e.to_string())
    }
}

impl Default for MultiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiStore {
    /// Creates an empty multistore with one (block-level) cache layer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: multistore::Store::new(),
            kinds: BTreeMap::new(),
            volatile: BTreeMap::new(),
            stacks: [vec![BTreeMap::new()], vec![BTreeMap::new()]],
            active: 0,
            nil_stores: std::collections::BTreeSet::new(),
        }
    }

    /// Creates a storage-backed multistore over `db` (ADR 0002). Mount the
    /// stores, then call [`Self::load_latest_version`]. Transient and memory
    /// stores stay in memory.
    #[must_use]
    pub fn open(
        db: std::sync::Arc<dyn multistore::kvdb::KvBackend>,
        options: multistore::StoreOptions,
    ) -> Self {
        Self {
            root: multistore::Store::open(db, options),
            ..Self::new()
        }
    }

    /// Positions every IAVL store at the latest committed version
    /// (`LoadLatestVersion`) and returns its commit ID.
    pub fn load_latest_version(&mut self) -> Result<CommitId, StoreError> {
        Ok(self.root.load_latest_version()?)
    }

    /// A read-only branch of committed `version` for queries (Go
    /// `CacheMultiStoreWithVersion`): IAVL stores at `version`, transient
    /// and memory stores as they are now. Writes go to cache layers that
    /// are dropped with the view.
    /// Stores added after `version` (e.g. the v10 upgrade's `fibre` and
    /// `valaddr` at the upgrade height) are nil there: any access fails
    /// with [`StoreError::NilStore`].
    pub fn view_at(&self, version: i64) -> Result<Self, StoreError> {
        Ok(Self {
            root: self.root.view_at(version)?,
            kinds: self.kinds.clone(),
            volatile: self.volatile.clone(),
            stacks: [vec![BTreeMap::new()], vec![BTreeMap::new()]],
            active: 0,
            nil_stores: self.root.absent_at(version)?.into_iter().collect(),
        })
    }

    fn check_nil(&self, store: &str) -> Result<(), StoreError> {
        if self.nil_stores.contains(store) {
            return Err(StoreError::NilStore);
        }
        Ok(())
    }

    /// Mounts a store.
    pub fn mount(&mut self, name: &str, kind: Kind) -> Result<(), StoreError> {
        if kind == Kind::Iavl {
            self.root.mount_store(name, StoreKind::Iavl)?;
        } else {
            self.volatile.insert(name.to_owned(), BTreeMap::new());
        }
        self.kinds.insert(name.to_owned(), kind);
        Ok(())
    }

    /// Mounts a new IAVL store whose first commit is `initial_version` (a
    /// store upgrade's `Added` store).
    pub fn mount_iavl_at(&mut self, name: &str, initial_version: i64) -> Result<(), StoreError> {
        self.root
            .mount_store_at(name, StoreKind::Iavl, initial_version)?;
        self.kinds.insert(name.to_owned(), Kind::Iavl);
        Ok(())
    }

    /// Unmounts an IAVL store that holds no committed version.
    pub fn unmount_iavl(&mut self, name: &str) -> Result<(), StoreError> {
        self.root.unmount_store(name)?;
        self.kinds.remove(name);
        Ok(())
    }

    /// Whether a store is mounted.
    #[must_use]
    pub fn is_mounted(&self, name: &str) -> bool {
        self.kinds.contains_key(name)
    }

    /// Returns the kind of a mounted store.
    pub fn kind(&self, name: &str) -> Result<Kind, StoreError> {
        self.kinds
            .get(name)
            .copied()
            .ok_or_else(|| StoreError::UnknownStore(name.to_owned()))
    }

    /// Selects the layer stack used by subsequent operations.
    pub fn select(&mut self, stack: Stack) {
        self.active = stack as usize;
    }

    /// Discards all check-state writes (`setState(execModeCheck)` at commit).
    pub fn reset_check(&mut self) {
        self.stacks[Stack::Check as usize] = vec![BTreeMap::new()];
    }

    /// Current branch depth (1 = only the block layer).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stacks[self.active].len()
    }

    /// Pushes a new cache layer.
    pub fn branch(&mut self) {
        self.stacks[self.active].push(BTreeMap::new());
    }

    /// Pops the top layer and merges it into the one below (`cache.Write()`).
    pub fn write_branch(&mut self) {
        if self.stacks[self.active].len() < 2 {
            return;
        }
        let top = self.stacks[self.active].pop().unwrap_or_default();
        let below = self.stacks[self.active].last_mut();
        if let Some(below) = below {
            for (store, writes) in top {
                below.entry(store).or_default().extend(writes);
            }
        }
    }

    /// Pops and discards the top layer.
    pub fn discard_branch(&mut self) {
        if self.stacks[self.active].len() > 1 {
            self.stacks[self.active].pop();
        }
    }

    fn committed_get(&self, store: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        match self.kind(store)? {
            Kind::Iavl => Ok(self.root.get_store(store)?.get(key)?),
            Kind::Transient | Kind::Memory => {
                Ok(self.volatile.get(store).and_then(|m| m.get(key)).cloned())
            }
        }
    }

    /// Reads `key`, walking cache layers top-down.
    pub fn get(&self, store: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.check_nil(store)?;
        for layer in self.stacks[self.active].iter().rev() {
            if let Some(v) = layer.get(store).and_then(|w| w.get(key)) {
                return Ok(v.clone());
            }
        }
        self.committed_get(store, key)
    }

    /// Writes `key` in the top layer.
    pub fn set(&mut self, store: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        self.check_nil(store)?;
        if key.is_empty() {
            return Err(StoreError::EmptyKey);
        }
        self.kind(store)?;
        self.top()
            .entry(store.to_owned())
            .or_default()
            .insert(key, Some(value));
        Ok(())
    }

    /// Deletes `key` in the top layer.
    pub fn delete(&mut self, store: &str, key: &[u8]) -> Result<(), StoreError> {
        self.check_nil(store)?;
        if key.is_empty() {
            return Err(StoreError::EmptyKey);
        }
        self.kind(store)?;
        self.top()
            .entry(store.to_owned())
            .or_default()
            .insert(key.to_vec(), None);
        Ok(())
    }

    fn top(&mut self) -> &mut Layer {
        if self.stacks[self.active].is_empty() {
            self.stacks[self.active].push(BTreeMap::new());
        }
        let n = self.stacks[self.active].len() - 1;
        &mut self.stacks[self.active][n]
    }

    /// Returns all live entries in `[start, end)` merged across layers, in
    /// ascending (or descending) key order.
    pub fn range(
        &self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> Result<Entries, StoreError> {
        self.check_nil(store)?;
        let lo = start.map_or(Bound::Unbounded, |s| Bound::Included(s.to_vec()));
        let hi = end.map_or(Bound::Unbounded, |e| Bound::Excluded(e.to_vec()));
        let mut merged: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
        match self.kind(store)? {
            Kind::Iavl => {
                for e in self.root.get_store(store)?.iter(start, end)? {
                    merged.insert(e.key, Some(e.value));
                }
            }
            Kind::Transient | Kind::Memory => {
                if let Some(m) = self.volatile.get(store) {
                    for (k, v) in m.range((lo.clone(), hi.clone())) {
                        merged.insert(k.clone(), Some(v.clone()));
                    }
                }
            }
        }
        for layer in &self.stacks[self.active] {
            if let Some(w) = layer.get(store) {
                for (k, v) in w.range((lo.clone(), hi.clone())) {
                    merged.insert(k.clone(), v.clone());
                }
            }
        }
        let iter = merged.into_iter().filter_map(|(k, v)| v.map(|v| (k, v)));
        Ok(if reverse {
            iter.rev().collect()
        } else {
            iter.collect()
        })
    }

    /// Flushes all cache layers into the committed stores in ascending key
    /// order per store (stores in name order), and clears transient stores.
    /// Equivalent to `finalizeBlockState.ms.Write()`.
    pub fn flush(&mut self) -> Result<(), StoreError> {
        let prev = self.active;
        self.active = Stack::Finalize as usize;
        while self.stacks[self.active].len() > 1 {
            self.write_branch();
        }
        self.active = prev;
        let block = std::mem::take(&mut self.stacks[Stack::Finalize as usize][0]);
        for (store, writes) in block {
            match self.kind(&store)? {
                Kind::Iavl => {
                    let s = self.root.get_store_mut(&store)?;
                    for (k, v) in writes {
                        match v {
                            Some(v) => {
                                s.set(k, v)?;
                            }
                            None => {
                                s.delete(&k)?;
                            }
                        }
                    }
                }
                Kind::Memory => {
                    let m = self.volatile.entry(store).or_default();
                    for (k, v) in writes {
                        match v {
                            Some(v) => {
                                m.insert(k, v);
                            }
                            None => {
                                m.remove(&k);
                            }
                        }
                    }
                }
                Kind::Transient => {}
            }
        }
        for (name, kind) in &self.kinds {
            if *kind == Kind::Transient
                && let Some(m) = self.volatile.get_mut(name)
            {
                m.clear();
            }
        }
        Ok(())
    }

    /// Hash of the committed stores as they would be committed now.
    pub fn working_hash(&self) -> Result<[u8; 32], StoreError> {
        Ok(self.root.working_hash()?)
    }

    /// Commits the IAVL stores (after [`Self::flush`]).
    pub fn commit(&mut self) -> Result<CommitId, StoreError> {
        Ok(self.root.commit()?)
    }

    /// Last committed version.
    #[must_use]
    pub fn version(&self) -> i64 {
        self.root.version()
    }

    /// Committed root store (read-only), e.g. for per-store hashes.
    #[must_use]
    pub fn root(&self) -> &multistore::Store {
        &self.root
    }

    /// Committed root store, mutably (snapshot restore, pruning, metadata).
    /// Callers must not leave uncommitted IAVL writes behind.
    pub fn root_mut(&mut self) -> &mut multistore::Store {
        &mut self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms() -> MultiStore {
        let mut m = MultiStore::new();
        m.mount("a", Kind::Iavl).unwrap();
        m.mount("t", Kind::Transient).unwrap();
        m
    }

    #[test]
    fn branch_write_and_discard() -> Result<(), StoreError> {
        let mut m = ms();
        m.set("a", b"k1".to_vec(), b"v1".to_vec())?;
        m.branch();
        m.set("a", b"k2".to_vec(), b"v2".to_vec())?;
        m.delete("a", b"k1")?;
        assert_eq!(m.get("a", b"k1")?, None);
        m.discard_branch();
        assert_eq!(m.get("a", b"k1")?, Some(b"v1".to_vec()));
        assert_eq!(m.get("a", b"k2")?, None);
        m.branch();
        m.set("a", b"k2".to_vec(), b"v2".to_vec())?;
        m.write_branch();
        assert_eq!(m.range("a", None, None, false)?.len(), 2);
        assert_eq!(
            m.range("a", Some(b"k2"), None, true)?,
            vec![(b"k2".to_vec(), b"v2".to_vec())]
        );
        Ok(())
    }

    #[test]
    fn flush_commits_iavl_and_clears_transient() -> Result<(), StoreError> {
        let mut m = ms();
        m.set("a", b"k".to_vec(), b"v".to_vec())?;
        m.set("t", b"k".to_vec(), b"v".to_vec())?;
        m.flush()?;
        assert_eq!(m.get("t", b"k")?, None);
        let id = m.commit()?;
        assert_eq!(id.version, 1);
        assert_eq!(m.get("a", b"k")?, Some(b"v".to_vec()));
        Ok(())
    }

    #[test]
    fn unknown_store_and_empty_key() {
        let mut m = ms();
        assert!(m.get("zz", b"k").is_err());
        assert!(m.set("a", vec![], vec![]).is_err());
    }
}

#[cfg(test)]
mod stack_tests {
    use super::*;

    #[test]
    fn check_stack_is_isolated_and_reset() -> Result<(), StoreError> {
        let mut m = MultiStore::new();
        m.mount("a", Kind::Iavl)?;
        m.select(Stack::Check);
        m.set("a", b"k".to_vec(), b"check".to_vec())?;
        m.select(Stack::Finalize);
        assert_eq!(m.get("a", b"k")?, None);
        m.set("a", b"k".to_vec(), b"final".to_vec())?;
        m.select(Stack::Check);
        assert_eq!(m.get("a", b"k")?, Some(b"check".to_vec()));
        m.flush()?;
        m.commit()?;
        m.reset_check();
        assert_eq!(m.get("a", b"k")?, Some(b"final".to_vec()));
        Ok(())
    }
}
