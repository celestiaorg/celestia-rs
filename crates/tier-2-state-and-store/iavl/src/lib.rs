//! Cosmos IAVL+ state tree primitives.
//!
//! See `docs/specs/m2/iavl.md` for the API contract and
//! `docs/adr/0002-persistent-state-storage.md` for the storage design.
//!
//! A [`Tree`] is either purely in memory ([`Tree::new`]) or backed by a
//! [`NodeDb`] ([`Tree::open`]): nodes persisted under their IAVL v1 node key
//! `(version, nonce)`, loaded lazily through a bounded LRU [`NodeCache`].
//! Between commits only the dirty (modified) nodes of the working tree live
//! in memory. Both modes share the hashing code, so root hashes are
//! identical (and match Go IAVL v1).
//!
//! Committed versions stay readable as [`ImmutableTree`]s (get, iterate,
//! ICS-23 proofs, snapshot export) until pruned with [`Tree::prune`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
mod export;
mod mutate;
mod node;
mod nodedb;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use error::{Error, Result};
pub use export::{ExportNode, Exporter, Importer};
pub use kvdb;
pub use node::NodeKey;
pub use nodedb::{CacheStats, NodeCache, NodeDb};

use kvdb::WriteBatch;
use mutate::{Mutator, load};
use node::{Inner, Leaf, Link, Node, StoredRef, encode_inner, encode_leaf, encode_root};
use nodedb::db_err;

/// Byte length of an IAVL root hash.
pub const HASH_SIZE: usize = 32;
const HASH_SIZE_PREFIX: u8 = 32;

/// IAVL root hash bytes.
pub type Hash = [u8; HASH_SIZE];

/// IAVL commit version.
pub type Version = i64;

/// Versioned IAVL tree (in memory or storage backed).
///
/// Cloning is cheap (subtrees are shared); two clones must not both commit
/// into the same [`NodeDb`].
#[derive(Debug, Clone)]
pub struct Tree {
    ndb: Option<Arc<NodeDb>>,
    initial_version: Version,
    version: Version,
    root: Option<Link>,
    dirty: bool,
    /// Persisted nodes replaced in the working tree since the last commit.
    orphans: Vec<NodeKey>,
    /// Committed versions still available, with their roots.
    versions: BTreeMap<Version, Option<Link>>,
}

impl Default for Tree {
    fn default() -> Self {
        Self {
            ndb: None,
            initial_version: 1,
            version: 0,
            root: None,
            dirty: false,
            orphans: Vec::new(),
            versions: BTreeMap::new(),
        }
    }
}

impl Tree {
    /// Create an empty in-memory tree at version zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty in-memory tree with a custom first commit version.
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

    /// Open a storage-backed tree at its latest committed version (an empty
    /// tree at version zero when nothing was committed yet).
    pub fn open(ndb: Arc<NodeDb>) -> Result<Self> {
        let mut versions = BTreeMap::new();
        for (version, root) in ndb.roots()? {
            versions.insert(version, root.map(Link::Stored));
        }
        let (version, root) = versions
            .iter()
            .next_back()
            .map_or((0, None), |(v, r)| (*v, r.clone()));
        Ok(Self {
            ndb: Some(ndb),
            version,
            root,
            versions,
            ..Self::default()
        })
    }

    /// Set the first commit version of a tree that has no versions yet.
    pub fn set_initial_version(&mut self, initial_version: Version) -> Result<()> {
        if initial_version <= 0 {
            return Err(Error::InvalidVersion {
                version: initial_version,
            });
        }
        self.initial_version = initial_version;
        Ok(())
    }

    /// Make committed `version` the working tree, discarding uncommitted
    /// changes. Fails when `version` is unknown or newer versions exist
    /// (their node keys would collide with the next commit).
    pub fn load_version(&mut self, version: Version) -> Result<()> {
        if version == 0 && self.versions.is_empty() {
            self.root = None;
            self.version = 0;
        } else {
            let root = self
                .versions
                .get(&version)
                .ok_or(Error::VersionNotFound { version })?
                .clone();
            if self.versions.range(version + 1..).next().is_some() {
                return Err(Error::InvalidVersion { version });
            }
            self.root = root;
            self.version = version;
        }
        self.dirty = false;
        self.orphans.clear();
        Ok(())
    }

    /// Discard uncommitted changes.
    pub fn rollback(&mut self) {
        self.root = self.versions.get(&self.version).cloned().flatten();
        self.dirty = false;
        self.orphans.clear();
    }

    /// The node store, for storage-backed trees.
    #[must_use]
    pub fn node_db(&self) -> Option<&Arc<NodeDb>> {
        self.ndb.as_ref()
    }

    /// Return the latest saved version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Committed versions still available, ascending.
    #[must_use]
    pub fn available_versions(&self) -> Vec<Version> {
        self.versions.keys().copied().collect()
    }

    /// True when committed `version` is available.
    #[must_use]
    pub fn version_exists(&self, version: Version) -> bool {
        self.versions.contains_key(&version)
    }

    /// Return the number of key/value entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.root
            .as_ref()
            .map_or(0, |root| usize::try_from(root.size()).unwrap_or(0))
    }

    /// Return true when the tree contains no key/value entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// True when there are uncommitted changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn view(&self) -> View<'_> {
        View {
            ndb: self.ndb.as_ref(),
            root: self.root.as_ref(),
            version: self.hash_version(),
        }
    }

    /// Return the deterministic root hash for the current tree contents.
    #[must_use]
    pub fn root_hash(&self) -> Hash {
        self.view().root_hash()
    }

    /// Return the value for `key`, if present.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.view().get(key)
    }

    /// Insert or replace a key/value pair, returning the previous value.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let mut mutator = Mutator {
            ndb: self.ndb.as_deref(),
            orphans: &mut self.orphans,
        };
        let (root, previous) = match &self.root {
            Some(root) => {
                let (root, _, previous) = mutator.set(root, key, value)?;
                (root, previous)
            }
            None => (Link::mem(Node::leaf(key, value)), None),
        };
        self.root = Some(root);
        self.dirty = true;
        Ok(previous)
    }

    /// Remove a key/value pair, returning the previous value.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let Some(root) = &self.root else {
            return Ok(None);
        };
        let mut mutator = Mutator {
            ndb: self.ndb.as_deref(),
            orphans: &mut self.orphans,
        };
        let removed = mutator.remove(root, key)?;
        if !removed.removed {
            return Ok(None);
        }
        self.root = removed.node;
        self.dirty = true;
        Ok(removed.value)
    }

    /// Commit the current working tree as the next version. Storage-backed
    /// trees write the new nodes in one backend batch.
    pub fn save_version(&mut self) -> Result<CommitInfo> {
        let mut batch = WriteBatch::new();
        let info = self.save_version_to(&mut batch)?;
        if let Some(ndb) = &self.ndb {
            ndb.backend().write(batch).map_err(db_err)?;
        }
        Ok(info)
    }

    /// Commit the working tree as the next version, queueing the storage
    /// writes (new nodes, root record, orphan records) into `batch` instead
    /// of writing them. The caller must write `batch` before reading the new
    /// version back from storage. In-memory trees leave `batch` untouched.
    pub fn save_version_to(&mut self, batch: &mut WriteBatch) -> Result<CommitInfo> {
        let next_version = self.next_version()?;
        let root = match self.root.take() {
            None => None,
            Some(root) => Some(match &self.ndb {
                Some(ndb) => {
                    let mut nonce = 0u32;
                    Link::Stored(persist(ndb, root, next_version, &mut nonce, batch)?)
                }
                None => assign_hashes(root, next_version),
            }),
        };
        if let Some(ndb) = &self.ndb {
            let root_ref = match &root {
                Some(Link::Stored(r)) => Some(*r),
                _ => None,
            };
            batch.put(ndb.root_key(next_version), encode_root(root_ref.as_ref()));
            self.orphans.sort_unstable();
            self.orphans.dedup();
            for nk in self.orphans.drain(..) {
                batch.put(ndb.orphan_key(self.version, nk), Vec::new());
            }
        }
        let root_hash = root
            .as_ref()
            .map_or_else(|| hash::sha256(&[]), |root| root.hash_for(next_version));
        self.root = root;
        self.versions.insert(next_version, self.root.clone());
        self.version = next_version;
        self.dirty = false;
        self.orphans.clear();
        Ok(CommitInfo {
            version: self.version,
            root_hash,
            size: self.len(),
        })
    }

    /// Delete every committed version (except the latest) for which
    /// `keep(version)` is false, queueing the storage deletions into
    /// `batch`. Returns the deleted versions.
    ///
    /// Nodes are deleted only when no retained version still references
    /// them, so retained versions keep their exact hashes.
    pub fn prune(
        &mut self,
        keep: impl Fn(Version) -> bool,
        batch: &mut WriteBatch,
    ) -> Result<Vec<Version>> {
        let latest = self.version;
        let delete: BTreeSet<Version> = self
            .versions
            .keys()
            .copied()
            .filter(|v| *v != latest && !keep(*v))
            .collect();
        if delete.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(ndb) = &self.ndb {
            let retained: BTreeSet<Version> = self
                .versions
                .keys()
                .copied()
                .filter(|v| !delete.contains(v))
                .collect();
            for &d in &delete {
                batch.delete(ndb.root_key(d));
                for nk in ndb.orphans(d)? {
                    batch.delete(ndb.orphan_key(d, nk));
                    match retained.range(nk.version..d).next_back() {
                        Some(&t) => batch.put(ndb.orphan_key(t, nk), Vec::new()),
                        None => ndb.delete_node(batch, nk),
                    }
                }
            }
        }
        for d in &delete {
            self.versions.remove(d);
        }
        Ok(delete.into_iter().collect())
    }

    /// Deletes every committed version above `target` and makes `target`
    /// the working tree (Go `LoadVersionForOverwriting`, used by
    /// `rollback`), queueing the storage deletions into `batch`: the roots
    /// and nodes created by the deleted versions, and the orphan records
    /// their commits wrote (those nodes are live again in `target`).
    pub fn rollback_to(&mut self, target: Version, batch: &mut WriteBatch) -> Result<()> {
        if target != 0 && !self.versions.contains_key(&target) {
            return Err(Error::VersionNotFound { version: target });
        }
        let doomed: Vec<Version> = self.versions.range(target + 1..).map(|(v, _)| *v).collect();
        if let Some(ndb) = &self.ndb {
            for &d in &doomed {
                batch.delete(ndb.root_key(d));
                for nk in ndb.node_keys_at(d)? {
                    ndb.delete_node(batch, nk);
                }
                // Orphans written by the commit of `d` are keyed by the
                // version before it.
                let prev = self.versions.range(..d).next_back().map_or(0, |(v, _)| *v);
                for nk in ndb.orphans(prev)? {
                    batch.delete(ndb.orphan_key(prev, nk));
                }
            }
        }
        for d in &doomed {
            self.versions.remove(d);
        }
        self.version = target;
        self.root = self.versions.get(&target).cloned().flatten();
        self.dirty = false;
        self.orphans.clear();
        Ok(())
    }

    /// An immutable view of committed `version`.
    pub fn get_immutable(&self, version: Version) -> Result<ImmutableTree> {
        let root = self
            .versions
            .get(&version)
            .ok_or(Error::VersionNotFound { version })?
            .clone();
        Ok(ImmutableTree {
            ndb: self.ndb.clone(),
            root,
            version,
        })
    }

    /// A read-only tree positioned at committed `version` (versions after
    /// it are hidden). Used for historical queries; the result must never
    /// be committed.
    pub fn at_version(&self, version: Version) -> Result<Tree> {
        let root = self
            .versions
            .get(&version)
            .ok_or(Error::VersionNotFound { version })?
            .clone();
        Ok(Tree {
            ndb: self.ndb.clone(),
            initial_version: self.initial_version,
            version,
            root,
            dirty: false,
            orphans: Vec::new(),
            versions: self
                .versions
                .range(..=version)
                .map(|(v, r)| (*v, r.clone()))
                .collect(),
        })
    }

    /// Export committed `version` as IAVL export nodes in post-order.
    pub fn export(&self, version: Version) -> Result<Exporter> {
        self.get_immutable(version)?.export()
    }

    /// Iterate forward over the half-open range `[start, end)`.
    #[must_use]
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Iter {
        self.view().iter(start, end, false)
    }

    /// Iterate backward over the half-open range `[start, end)`.
    #[must_use]
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Iter {
        self.view().iter(start, end, true)
    }

    /// Return membership or non-membership proof material for `key`.
    pub fn get_with_proof(&self, key: &[u8]) -> Result<Option<Proof>> {
        self.view().get_with_proof(key)
    }

    /// Verify an IAVL membership proof.
    pub fn verify_membership(root: Hash, proof: &Proof, key: &[u8], value: &[u8]) -> Result<()> {
        if !proof.is_membership() {
            return Err(Error::InvalidProof("membership proof mismatch"));
        }
        proof.validate_membership_ops()?;
        if proof.key() == key
            && proof.value() == Some(value)
            && proof.root_hash == root
            && proof.computed_root()? == root
        {
            return Ok(());
        }
        Err(Error::InvalidProof("membership proof mismatch"))
    }

    /// Verify an IAVL non-membership proof.
    pub fn verify_non_membership(root: Hash, proof: &Proof, key: &[u8]) -> Result<()> {
        if proof.is_membership() {
            return Err(Error::InvalidProof("non-membership proof mismatch"));
        }
        proof.validate_non_membership_ops()?;
        if proof.key() == key
            && proof.root_hash == root
            && proof.non_membership_root_matches(root)?
        {
            return Ok(());
        }
        Err(Error::InvalidProof("non-membership proof mismatch"))
    }

    fn hash_version(&self) -> Version {
        if self.dirty {
            self.next_version()
                .unwrap_or_else(|_| self.version.saturating_add(1).max(1))
        } else {
            self.version.max(1)
        }
    }

    fn next_version(&self) -> Result<Version> {
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
        Ok(next_version)
    }

    pub(crate) fn from_import(
        ndb: Option<Arc<NodeDb>>,
        version: Version,
        root: Option<Link>,
    ) -> Self {
        let mut versions = BTreeMap::new();
        versions.insert(version, root.clone());
        Self {
            ndb,
            version,
            root,
            versions,
            ..Self::default()
        }
    }
}

/// Writes the dirty part of `link` (post-order) at `version`.
fn persist(
    ndb: &NodeDb,
    link: Link,
    version: Version,
    nonce: &mut u32,
    batch: &mut WriteBatch,
) -> Result<StoredRef> {
    let arc = match link {
        Link::Stored(r) => return Ok(r),
        Link::Mem(arc) => arc,
    };
    if arc.saved_version().is_some() {
        return Err(Error::Corrupt(
            "saved in-memory node in a storage-backed tree",
        ));
    }
    let node = Arc::try_unwrap(arc).unwrap_or_else(|arc| (*arc).clone());
    let (r, bytes, saved) = match node {
        Node::Leaf(leaf) => {
            let hash = leaf_hash(version, &leaf.key, &leaf.value);
            *nonce += 1;
            let r = StoredRef {
                nk: NodeKey {
                    version,
                    nonce: *nonce,
                },
                hash,
                height: 0,
                size: 1,
            };
            let bytes = encode_leaf(&leaf.key, &leaf.value);
            let saved = Node::Leaf(Leaf {
                version,
                hash: Some(hash),
                ..leaf
            });
            (r, bytes, saved)
        }
        Node::Inner(inner) => {
            let left = persist(ndb, inner.left, version, nonce, batch)?;
            let right = persist(ndb, inner.right, version, nonce, batch)?;
            let hash = inner_hash(
                i64::from(inner.height),
                inner.size,
                version,
                left.hash,
                right.hash,
            );
            *nonce += 1;
            let r = StoredRef {
                nk: NodeKey {
                    version,
                    nonce: *nonce,
                },
                hash,
                height: inner.height,
                size: inner.size,
            };
            let bytes = encode_inner(&inner.key, inner.height, inner.size, &left, &right);
            let saved = Node::Inner(Inner {
                key: inner.key,
                height: inner.height,
                size: inner.size,
                left: Link::Stored(left),
                right: Link::Stored(right),
                version,
                hash: Some(hash),
            });
            (r, bytes, saved)
        }
    };
    ndb.put_node(batch, r.nk, bytes, Arc::new(saved));
    Ok(r)
}

/// In-memory commit: hash and version every dirty node of `link`.
fn assign_hashes(link: Link, version: Version) -> Link {
    let arc = match link {
        Link::Mem(arc) if arc.saved_hash().is_none() => arc,
        other => return other,
    };
    let node = Arc::try_unwrap(arc).unwrap_or_else(|arc| (*arc).clone());
    Link::mem(match node {
        Node::Leaf(leaf) => {
            let hash = leaf_hash(version, &leaf.key, &leaf.value);
            Node::Leaf(Leaf {
                version,
                hash: Some(hash),
                ..leaf
            })
        }
        Node::Inner(inner) => {
            let left = assign_hashes(inner.left, version);
            let right = assign_hashes(inner.right, version);
            let hash = inner_hash(
                i64::from(inner.height),
                inner.size,
                version,
                left.hash_for(version),
                right.hash_for(version),
            );
            Node::Inner(Inner {
                left,
                right,
                version,
                hash: Some(hash),
                ..inner
            })
        }
    })
}

/// Read-only snapshot of one committed version.
#[derive(Debug, Clone)]
pub struct ImmutableTree {
    ndb: Option<Arc<NodeDb>>,
    root: Option<Link>,
    version: Version,
}

impl ImmutableTree {
    fn view(&self) -> View<'_> {
        View {
            ndb: self.ndb.as_ref(),
            root: self.root.as_ref(),
            version: self.version.max(1),
        }
    }

    /// The version of this snapshot.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Root hash at this version.
    #[must_use]
    pub fn root_hash(&self) -> Hash {
        self.view().root_hash()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.root
            .as_ref()
            .map_or(0, |root| usize::try_from(root.size()).unwrap_or(0))
    }

    /// True when the tree is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Value of `key` at this version.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.view().get(key)
    }

    /// Iterate forward over `[start, end)`.
    #[must_use]
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Iter {
        self.view().iter(start, end, false)
    }

    /// Iterate backward over `[start, end)`.
    #[must_use]
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Iter {
        self.view().iter(start, end, true)
    }

    /// Membership or non-membership proof for `key` at this version.
    pub fn get_with_proof(&self, key: &[u8]) -> Result<Option<Proof>> {
        self.view().get_with_proof(key)
    }

    /// Export nodes in post-order (the SDK snapshot order).
    pub fn export(&self) -> Result<Exporter> {
        Exporter::new(self.ndb.clone(), self.root.as_ref())
    }
}

/// Read access to one root.
struct View<'a> {
    ndb: Option<&'a Arc<NodeDb>>,
    root: Option<&'a Link>,
    /// Version dirty nodes are hashed at.
    version: Version,
}

impl View<'_> {
    fn load(&self, link: &Link) -> Result<Arc<Node>> {
        load(self.ndb.map(AsRef::as_ref), link)
    }

    fn root_hash(&self) -> Hash {
        self.root
            .map_or_else(|| hash::sha256(&[]), |root| root.hash_for(self.version))
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let Some(mut link) = self.root.cloned() else {
            return Ok(None);
        };
        loop {
            let node = self.load(&link)?;
            match &*node {
                Node::Leaf(leaf) => {
                    return Ok((leaf.key.as_slice() == key).then(|| leaf.value.clone()));
                }
                Node::Inner(inner) => {
                    link = if key < inner.key.as_slice() {
                        inner.left.clone()
                    } else {
                        inner.right.clone()
                    };
                }
            }
        }
    }

    fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Iter {
        let invalid_range = matches!((start, end), (Some(start), Some(end)) if start > end);
        Iter {
            ndb: self.ndb.cloned(),
            stack: if invalid_range {
                Vec::new()
            } else {
                self.root.cloned().into_iter().collect()
            },
            start: start.map(<[u8]>::to_vec),
            end: end.map(<[u8]>::to_vec),
            reverse,
        }
    }

    fn get_with_proof(&self, key: &[u8]) -> Result<Option<Proof>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        if self.root.is_none() {
            return Ok(None);
        }
        Ok(Some(match self.get(key)? {
            Some(value) => self.membership_proof(key, &value)?,
            None => self.non_membership_proof(key)?,
        }))
    }

    fn membership_proof(&self, key: &[u8], value: &[u8]) -> Result<Proof> {
        let existence = self
            .existence_proof(key)?
            .ok_or(Error::InvalidProof("missing membership proof path"))?;
        let data = commitment_existence_proto(&existence);
        Ok(Proof {
            key: key.to_vec(),
            value: Some(value.to_vec()),
            ops: vec![ProofOp {
                field_type: ProofOpType::IavlValue,
                key: key.to_vec(),
                data,
            }],
            root_hash: self.root_hash(),
            kind: ProofKind::Membership(existence),
        })
    }

    fn non_membership_proof(&self, key: &[u8]) -> Result<Proof> {
        let left_key = self
            .iter(None, Some(key), true)
            .next()
            .transpose()?
            .map(|entry| entry.key);
        let right_key = self
            .iter(Some(key), None, false)
            .next()
            .transpose()?
            .map(|entry| entry.key);
        let left = match left_key.as_deref() {
            Some(left) => Some(
                self.existence_proof(left)?
                    .ok_or(Error::InvalidProof("missing left non-membership neighbor"))?,
            ),
            None => None,
        };
        let right = match right_key.as_deref() {
            Some(right) => Some(
                self.existence_proof(right)?
                    .ok_or(Error::InvalidProof("missing right non-membership neighbor"))?,
            ),
            None => None,
        };
        let data = commitment_nonexistence_proto(key, left.as_ref(), right.as_ref());
        Ok(Proof {
            key: key.to_vec(),
            value: None,
            ops: vec![ProofOp {
                field_type: ProofOpType::IavlAbsence,
                key: key.to_vec(),
                data,
            }],
            root_hash: self.root_hash(),
            kind: ProofKind::NonMembership { left, right },
        })
    }

    fn existence_proof(&self, key: &[u8]) -> Result<Option<ExistenceProof>> {
        let Some(mut link) = self.root.cloned() else {
            return Ok(None);
        };
        let mut path = Vec::new();
        loop {
            let node = self.load(&link)?;
            match &*node {
                Node::Leaf(leaf) => {
                    if leaf.key.as_slice() != key {
                        return Ok(None);
                    }
                    return Ok(Some(ExistenceProof {
                        key: leaf.key.clone(),
                        value: leaf.value.clone(),
                        leaf_version: node.saved_version().unwrap_or(self.version),
                        path,
                    }));
                }
                Node::Inner(inner) => {
                    let version = node.saved_version().unwrap_or(self.version);
                    if key < inner.key.as_slice() {
                        path.push(ProofInner {
                            height: inner.height,
                            size: inner.size,
                            version,
                            left: None,
                            right: Some(inner.right.hash_for(self.version)),
                        });
                        link = inner.left.clone();
                    } else {
                        path.push(ProofInner {
                            height: inner.height,
                            size: inner.size,
                            version,
                            left: Some(inner.left.hash_for(self.version)),
                            right: None,
                        });
                        link = inner.right.clone();
                    }
                }
            }
        }
    }
}

/// Result returned by `Tree::save_version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Committed version.
    pub version: Version,
    /// Root hash at the committed version.
    pub root_hash: Hash,
    /// Number of entries in the committed tree.
    pub size: usize,
}

/// Owned key/value iterator entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Entry key.
    pub key: Vec<u8>,
    /// Entry value.
    pub value: Vec<u8>,
}

/// Lazy in-order range iterator. Loads only the nodes on the iterated
/// path; yields an error (and stops) when a node cannot be loaded.
#[derive(Debug)]
pub struct Iter {
    ndb: Option<Arc<NodeDb>>,
    stack: Vec<Link>,
    start: Option<Vec<u8>>,
    end: Option<Vec<u8>>,
    reverse: bool,
}

impl Iterator for Iter {
    type Item = Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(link) = self.stack.pop() {
            let node = match load(self.ndb.as_deref(), &link) {
                Ok(node) => node,
                Err(err) => {
                    self.stack.clear();
                    return Some(Err(err));
                }
            };
            match &*node {
                Node::Leaf(leaf) => {
                    let below = self
                        .start
                        .as_deref()
                        .is_some_and(|start| leaf.key.as_slice() < start);
                    let above = self
                        .end
                        .as_deref()
                        .is_some_and(|end| leaf.key.as_slice() >= end);
                    if below || above {
                        continue;
                    }
                    return Some(Ok(Entry {
                        key: leaf.key.clone(),
                        value: leaf.value.clone(),
                    }));
                }
                Node::Inner(inner) => {
                    // Left subtree: keys < inner.key; right subtree: keys >= inner.key.
                    let want_left = self
                        .start
                        .as_deref()
                        .is_none_or(|start| start < inner.key.as_slice());
                    let want_right = self
                        .end
                        .as_deref()
                        .is_none_or(|end| end > inner.key.as_slice());
                    if self.reverse {
                        if want_left {
                            self.stack.push(inner.left.clone());
                        }
                        if want_right {
                            self.stack.push(inner.right.clone());
                        }
                    } else {
                        if want_right {
                            self.stack.push(inner.right.clone());
                        }
                        if want_left {
                            self.stack.push(inner.left.clone());
                        }
                    }
                }
            }
        }
        None
    }
}

/// IAVL proof material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    key: Vec<u8>,
    value: Option<Vec<u8>>,
    ops: Vec<ProofOp>,
    root_hash: Hash,
    kind: ProofKind,
}

impl Proof {
    /// Return the key this proof speaks about.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Return the proven value for membership proofs.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Return true when this is a membership proof.
    #[must_use]
    pub fn is_membership(&self) -> bool {
        self.value.is_some()
    }

    /// Return proof operations in verification order.
    #[must_use]
    pub fn ops(&self) -> &[ProofOp] {
        &self.ops
    }

    fn computed_root(&self) -> Result<Hash> {
        match &self.kind {
            ProofKind::Membership(existence) => Ok(existence.compute_root()),
            ProofKind::NonMembership { .. } => {
                Err(Error::InvalidProof("expected membership proof"))
            }
        }
    }

    fn validate_membership_ops(&self) -> Result<()> {
        let ProofKind::Membership(existence) = &self.kind else {
            return Err(Error::InvalidProof("expected membership proof"));
        };
        self.validate_single_op(
            ProofOpType::IavlValue,
            &self.key,
            &commitment_existence_proto(existence),
        )
    }

    fn validate_non_membership_ops(&self) -> Result<()> {
        let ProofKind::NonMembership { left, right } = &self.kind else {
            return Err(Error::InvalidProof("expected non-membership proof"));
        };
        self.validate_single_op(
            ProofOpType::IavlAbsence,
            &self.key,
            &commitment_nonexistence_proto(&self.key, left.as_ref(), right.as_ref()),
        )
    }

    fn validate_single_op(&self, field_type: ProofOpType, key: &[u8], data: &[u8]) -> Result<()> {
        let [op] = self.ops.as_slice() else {
            return Err(Error::InvalidProof("expected exactly one proof operation"));
        };
        if op.field_type != field_type {
            return Err(Error::InvalidProof("proof operation type mismatch"));
        }
        if op.key != key {
            return Err(Error::InvalidProof("proof operation key mismatch"));
        }
        if op.data != data {
            return Err(Error::InvalidProof("proof operation data mismatch"));
        }
        Ok(())
    }

    fn non_membership_root_matches(&self, root: Hash) -> Result<bool> {
        let ProofKind::NonMembership { left, right } = &self.kind else {
            return Err(Error::InvalidProof("expected non-membership proof"));
        };
        let left_ok = left.as_ref().is_none_or(|left| {
            left.key.as_slice() < self.key.as_slice() && left.compute_root() == root
        });
        let right_ok = right.as_ref().is_none_or(|right| {
            right.key.as_slice() > self.key.as_slice() && right.compute_root() == root
        });
        Ok(left_ok && right_ok && (left.is_some() || right.is_some()))
    }
}

/// IAVL proof operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    field_type: ProofOpType,
    key: Vec<u8>,
    data: Vec<u8>,
}

impl ProofOp {
    /// Return the proof operation type.
    #[must_use]
    pub fn field_type(&self) -> ProofOpType {
        self.field_type
    }

    /// Return operation key bytes.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Return operation payload bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// IAVL proof operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofOpType {
    /// Value proof operation.
    IavlValue,
    /// Absence proof operation.
    IavlAbsence,
    /// Commitment proof operation.
    IavlCommitment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProofKind {
    Membership(ExistenceProof),
    NonMembership {
        left: Option<ExistenceProof>,
        right: Option<ExistenceProof>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExistenceProof {
    key: Vec<u8>,
    value: Vec<u8>,
    leaf_version: Version,
    path: Vec<ProofInner>,
}

impl ExistenceProof {
    fn compute_root(&self) -> Hash {
        let mut hash = leaf_hash(self.leaf_version, &self.key, &self.value);
        for inner in self.path.iter().rev() {
            hash = match (inner.left, inner.right) {
                (Some(left), None) => inner_hash(
                    i64::from(inner.height),
                    inner.size,
                    inner.version,
                    left,
                    hash,
                ),
                (None, Some(right)) => inner_hash(
                    i64::from(inner.height),
                    inner.size,
                    inner.version,
                    hash,
                    right,
                ),
                _ => hash,
            };
        }
        hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProofInner {
    height: i8,
    size: i64,
    version: Version,
    left: Option<Hash>,
    right: Option<Hash>,
}

fn commitment_existence_proto(proof: &ExistenceProof) -> Vec<u8> {
    let exist = existence_proto(proof);
    let mut out = Vec::new();
    proto_bytes_field(&mut out, 1, &exist);
    out
}

fn commitment_nonexistence_proto(
    key: &[u8],
    left: Option<&ExistenceProof>,
    right: Option<&ExistenceProof>,
) -> Vec<u8> {
    let mut nonexist = Vec::new();
    proto_bytes_field(&mut nonexist, 1, key);
    if let Some(left) = left {
        proto_bytes_field(&mut nonexist, 2, &existence_proto(left));
    }
    if let Some(right) = right {
        proto_bytes_field(&mut nonexist, 3, &existence_proto(right));
    }

    let mut out = Vec::new();
    proto_bytes_field(&mut out, 2, &nonexist);
    out
}

fn existence_proto(proof: &ExistenceProof) -> Vec<u8> {
    let mut out = Vec::new();
    proto_bytes_field(&mut out, 1, &proof.key);
    proto_bytes_field(&mut out, 2, &proof.value);
    proto_bytes_field(&mut out, 3, &leaf_op_proto(proof.leaf_version));
    for inner in proof.path.iter().rev() {
        proto_bytes_field(&mut out, 4, &inner_op_proto(inner));
    }
    out
}

fn leaf_op_proto(version: Version) -> Vec<u8> {
    let mut prefix = Vec::new();
    encode_varint(&mut prefix, 0);
    encode_varint(&mut prefix, 1);
    encode_varint(&mut prefix, version);

    let mut out = Vec::new();
    proto_varint_field(&mut out, 1, 1);
    proto_varint_field(&mut out, 3, 1);
    proto_varint_field(&mut out, 4, 1);
    proto_bytes_field(&mut out, 5, &prefix);
    out
}

fn inner_op_proto(inner: &ProofInner) -> Vec<u8> {
    let mut prefix = Vec::new();
    encode_varint(&mut prefix, i64::from(inner.height));
    encode_varint(&mut prefix, inner.size);
    encode_varint(&mut prefix, inner.version);

    let mut suffix = Vec::new();
    if let Some(left) = inner.left {
        prefix.push(HASH_SIZE_PREFIX);
        prefix.extend_from_slice(&left);
        prefix.push(HASH_SIZE_PREFIX);
    } else if let Some(right) = inner.right {
        prefix.push(HASH_SIZE_PREFIX);
        suffix.push(HASH_SIZE_PREFIX);
        suffix.extend_from_slice(&right);
    }

    let mut out = Vec::new();
    proto_varint_field(&mut out, 1, 1);
    proto_bytes_field(&mut out, 2, &prefix);
    proto_bytes_field(&mut out, 3, &suffix);
    out
}

fn proto_varint_field(out: &mut Vec<u8>, field_number: u64, value: u64) {
    encode_uvarint(out, field_number << 3);
    encode_uvarint(out, value);
}

fn proto_bytes_field(out: &mut Vec<u8>, field_number: u64, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    encode_uvarint(out, (field_number << 3) | 2);
    encode_uvarint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

pub(crate) fn leaf_hash(version: Version, key: &[u8], value: &[u8]) -> Hash {
    node_hash(0, 1, version, Some((key, value)), None)
}

pub(crate) fn inner_hash(
    height: i64,
    size: i64,
    version: Version,
    left: Hash,
    right: Hash,
) -> Hash {
    node_hash(height, size, version, None, Some((left, right)))
}

fn node_hash(
    height: i64,
    size: i64,
    version: Version,
    leaf: Option<(&[u8], &[u8])>,
    children: Option<(Hash, Hash)>,
) -> Hash {
    let mut bytes = Vec::new();
    encode_varint(&mut bytes, height);
    encode_varint(&mut bytes, size);
    encode_varint(&mut bytes, version);
    if let Some((left, right)) = children {
        encode_hash(&mut bytes, &left);
        encode_hash(&mut bytes, &right);
    } else if let Some((key, value)) = leaf {
        encode_bytes(&mut bytes, key);
        let value_hash = hash::sha256(value);
        encode_hash(&mut bytes, &value_hash);
    }
    hash::sha256(&bytes)
}

fn encode_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    encode_uvarint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn encode_hash(out: &mut Vec<u8>, bytes: &Hash) {
    encode_uvarint(out, HASH_SIZE as u64);
    out.extend_from_slice(bytes);
}

fn encode_varint(out: &mut Vec<u8>, value: i64) {
    debug_assert!(value >= 0);
    let ux = u64::try_from(value).unwrap_or(0) << 1;
    encode_uvarint(out, ux);
}

#[allow(clippy::cast_possible_truncation)]
fn encode_uvarint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::similar_names,
        clippy::type_complexity
    )]

    use std::sync::Arc;

    use super::{Error, ImmutableTree, NodeDb, ProofOpType, Tree, Version};
    use kvdb::WriteBatch;

    fn entries(iter: super::Iter) -> Vec<(Vec<u8>, Vec<u8>)> {
        iter.map(|e| {
            let e = e.unwrap();
            (e.key, e.value)
        })
        .collect()
    }

    #[test]
    fn empty_tree_matches_iavl_empty_hash() {
        let tree = Tree::new();

        assert_eq!(
            hex::encode(tree.root_hash()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn set_get_replace_remove_and_commit_work() -> Result<(), Error> {
        let mut tree = Tree::new();

        assert_eq!(tree.set(b"b".to_vec(), b"two".to_vec())?, None);
        assert_eq!(tree.set(b"a".to_vec(), b"one".to_vec())?, None);
        assert_eq!(
            tree.set(b"b".to_vec(), b"two-updated".to_vec())?,
            Some(b"two".to_vec())
        );
        assert_eq!(tree.get(b"a")?, Some(b"one".to_vec()));
        assert_eq!(tree.get(b"b")?, Some(b"two-updated".to_vec()));

        let commit = tree.save_version()?;
        assert_eq!(commit.version, 1);
        assert_eq!(commit.size, 2);
        assert_eq!(tree.version(), 1);
        assert!(!tree.root_hash().iter().all(|byte| *byte == 0));

        assert_eq!(tree.remove(b"a")?, Some(b"one".to_vec()));
        assert_eq!(tree.remove(b"missing")?, None);
        assert_eq!(tree.get(b"a")?, None);

        Ok(())
    }

    #[test]
    fn custom_initial_version_controls_first_hash_version() -> Result<(), Error> {
        let mut tree = Tree::with_initial_version(7)?;
        tree.set(b"k".to_vec(), b"v".to_vec())?;
        let working_hash = tree.root_hash();
        let commit = tree.save_version()?;
        assert_eq!(commit.version, 7);
        assert_eq!(commit.root_hash, working_hash);

        let mut default_tree = Tree::new();
        default_tree.set(b"k".to_vec(), b"v".to_vec())?;
        assert_ne!(commit.root_hash, default_tree.save_version()?.root_hash);

        tree.set(b"k".to_vec(), b"v2".to_vec())?;
        assert_eq!(tree.save_version()?.version, 8);
        assert!(matches!(
            Tree::with_initial_version(0),
            Err(Error::InvalidVersion { version: 0 })
        ));
        Ok(())
    }

    #[test]
    fn mutations_reject_empty_keys() {
        let mut tree = Tree::new();

        assert_eq!(
            tree.set(Vec::new(), b"value".to_vec()),
            Err(Error::EmptyKey)
        );
        assert_eq!(tree.remove(b""), Err(Error::EmptyKey));
        assert_eq!(tree.get_with_proof(b""), Err(Error::EmptyKey));
    }

    #[test]
    fn length_empty_invalid_versions_and_errors_are_reported() -> Result<(), Error> {
        let mut tree = Tree::new();
        assert_eq!(tree.len(), 0);
        assert!(tree.is_empty());

        tree.set(b"a".to_vec(), b"1".to_vec())?;
        assert_eq!(tree.len(), 1);
        assert!(!tree.is_empty());

        tree.version = Version::MAX;
        assert_eq!(
            tree.save_version(),
            Err(Error::InvalidVersion {
                version: Version::MAX
            })
        );
        tree.version = -1;
        assert_eq!(
            tree.save_version(),
            Err(Error::InvalidVersion { version: 0 })
        );

        assert_eq!(Error::EmptyKey.to_string(), "empty key");
        assert_eq!(
            Error::InvalidVersion { version: 7 }.to_string(),
            "invalid version 7"
        );
        assert_eq!(Error::InvalidRange.to_string(), "invalid range");
        assert_eq!(Error::InvalidProof("bad").to_string(), "invalid proof: bad");
        assert_eq!(
            Error::MalformedNode("bad").to_string(),
            "malformed node: bad"
        );

        Ok(())
    }

    #[test]
    fn iterators_respect_half_open_ranges() -> Result<(), Error> {
        let mut tree = Tree::new();
        for (key, value) in [(b"a", b"1"), (b"b", b"2"), (b"c", b"3"), (b"d", b"4")] {
            tree.set(key.to_vec(), value.to_vec())?;
        }

        assert_eq!(
            entries(tree.iter(Some(b"b"), Some(b"d"))),
            vec![
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec())
            ]
        );
        assert_eq!(
            entries(tree.reverse_iter(Some(b"b"), Some(b"d"))),
            vec![
                (b"c".to_vec(), b"3".to_vec()),
                (b"b".to_vec(), b"2".to_vec())
            ]
        );
        assert_eq!(tree.iter(Some(b"d"), Some(b"a")).count(), 0);
        assert_eq!(tree.iter(Some(b"b"), Some(b"b")).count(), 0);

        Ok(())
    }

    #[test]
    fn membership_proofs_round_trip_key_value_and_root() -> Result<(), Error> {
        let mut tree = Tree::new();
        tree.set(b"key".to_vec(), b"value".to_vec())?;
        let root = tree.save_version()?.root_hash;

        let proof = tree
            .get_with_proof(b"key")?
            .ok_or(Error::InvalidProof("missing proof"))?;
        assert_eq!(proof.ops()[0].field_type(), ProofOpType::IavlValue);
        assert_eq!(proof.ops()[0].key(), b"key");
        assert!(!proof.ops()[0].data().is_empty());
        Tree::verify_membership(root, &proof, b"key", b"value")?;
        assert!(Tree::verify_membership(root, &proof, b"key", b"other").is_err());

        Ok(())
    }

    #[test]
    fn non_membership_proofs_round_trip_neighbors_and_ops() -> Result<(), Error> {
        let mut tree = Tree::new();
        tree.set(b"a".to_vec(), b"1".to_vec())?;
        tree.set(b"c".to_vec(), b"3".to_vec())?;
        let root = tree.save_version()?.root_hash;

        let proof = tree
            .get_with_proof(b"b")?
            .ok_or(Error::InvalidProof("missing proof"))?;

        assert!(!proof.is_membership());
        assert_eq!(proof.value(), None);
        assert_eq!(proof.ops()[0].field_type(), ProofOpType::IavlAbsence);
        assert_eq!(proof.ops()[0].key(), b"b");
        assert!(!proof.ops()[0].data().is_empty());
        Tree::verify_non_membership(root, &proof, b"b")?;
        assert!(Tree::verify_non_membership(root, &proof, b"a").is_err());

        Ok(())
    }

    #[test]
    fn proof_verification_rejects_malformed_operation_material() -> Result<(), Error> {
        let mut tree = Tree::new();
        tree.set(b"a".to_vec(), b"1".to_vec())?;
        tree.set(b"c".to_vec(), b"3".to_vec())?;
        let root = tree.save_version()?.root_hash;

        let membership = tree
            .get_with_proof(b"a")?
            .ok_or(Error::InvalidProof("missing proof"))?;
        let mut missing_op = membership.clone();
        missing_op.ops.clear();
        assert_eq!(
            Tree::verify_membership(root, &missing_op, b"a", b"1"),
            Err(Error::InvalidProof("expected exactly one proof operation"))
        );

        let mut wrong_type = membership.clone();
        wrong_type.ops[0].field_type = ProofOpType::IavlAbsence;
        assert_eq!(
            Tree::verify_membership(root, &wrong_type, b"a", b"1"),
            Err(Error::InvalidProof("proof operation type mismatch"))
        );

        let mut wrong_key = membership.clone();
        wrong_key.ops[0].key = b"b".to_vec();
        assert_eq!(
            Tree::verify_membership(root, &wrong_key, b"a", b"1"),
            Err(Error::InvalidProof("proof operation key mismatch"))
        );

        let mut wrong_data = membership;
        wrong_data.ops[0].data.push(0);
        assert_eq!(
            Tree::verify_membership(root, &wrong_data, b"a", b"1"),
            Err(Error::InvalidProof("proof operation data mismatch"))
        );

        let absence = tree
            .get_with_proof(b"b")?
            .ok_or(Error::InvalidProof("missing proof"))?;
        let mut wrong_absence_data = absence;
        wrong_absence_data.ops[0].data.clear();
        assert_eq!(
            Tree::verify_non_membership(root, &wrong_absence_data, b"b"),
            Err(Error::InvalidProof("proof operation data mismatch"))
        );

        Ok(())
    }

    #[test]
    fn avl_rotations_and_removals_cover_edge_paths() -> Result<(), Error> {
        let mut left_right = Tree::new();
        for key in [b"c", b"a", b"b"] {
            left_right.set(key.to_vec(), key.to_vec())?;
        }
        assert_ne!(left_right.root_hash(), [0; 32]);
        assert_eq!(left_right.save_version()?.size, 3);

        let mut right_left = Tree::new();
        for key in [b"a", b"c", b"b"] {
            right_left.set(key.to_vec(), key.to_vec())?;
        }
        assert_eq!(right_left.save_version()?.size, 3);
        assert_eq!(right_left.remove(b"0")?, None);
        assert_eq!(right_left.remove(b"z")?, None);
        assert_eq!(right_left.remove(b"c")?, Some(b"c".to_vec()));
        assert_eq!(right_left.remove(b"a")?, Some(b"a".to_vec()));
        assert_eq!(right_left.save_version()?.size, 1);
        Ok(())
    }

    // ---------------------------------------------------------------
    // Storage-backed mode
    // ---------------------------------------------------------------

    /// Deterministic pseudo-random workload: `(op, key, value)` per version.
    fn workload(versions: usize, ops: usize, seed: u64) -> Vec<Vec<(bool, Vec<u8>, Vec<u8>)>> {
        let mut x = seed;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        (0..versions)
            .map(|_| {
                (0..ops)
                    .map(|_| {
                        let r = next();
                        let key = format!("k{:03}", r % 300).into_bytes();
                        let value = format!("v{}", r % 1000).into_bytes();
                        (r % 4 != 0, key, value)
                    })
                    .collect()
            })
            .collect()
    }

    fn apply(tree: &mut Tree, ops: &[(bool, Vec<u8>, Vec<u8>)]) {
        for (set, key, value) in ops {
            if *set {
                tree.set(key.clone(), value.clone()).unwrap();
            } else {
                tree.remove(key).unwrap();
            }
        }
    }

    #[test]
    fn storage_backed_tree_matches_in_memory_hashes_and_reopens() {
        let ndb = NodeDb::in_memory(64);
        let mut disk = Tree::open(Arc::clone(&ndb)).unwrap();
        let mut mem = Tree::new();
        let mut roots = Vec::new();
        for ops in workload(30, 40, 7) {
            apply(&mut disk, &ops);
            apply(&mut mem, &ops);
            assert_eq!(disk.root_hash(), mem.root_hash());
            let a = disk.save_version().unwrap();
            let b = mem.save_version().unwrap();
            assert_eq!(a, b);
            roots.push((a.version, a.root_hash, entries(mem.iter(None, None))));
        }
        // Every historical version is readable with its exact hash/content.
        for (version, hash, content) in &roots {
            let snap = disk.get_immutable(*version).unwrap();
            assert_eq!(snap.root_hash(), *hash);
            assert_eq!(&entries(snap.iter(None, None)), content);
            let mem_snap = mem.get_immutable(*version).unwrap();
            assert_eq!(mem_snap.root_hash(), *hash);
        }
        // Reopen from storage with a cold cache.
        let reopened_db = NodeDb::new(
            Arc::clone(ndb.backend()),
            Vec::new(),
            super::NodeCache::new(16),
        );
        let mut reopened = Tree::open(reopened_db).unwrap();
        assert_eq!(reopened.version(), 30);
        assert_eq!(reopened.root_hash(), mem.root_hash());
        assert_eq!(
            entries(reopened.iter(None, None)),
            entries(mem.iter(None, None))
        );
        assert_eq!(
            entries(reopened.reverse_iter(Some(b"k100"), Some(b"k200"))),
            entries(mem.reverse_iter(Some(b"k100"), Some(b"k200")))
        );
        for ops in workload(5, 40, 99) {
            apply(&mut reopened, &ops);
            apply(&mut mem, &ops);
            assert_eq!(
                reopened.save_version().unwrap(),
                mem.save_version().unwrap()
            );
        }
        let stats = ndb.cache().stats();
        assert!(stats.len <= 64, "{stats:?}");
    }

    #[test]
    fn historical_proofs_verify_against_historical_roots() {
        let mut tree = Tree::open(NodeDb::in_memory(1000)).unwrap();
        let mut roots = Vec::new();
        for ops in workload(10, 30, 3) {
            apply(&mut tree, &ops);
            roots.push(tree.save_version().unwrap());
        }
        for info in &roots {
            let snap: ImmutableTree = tree.get_immutable(info.version).unwrap();
            for entry in snap.iter(None, None).take(5) {
                let entry = entry.unwrap();
                let proof = snap.get_with_proof(&entry.key).unwrap().unwrap();
                Tree::verify_membership(info.root_hash, &proof, &entry.key, &entry.value).unwrap();
            }
            let proof = snap.get_with_proof(b"k150x").unwrap().unwrap();
            Tree::verify_non_membership(info.root_hash, &proof, b"k150x").unwrap();
        }
    }

    /// Nodes reachable from the given roots.
    fn reachable(tree: &Tree) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        for version in tree.available_versions() {
            let Some(super::Link::Stored(root)) = tree.versions[&version].clone() else {
                continue;
            };
            let mut stack = vec![root];
            while let Some(r) = stack.pop() {
                if !seen.insert(r.nk) {
                    continue;
                }
                let node = tree.ndb.as_ref().unwrap().load(&r).unwrap();
                if let super::Node::Inner(inner) = &*node {
                    for link in [&inner.left, &inner.right] {
                        if let super::Link::Stored(c) = link {
                            stack.push(*c);
                        }
                    }
                }
            }
        }
        seen.len()
    }

    #[test]
    fn pruning_keeps_retained_versions_exact_and_frees_everything_else() {
        let ndb = NodeDb::in_memory(10_000);
        let mut tree = Tree::open(Arc::clone(&ndb)).unwrap();
        let mut infos = Vec::new();
        let keep = |latest: Version| move |v: Version| v > latest - 3 || v % 5 == 0;
        for (i, ops) in workload(40, 25, 11).into_iter().enumerate() {
            apply(&mut tree, &ops);
            let info = tree.save_version().unwrap();
            infos.push((info.clone(), entries(tree.iter(None, None))));
            if i % 4 == 3 {
                let mut batch = WriteBatch::new();
                tree.prune(keep(info.version), &mut batch).unwrap();
                ndb.backend().write(batch).unwrap();
            }
        }
        let latest = tree.version();
        let mut batch = WriteBatch::new();
        tree.prune(keep(latest), &mut batch).unwrap();
        ndb.backend().write(batch).unwrap();
        let expected: Vec<Version> = (1..=latest).filter(|v| keep(latest)(*v)).collect();
        assert_eq!(tree.available_versions(), expected);
        for (info, content) in &infos {
            if !expected.contains(&info.version) {
                assert!(tree.get_immutable(info.version).is_err());
                continue;
            }
            let snap = tree.get_immutable(info.version).unwrap();
            assert_eq!(snap.root_hash(), info.root_hash);
            assert_eq!(&entries(snap.iter(None, None)), content);
        }
        // No leaked nodes: every stored node is reachable from a retained root.
        assert_eq!(ndb.node_count().unwrap(), reachable(&tree));

        // Prune down to the latest version only.
        let mut batch = WriteBatch::new();
        tree.prune(|_| false, &mut batch).unwrap();
        ndb.backend().write(batch).unwrap();
        assert_eq!(tree.available_versions(), vec![latest]);
        assert_eq!(ndb.node_count().unwrap(), tree.len() * 2 - 1);
        assert_eq!(ndb.orphan_count().unwrap(), 0);
        let reopened = Tree::open(NodeDb::new(
            Arc::clone(ndb.backend()),
            Vec::new(),
            super::NodeCache::new(8),
        ))
        .unwrap();
        assert_eq!(reopened.root_hash(), infos.last().unwrap().0.root_hash);
    }

    #[test]
    fn in_memory_pruning_drops_versions() {
        let mut tree = Tree::new();
        for ops in workload(5, 10, 5) {
            apply(&mut tree, &ops);
            tree.save_version().unwrap();
        }
        let mut batch = WriteBatch::new();
        assert_eq!(tree.prune(|v| v == 2, &mut batch).unwrap(), vec![1, 3, 4]);
        assert!(batch.is_empty());
        assert_eq!(tree.available_versions(), vec![2, 5]);
        assert!(matches!(
            tree.get_immutable(3),
            Err(Error::VersionNotFound { version: 3 })
        ));
    }

    #[test]
    fn unchanged_and_emptied_versions_round_trip() {
        let ndb = NodeDb::in_memory(100);
        let mut tree = Tree::open(Arc::clone(&ndb)).unwrap();
        let empty = tree.save_version().unwrap();
        tree.set(b"a".to_vec(), b"1".to_vec()).unwrap();
        let one = tree.save_version().unwrap();
        let same = tree.save_version().unwrap();
        assert_eq!(one.root_hash, same.root_hash);
        tree.remove(b"a").unwrap();
        let emptied = tree.save_version().unwrap();
        assert_eq!(emptied.root_hash, empty.root_hash);
        let reopened = Tree::open(NodeDb::new(
            Arc::clone(ndb.backend()),
            Vec::new(),
            super::NodeCache::new(8),
        ))
        .unwrap();
        assert_eq!(reopened.available_versions(), vec![1, 2, 3, 4]);
        assert_eq!(
            reopened.get_immutable(3).unwrap().root_hash(),
            one.root_hash
        );
        assert!(reopened.is_empty());
    }

    #[test]
    fn load_version_and_rollback() {
        let ndb = NodeDb::in_memory(100);
        let mut tree = Tree::open(Arc::clone(&ndb)).unwrap();
        tree.set(b"a".to_vec(), b"1".to_vec()).unwrap();
        let one = tree.save_version().unwrap();
        tree.set(b"a".to_vec(), b"2".to_vec()).unwrap();
        tree.rollback();
        assert_eq!(tree.root_hash(), one.root_hash);
        assert_eq!(tree.get(b"a").unwrap(), Some(b"1".to_vec()));
        tree.set(b"b".to_vec(), b"2".to_vec()).unwrap();
        tree.save_version().unwrap();
        assert!(tree.load_version(1).is_err());
        assert!(matches!(
            tree.load_version(9),
            Err(Error::VersionNotFound { version: 9 })
        ));
        tree.load_version(2).unwrap();
        // A missing node surfaces as an error, not a panic.
        let mut batch = WriteBatch::new();
        ndb.clear(&mut batch).unwrap();
        ndb.backend().write(batch).unwrap();
        let cold = Tree::open(NodeDb::new(
            Arc::clone(ndb.backend()),
            Vec::new(),
            super::NodeCache::new(8),
        ))
        .unwrap();
        assert_eq!(cold.version(), 0);
    }

    #[test]
    fn export_import_round_trips_in_both_modes() {
        let mut tree = Tree::open(NodeDb::in_memory(1000)).unwrap();
        for ops in workload(12, 30, 21) {
            apply(&mut tree, &ops);
            tree.save_version().unwrap();
        }
        let version = tree.version();
        let hash = tree.root_hash();
        let nodes: Vec<_> = tree.export(version).unwrap().map(Result::unwrap).collect();
        assert_eq!(nodes.len(), tree.len() * 2 - 1);
        assert_eq!(nodes.last().unwrap().height, {
            let snap = tree.get_immutable(version).unwrap();
            let _ = snap;
            nodes.iter().map(|n| n.height).max().unwrap()
        });

        // Into storage.
        let ndb = NodeDb::in_memory(1000);
        let mut importer = super::Importer::new(Some(Arc::clone(&ndb)), version).unwrap();
        for node in nodes.iter().cloned() {
            importer.add(node).unwrap();
        }
        let mut batch = WriteBatch::new();
        let mut imported = importer.commit(&mut batch).unwrap();
        ndb.backend().write(batch).unwrap();
        assert_eq!(imported.root_hash(), hash);
        assert_eq!(imported.version(), version);
        assert_eq!(
            entries(imported.iter(None, None)),
            entries(tree.iter(None, None))
        );
        // Continue committing on both and stay in lockstep.
        for ops in workload(3, 30, 77) {
            apply(&mut imported, &ops);
            apply(&mut tree, &ops);
            assert_eq!(
                imported.save_version().unwrap(),
                tree.save_version().unwrap()
            );
        }

        // Into memory.
        let mut importer = super::Importer::new(None, version).unwrap();
        for node in nodes {
            importer.add(node).unwrap();
        }
        let mem = importer.commit(&mut WriteBatch::new()).unwrap();
        assert_eq!(mem.root_hash(), hash);

        // Malformed streams are rejected.
        let mut importer = super::Importer::new(None, 1).unwrap();
        assert!(
            importer
                .add(super::ExportNode {
                    key: b"a".to_vec(),
                    value: Vec::new(),
                    version: 1,
                    height: 1,
                })
                .is_err()
        );
        assert!(
            importer
                .add(super::ExportNode {
                    key: b"a".to_vec(),
                    value: Vec::new(),
                    version: 2,
                    height: 0,
                })
                .is_err()
        );
        // Importing into a store that already has versions is refused.
        let mut used = Tree::open(NodeDb::in_memory(10)).unwrap();
        used.set(b"a".to_vec(), b"1".to_vec()).unwrap();
        used.save_version().unwrap();
        assert!(super::Importer::new(used.node_db().cloned(), 5).is_err());
    }

    #[test]
    fn empty_tree_export_import() {
        let mut tree = Tree::open(NodeDb::in_memory(10)).unwrap();
        tree.save_version().unwrap();
        assert_eq!(tree.export(1).unwrap().count(), 0);
        let ndb = NodeDb::in_memory(10);
        let importer = super::Importer::new(Some(Arc::clone(&ndb)), 1).unwrap();
        let mut batch = WriteBatch::new();
        let imported = importer.commit(&mut batch).unwrap();
        ndb.backend().write(batch).unwrap();
        assert_eq!(imported.root_hash(), tree.root_hash());
        assert_eq!(Tree::open(ndb).unwrap().version(), 1);
    }
}
