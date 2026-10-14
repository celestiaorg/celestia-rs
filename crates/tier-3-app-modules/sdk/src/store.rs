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
    layers: Vec<Layer>,
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
            layers: vec![BTreeMap::new()],
        }
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

    /// Returns the kind of a mounted store.
    pub fn kind(&self, name: &str) -> Result<Kind, StoreError> {
        self.kinds
            .get(name)
            .copied()
            .ok_or_else(|| StoreError::UnknownStore(name.to_owned()))
    }

    /// Current branch depth (1 = only the block layer).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Pushes a new cache layer.
    pub fn branch(&mut self) {
        self.layers.push(BTreeMap::new());
    }

    /// Pops the top layer and merges it into the one below (`cache.Write()`).
    pub fn write_branch(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let top = self.layers.pop().unwrap_or_default();
        let below = self.layers.last_mut();
        if let Some(below) = below {
            for (store, writes) in top {
                below.entry(store).or_default().extend(writes);
            }
        }
    }

    /// Pops and discards the top layer.
    pub fn discard_branch(&mut self) {
        if self.layers.len() > 1 {
            self.layers.pop();
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
        for layer in self.layers.iter().rev() {
            if let Some(v) = layer.get(store).and_then(|w| w.get(key)) {
                return Ok(v.clone());
            }
        }
        self.committed_get(store, key)
    }

    /// Writes `key` in the top layer.
    pub fn set(&mut self, store: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
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
        if self.layers.is_empty() {
            self.layers.push(BTreeMap::new());
        }
        let n = self.layers.len() - 1;
        &mut self.layers[n]
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
        for layer in &self.layers {
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
        while self.layers.len() > 1 {
            self.write_branch();
        }
        let block = std::mem::take(&mut self.layers[0]);
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
            if *kind == Kind::Transient {
                if let Some(m) = self.volatile.get_mut(name) {
                    m.clear();
                }
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
