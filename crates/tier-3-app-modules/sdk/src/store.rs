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

type BaseIter = Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + Send>;

/// A cache layer write in a range: the value, or `None` for a delete.
type Write = (Vec<u8>, Option<Vec<u8>>);

/// Lazy merged range of a [`MultiStore`] store (see
/// [`MultiStore::lazy_range`]): committed entries, read on demand, merged
/// with a snapshot of the cache layers' writes; a layered write shadows the
/// committed value and a layered delete hides it.
pub struct LazyRange {
    base: std::iter::Peekable<BaseIter>,
    overlay: std::iter::Peekable<std::vec::IntoIter<Write>>,
    reverse: bool,
    failed: bool,
}

impl std::fmt::Debug for LazyRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyRange")
            .field("reverse", &self.reverse)
            .finish_non_exhaustive()
    }
}

impl Iterator for LazyRange {
    type Item = Result<(Vec<u8>, Vec<u8>), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            let from_base = match (self.base.peek(), self.overlay.peek()) {
                (None, None) => return None,
                (Some(Err(_)), _) => {
                    self.failed = true;
                    return self.base.next();
                }
                (Some(Ok(_)), None) => true,
                (None, Some(_)) => false,
                (Some(Ok((bk, _))), Some((ok, _))) => {
                    let mut ord = bk.cmp(ok);
                    if self.reverse {
                        ord = ord.reverse();
                    }
                    match ord {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Greater => false,
                        std::cmp::Ordering::Equal => {
                            // The layered write wins over the committed value.
                            self.base.next();
                            false
                        }
                    }
                }
            };
            if from_base {
                return self.base.next();
            }
            if let Some((k, Some(v))) = self.overlay.next() {
                return Some(Ok((k, v)));
            }
            // A layered delete: skip the key.
        }
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

    /// Lists `name` in every commit info with the empty commit ID: a
    /// cosmos-sdk v0.46 memory store as rootmulti commits it (e.g.
    /// `mem_capability`, celestia-app v1 to v3). It holds no data; the
    /// module's memory data lives in a [`Kind::Memory`] store.
    pub fn mount_commit_memory(&mut self, name: &str) -> Result<(), StoreError> {
        Ok(self.root.mount_store(name, StoreKind::Memory)?)
    }

    /// Mounts a new IAVL store whose first commit is `initial_version` (a
    /// store upgrade's `Added` store).
    pub fn mount_iavl_at(&mut self, name: &str, initial_version: i64) -> Result<(), StoreError> {
        self.root
            .mount_store_at(name, StoreKind::Iavl, initial_version)?;
        self.kinds.insert(name.to_owned(), Kind::Iavl);
        Ok(())
    }

    /// Mounts a new IAVL store as cosmos-sdk v0.46 adds one in a store
    /// upgrade ([`multistore::Store::mount_iavl_store_at_v019`]).
    pub fn mount_iavl_at_v019(
        &mut self,
        name: &str,
        initial_version: i64,
    ) -> Result<(), StoreError> {
        self.root.mount_iavl_store_at_v019(name, initial_version)?;
        self.kinds.insert(name.to_owned(), Kind::Iavl);
        Ok(())
    }

    /// Removes an IAVL store from the commit set (a store upgrade's
    /// deleted store, [`multistore::Store::remove_store`]), with its
    /// pending writes.
    pub fn remove_iavl(&mut self, name: &str) -> Result<(), StoreError> {
        if self.kind(name)? != Kind::Iavl {
            return Err(StoreError::UnknownStore(name.to_owned()));
        }
        self.root.remove_store(name)?;
        self.kinds.remove(name);
        for stack in &mut self.stacks {
            for layer in stack {
                layer.remove(name);
            }
        }
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

    /// Empties every memory store, as in a freshly started process (memory
    /// stores are never persisted).
    pub fn clear_memory_stores(&mut self) {
        for (name, kind) in &self.kinds {
            if *kind == Kind::Memory {
                self.volatile.remove(name);
            }
        }
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
            Kind::Iavl => {
                crate::profile::count_read();
                Ok(self.root.get_store(store)?.get(key)?)
            }
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
        self.lazy_range(store, start, end, reverse)?.collect()
    }

    /// [`Self::range`] as a lazy iterator: the committed IAVL entries are
    /// read only as far as the caller iterates, merged with the cache
    /// layers' writes in the range (copied when the iterator is created,
    /// so later writes do not change what it yields, as with `range`).
    pub fn lazy_range(
        &self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> Result<LazyRange, StoreError> {
        self.check_nil(store)?;
        let lo = start.map_or(Bound::Unbounded, |s| Bound::Included(s.to_vec()));
        let hi = end.map_or(Bound::Unbounded, |e| Bound::Excluded(e.to_vec()));
        let base: BaseIter = match self.kind(store)? {
            Kind::Iavl => Box::new(
                self.root
                    .get_store(store)?
                    .lazy_iter(start, end, reverse)?
                    .map(|e| e.map(|e| (e.key, e.value)).map_err(StoreError::from)),
            ),
            Kind::Transient | Kind::Memory => {
                let mut entries: Entries = self
                    .volatile
                    .get(store)
                    .map(|m| {
                        m.range((lo.clone(), hi.clone()))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if reverse {
                    entries.reverse();
                }
                Box::new(entries.into_iter().map(Ok))
            }
        };
        let mut writes: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
        for layer in &self.stacks[self.active] {
            if let Some(w) = layer.get(store) {
                for (k, v) in w.range((lo.clone(), hi.clone())) {
                    writes.insert(k.clone(), v.clone());
                }
            }
        }
        let mut overlay: Vec<Write> = writes.into_iter().collect();
        if reverse {
            overlay.reverse();
        }
        Ok(LazyRange {
            base: base.peekable(),
            overlay: overlay.into_iter().peekable(),
            reverse,
            failed: false,
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

#[cfg(test)]
mod range_iter_tests {
    use super::*;

    type Bounds<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

    /// A committed store plus two cache layers with sets and deletes that
    /// shadow committed keys, add new ones and delete layered ones.
    fn layered() -> Result<MultiStore, StoreError> {
        let mut m = MultiStore::new();
        m.mount("a", Kind::Iavl)?;
        m.mount("t", Kind::Transient)?;
        for i in 0u8..40 {
            m.set("a", vec![b'k', i], vec![i])?;
        }
        m.flush()?;
        m.commit()?;
        for i in (0u8..40).step_by(3) {
            m.set("a", vec![b'k', i], vec![i, 1])?;
        }
        for i in (1u8..40).step_by(5) {
            m.delete("a", &[b'k', i])?;
        }
        m.set("a", vec![b'j'], vec![9])?;
        m.set("a", vec![b'k', 200], vec![2])?;
        m.branch();
        m.set("a", vec![b'k', 2], vec![2, 2])?;
        m.delete("a", &[b'k', 3])?;
        m.set("a", vec![b'k', 1], vec![1, 2])?;
        m.set("t", vec![b'x'], vec![1])?;
        m.set("t", vec![b'w'], vec![2])?;
        Ok(m)
    }

    /// What `range` must return: the live view of every key, in order.
    fn model(
        m: &MultiStore,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> Result<Entries, StoreError> {
        let mut out = Vec::new();
        for k in (0u8..=255).flat_map(|b| [vec![b'j'], vec![b'k', b], vec![b'w'], vec![b'x']]) {
            if start.is_some_and(|s| k.as_slice() < s) || end.is_some_and(|e| k.as_slice() >= e) {
                continue;
            }
            if let Some(v) = m.get(store, &k)? {
                out.push((k, v));
            }
        }
        out.sort();
        out.dedup();
        if reverse {
            out.reverse();
        }
        Ok(out)
    }

    #[test]
    fn lazy_range_equals_the_merged_view() -> Result<(), StoreError> {
        let m = layered()?;
        let bounds: [Bounds<'_>; 5] = [
            (None, None),
            (Some(b"k"), Some(b"l")),
            (Some(&[b'k', 2]), Some(&[b'k', 30])),
            (Some(&[b'k', 39]), None),
            (Some(b"z"), None),
        ];
        for store in ["a", "t"] {
            for (start, end) in bounds {
                for reverse in [false, true] {
                    let want = model(&m, store, start, end, reverse)?;
                    let got: Entries = m
                        .lazy_range(store, start, end, reverse)?
                        .collect::<Result<_, _>>()?;
                    assert_eq!(got, want, "{store} {start:?}..{end:?} rev {reverse}");
                    assert_eq!(m.range(store, start, end, reverse)?, want);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lazy_range_stops_early() -> Result<(), StoreError> {
        let m = layered()?;
        let mut it = m.lazy_range("a", Some(b"k"), None, false)?;
        assert_eq!(it.next().transpose()?, Some((vec![b'k', 0], vec![0, 1])));
        assert_eq!(it.next().transpose()?, Some((vec![b'k', 1], vec![1, 2])));
        let mut rev = m.lazy_range("a", None, Some(b"k"), true)?;
        assert_eq!(rev.next().transpose()?, Some((vec![b'j'], vec![9])));
        assert_eq!(rev.next().transpose()?, None);
        Ok(())
    }
}
