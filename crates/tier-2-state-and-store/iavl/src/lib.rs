//! Cosmos IAVL+ state tree primitives.
//!
//! See `docs/superpowers/specs/m2/iavl.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;

use std::collections::BTreeMap;

pub use error::{Error, Result};

/// Byte length of an IAVL root hash.
pub const HASH_SIZE: usize = 32;
const HASH_SIZE_PREFIX: u8 = 32;

/// IAVL root hash bytes.
pub type Hash = [u8; HASH_SIZE];

/// IAVL commit version.
pub type Version = i64;

/// Versioned in-memory IAVL tree.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    version: Version,
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    root: Option<Box<Node>>,
    dirty: bool,
}

impl Tree {
    /// Create an empty tree at version zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the latest saved version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Return the number of key/value entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return true when the tree contains no key/value entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the deterministic root hash for the current tree contents.
    #[must_use]
    pub fn root_hash(&self) -> Hash {
        root_hash_for_node(self.root.as_deref(), self.hash_version())
    }

    /// Return the value for `key`, if present.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// Insert or replace a key/value pair, returning the previous value.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let previous = self.entries.insert(key.clone(), value.clone());
        self.root = Some(match self.root.take() {
            Some(root) => Box::new(recursive_set(*root, key, value).0),
            None => Box::new(Node::leaf(key, value)),
        });
        self.dirty = true;
        Ok(previous)
    }

    /// Remove a key/value pair, returning the previous value.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let previous = self.entries.remove(key);
        if previous.is_some() {
            if let Some(root) = self.root.take() {
                self.root = recursive_remove(*root, key).node.map(Box::new);
            }
            self.dirty = true;
        }
        Ok(previous)
    }

    /// Commit the current working tree as the next version.
    pub fn save_version(&mut self) -> Result<CommitInfo> {
        let Some(next_version) = self.version.checked_add(1) else {
            return Err(Error::InvalidVersion {
                version: self.version,
            });
        };
        if next_version <= 0 {
            return Err(Error::InvalidVersion {
                version: next_version,
            });
        }
        let root_hash = match self.root.as_mut() {
            Some(root) => root.assign_hashes(next_version),
            None => hash::sha256(&[]),
        };
        self.version = next_version;
        self.dirty = false;
        Ok(CommitInfo {
            version: self.version,
            root_hash,
            size: self.entries.len(),
        })
    }

    /// Iterate forward over the half-open range `[start, end)`.
    #[must_use]
    pub fn iter<'a>(&'a self, start: Option<&'a [u8]>, end: Option<&'a [u8]>) -> Iter<'a> {
        Iter::new(self.entries.iter(), start, end, false)
    }

    /// Iterate backward over the half-open range `[start, end)`.
    #[must_use]
    pub fn reverse_iter<'a>(&'a self, start: Option<&'a [u8]>, end: Option<&'a [u8]>) -> Iter<'a> {
        Iter::new(self.entries.iter(), start, end, true)
    }

    /// Return membership or non-membership proof material for `key`.
    pub fn get_with_proof(&self, key: &[u8]) -> Result<Option<Proof>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        if self.root.is_none() {
            return Ok(None);
        }
        Ok(if let Some(value) = self.entries.get(key) {
            Some(self.membership_proof(key, value)?)
        } else {
            Some(self.non_membership_proof(key)?)
        })
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
            self.version.saturating_add(1).max(1)
        } else {
            self.version.max(1)
        }
    }

    fn membership_proof(&self, key: &[u8], value: &[u8]) -> Result<Proof> {
        let existence = self
            .existence_proof(key)
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
        let key_vec = key.to_vec();
        let left_key = self
            .entries
            .range(..key_vec.clone())
            .next_back()
            .map(|(left, _)| left.clone());
        let right_key = self
            .entries
            .range(key_vec..)
            .next()
            .map(|(right, _)| right.clone());

        let left = left_key
            .as_deref()
            .map(|left| {
                self.existence_proof(left)
                    .ok_or(Error::InvalidProof("missing left non-membership neighbor"))
            })
            .transpose()?;
        let right = right_key
            .as_deref()
            .map(|right| {
                self.existence_proof(right)
                    .ok_or(Error::InvalidProof("missing right non-membership neighbor"))
            })
            .transpose()?;
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

    fn existence_proof(&self, key: &[u8]) -> Option<ExistenceProof> {
        let mut path = Vec::new();
        let (leaf, found) = self
            .root
            .as_deref()?
            .path_to_leaf(key, self.hash_version(), &mut path);
        if !found {
            return None;
        }
        Some(ExistenceProof {
            key: leaf.key.clone(),
            value: leaf.value.clone(),
            leaf_version: leaf.version.unwrap_or_else(|| self.hash_version()),
            path,
        })
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

/// Borrowed key/value iterator entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Entry key.
    pub key: &'a [u8],
    /// Entry value.
    pub value: &'a [u8],
}

/// Range iterator over IAVL entries.
#[derive(Debug, Clone)]
pub struct Iter<'a> {
    entries: Vec<Entry<'a>>,
    index: usize,
}

impl<'a> Iter<'a> {
    fn new<'b>(
        source: impl Iterator<Item = (&'a Vec<u8>, &'a Vec<u8>)>,
        start: Option<&'b [u8]>,
        end: Option<&'b [u8]>,
        reverse: bool,
    ) -> Self {
        let invalid_range = matches!((start, end), (Some(start), Some(end)) if start > end);
        let mut entries = if invalid_range {
            Vec::new()
        } else {
            source
                .filter(|(key, _)| start.is_none_or(|bound| key.as_slice() >= bound))
                .filter(|(key, _)| end.is_none_or(|bound| key.as_slice() < bound))
                .map(|(key, value)| Entry {
                    key: key.as_slice(),
                    value: value.as_slice(),
                })
                .collect::<Vec<_>>()
        };
        if reverse {
            entries.reverse();
        }
        Self { entries, index: 0 }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.get(self.index).copied()?;
        self.index += 1;
        Some(entry)
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Leaf(LeafNode),
    Inner(InnerNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LeafNode {
    key: Vec<u8>,
    value: Vec<u8>,
    hash: Option<Hash>,
    version: Option<Version>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InnerNode {
    key: Vec<u8>,
    height: i8,
    size: i64,
    left: Box<Node>,
    right: Box<Node>,
    hash: Option<Hash>,
    version: Option<Version>,
}

impl Node {
    fn leaf(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self::Leaf(LeafNode {
            key,
            value,
            hash: None,
            version: None,
        })
    }

    fn height(&self) -> i8 {
        match self {
            Self::Leaf(_) => 0,
            Self::Inner(node) => node.height,
        }
    }

    fn size(&self) -> i64 {
        match self {
            Self::Leaf(_) => 1,
            Self::Inner(node) => node.size,
        }
    }

    fn cached_hash(&self) -> Option<Hash> {
        match self {
            Self::Leaf(node) => node.hash,
            Self::Inner(node) => node.hash,
        }
    }

    fn version(&self) -> Option<Version> {
        match self {
            Self::Leaf(node) => node.version,
            Self::Inner(node) => node.version,
        }
    }

    fn hash_for(&self, version: Version) -> Hash {
        if let Some(hash) = self.cached_hash() {
            return hash;
        }
        match self {
            Self::Leaf(node) => leaf_hash(version, &node.key, &node.value),
            Self::Inner(node) => {
                let left = node.left.hash_for(version);
                let right = node.right.hash_for(version);
                inner_hash(i64::from(node.height), node.size, version, left, right)
            }
        }
    }

    fn assign_hashes(&mut self, version: Version) -> Hash {
        if let Some(hash) = self.cached_hash() {
            return hash;
        }
        let hash = match self {
            Self::Leaf(node) => leaf_hash(version, &node.key, &node.value),
            Self::Inner(node) => {
                let left = node.left.assign_hashes(version);
                let right = node.right.assign_hashes(version);
                inner_hash(i64::from(node.height), node.size, version, left, right)
            }
        };
        match self {
            Self::Leaf(node) => {
                node.hash = Some(hash);
                node.version = Some(version);
            }
            Self::Inner(node) => {
                node.hash = Some(hash);
                node.version = Some(version);
            }
        }
        hash
    }

    fn path_to_leaf<'a>(
        &'a self,
        key: &[u8],
        version: Version,
        path: &mut Vec<ProofInner>,
    ) -> (&'a LeafNode, bool) {
        match self {
            Self::Leaf(leaf) => (leaf, leaf.key.as_slice() == key),
            Self::Inner(inner) if key < inner.key.as_slice() => {
                path.push(ProofInner {
                    height: inner.height,
                    size: inner.size,
                    version: self.version().unwrap_or(version),
                    left: None,
                    right: Some(inner.right.hash_for(version)),
                });
                inner.left.path_to_leaf(key, version, path)
            }
            Self::Inner(inner) => {
                path.push(ProofInner {
                    height: inner.height,
                    size: inner.size,
                    version: self.version().unwrap_or(version),
                    left: Some(inner.left.hash_for(version)),
                    right: None,
                });
                inner.right.path_to_leaf(key, version, path)
            }
        }
    }

    #[cfg(test)]
    fn validate_shape(&self, version: Version) -> Result<()> {
        if version <= 0 {
            return Err(Error::MalformedNode("version must be greater than 0"));
        }
        self.validate_node()
    }

    #[cfg(test)]
    fn validate_node(&self) -> Result<()> {
        match self {
            Self::Leaf(leaf) => {
                if leaf.key.is_empty() {
                    return Err(Error::MalformedNode("key cannot be empty"));
                }
                if leaf.version.is_some_and(|version| version <= 0) {
                    return Err(Error::MalformedNode("version must be greater than 0"));
                }
                Ok(())
            }
            Self::Inner(inner) => {
                if inner.key.is_empty() {
                    return Err(Error::MalformedNode("key cannot be empty"));
                }
                if inner.height <= 0 {
                    return Err(Error::MalformedNode("height must be greater than 0"));
                }
                if inner.size < 2 {
                    return Err(Error::MalformedNode("inner size must be at least 2"));
                }
                let expected_size = inner.left.size() + inner.right.size();
                if inner.size != expected_size {
                    return Err(Error::MalformedNode("inner size mismatch"));
                }
                let expected_height = inner.left.height().max(inner.right.height()) + 1;
                if inner.height != expected_height {
                    return Err(Error::MalformedNode("inner height mismatch"));
                }
                if inner.left.max_key() >= inner.key.as_slice() {
                    return Err(Error::MalformedNode("left subtree overlaps separator"));
                }
                if inner.right.min_key() != inner.key.as_slice() {
                    return Err(Error::MalformedNode(
                        "right subtree must start at separator",
                    ));
                }
                if inner.version.is_some_and(|version| version <= 0) {
                    return Err(Error::MalformedNode("version must be greater than 0"));
                }
                inner.left.validate_node()?;
                inner.right.validate_node()
            }
        }
    }

    #[cfg(test)]
    fn min_key(&self) -> &[u8] {
        match self {
            Self::Leaf(leaf) => &leaf.key,
            Self::Inner(inner) => inner.left.min_key(),
        }
    }

    #[cfg(test)]
    fn max_key(&self) -> &[u8] {
        match self {
            Self::Leaf(leaf) => &leaf.key,
            Self::Inner(inner) => inner.right.max_key(),
        }
    }
}

fn root_hash_for_node(root: Option<&Node>, version: Version) -> Hash {
    root.map_or_else(|| hash::sha256(&[]), |node| node.hash_for(version))
}

fn recursive_set(node: Node, key: Vec<u8>, value: Vec<u8>) -> (Node, bool) {
    match node {
        Node::Leaf(leaf) => match key.as_slice().cmp(leaf.key.as_slice()) {
            core::cmp::Ordering::Less => (
                make_inner(
                    leaf.key.clone(),
                    Box::new(Node::leaf(key, value)),
                    Box::new(Node::Leaf(leaf)),
                ),
                false,
            ),
            core::cmp::Ordering::Greater => (
                make_inner(
                    key.clone(),
                    Box::new(Node::Leaf(leaf)),
                    Box::new(Node::leaf(key, value)),
                ),
                false,
            ),
            core::cmp::Ordering::Equal => (Node::leaf(key, value), true),
        },
        Node::Inner(mut inner) => {
            let updated = if key.as_slice() < inner.key.as_slice() {
                let (left, updated) = recursive_set(*inner.left, key, value);
                inner.left = Box::new(left);
                updated
            } else {
                let (right, updated) = recursive_set(*inner.right, key, value);
                inner.right = Box::new(right);
                updated
            };
            if updated {
                return (
                    Node::Inner(InnerNode {
                        hash: None,
                        version: None,
                        ..inner
                    }),
                    true,
                );
            }
            (
                balance(make_inner(inner.key, inner.left, inner.right)),
                false,
            )
        }
    }
}

struct RemoveResult {
    node: Option<Node>,
    new_key: Option<Vec<u8>>,
    value: Option<Vec<u8>>,
    removed: bool,
}

#[allow(clippy::too_many_lines)]
fn recursive_remove(node: Node, key: &[u8]) -> RemoveResult {
    match node {
        Node::Leaf(leaf) => {
            if key == leaf.key.as_slice() {
                RemoveResult {
                    node: None,
                    new_key: None,
                    value: Some(leaf.value),
                    removed: true,
                }
            } else {
                RemoveResult {
                    node: Some(Node::Leaf(leaf)),
                    new_key: None,
                    value: None,
                    removed: false,
                }
            }
        }
        Node::Inner(inner) => {
            let InnerNode {
                key: mut node_key,
                height,
                size,
                left,
                right,
                hash,
                version,
            } = inner;
            if key < node_key.as_slice() {
                let result = recursive_remove(*left, key);
                if !result.removed {
                    let Some(left) = result.node else {
                        return RemoveResult {
                            node: None,
                            new_key: None,
                            value: result.value,
                            removed: false,
                        };
                    };
                    return RemoveResult {
                        node: Some(Node::Inner(InnerNode {
                            key: node_key,
                            height,
                            size,
                            left: Box::new(left),
                            right,
                            hash,
                            version,
                        })),
                        new_key: None,
                        value: result.value,
                        removed: false,
                    };
                }
                let Some(left) = result.node else {
                    return RemoveResult {
                        node: Some(*right),
                        new_key: Some(node_key),
                        value: result.value,
                        removed: true,
                    };
                };
                let node = balance(make_inner(node_key, Box::new(left), right));
                RemoveResult {
                    node: Some(node),
                    new_key: result.new_key,
                    value: result.value,
                    removed: true,
                }
            } else {
                let result = recursive_remove(*right, key);
                if !result.removed {
                    let Some(right) = result.node else {
                        return RemoveResult {
                            node: None,
                            new_key: None,
                            value: result.value,
                            removed: false,
                        };
                    };
                    return RemoveResult {
                        node: Some(Node::Inner(InnerNode {
                            key: node_key,
                            height,
                            size,
                            left,
                            right: Box::new(right),
                            hash,
                            version,
                        })),
                        new_key: None,
                        value: result.value,
                        removed: false,
                    };
                }
                let Some(right) = result.node else {
                    return RemoveResult {
                        node: Some(*left),
                        new_key: None,
                        value: result.value,
                        removed: true,
                    };
                };
                if let Some(new_key) = result.new_key {
                    node_key = new_key;
                }
                let node = balance(make_inner(node_key, left, Box::new(right)));
                RemoveResult {
                    node: Some(node),
                    new_key: None,
                    value: result.value,
                    removed: true,
                }
            }
        }
    }
}

fn make_inner(key: Vec<u8>, left: Box<Node>, right: Box<Node>) -> Node {
    Node::Inner(InnerNode {
        key,
        height: left.height().max(right.height()) + 1,
        size: left.size() + right.size(),
        left,
        right,
        hash: None,
        version: None,
    })
}

fn balance(node: Node) -> Node {
    let inner = match node {
        Node::Inner(inner) => inner,
        leaf @ Node::Leaf(_) => return leaf,
    };
    let balance = i16::from(inner.left.height()) - i16::from(inner.right.height());
    if balance > 1 {
        let left_balance = match inner.left.as_ref() {
            Node::Leaf(_) => 0,
            Node::Inner(left) => i16::from(left.left.height()) - i16::from(left.right.height()),
        };
        if left_balance >= 0 {
            return rotate_right(inner);
        }
        let mut inner = inner;
        inner.left = Box::new(rotate_left_child(*inner.left));
        return rotate_right(inner);
    }
    if balance < -1 {
        let right_balance = match inner.right.as_ref() {
            Node::Leaf(_) => 0,
            Node::Inner(right) => i16::from(right.left.height()) - i16::from(right.right.height()),
        };
        if right_balance <= 0 {
            return rotate_left(inner);
        }
        let mut inner = inner;
        inner.right = Box::new(rotate_right_child(*inner.right));
        return rotate_left(inner);
    }
    Node::Inner(inner)
}

fn rotate_right_child(node: Node) -> Node {
    match node {
        Node::Inner(inner) => rotate_right(inner),
        leaf @ Node::Leaf(_) => leaf,
    }
}

fn rotate_left_child(node: Node) -> Node {
    match node {
        Node::Inner(inner) => rotate_left(inner),
        leaf @ Node::Leaf(_) => leaf,
    }
}

fn rotate_right(mut node: InnerNode) -> Node {
    let left = match *node.left {
        Node::Inner(left) => left,
        leaf @ Node::Leaf(_) => {
            node.left = Box::new(leaf);
            return Node::Inner(node);
        }
    };
    let old_root = make_inner(node.key, left.right, node.right);
    make_inner(left.key, left.left, Box::new(old_root))
}

fn rotate_left(mut node: InnerNode) -> Node {
    let right = match *node.right {
        Node::Inner(right) => right,
        leaf @ Node::Leaf(_) => {
            node.right = Box::new(leaf);
            return Node::Inner(node);
        }
    };
    let old_root = make_inner(node.key, node.left, right.left);
    make_inner(right.key, Box::new(old_root), right.right)
}

fn leaf_hash(version: Version, key: &[u8], value: &[u8]) -> Hash {
    node_hash(0, 1, version, Some((key, value)), None)
}

fn inner_hash(height: i64, size: i64, version: Version, left: Hash, right: Hash) -> Hash {
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
    use super::{Error, InnerNode, Node, ProofOpType, Tree, Version};

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
        assert_eq!(tree.get(b"a"), Some(&b"one"[..]));
        assert_eq!(tree.get(b"b"), Some(&b"two-updated"[..]));

        let commit = tree.save_version()?;
        assert_eq!(commit.version, 1);
        assert_eq!(commit.size, 2);
        assert_eq!(tree.version(), 1);
        assert!(!tree.root_hash().iter().all(|byte| *byte == 0));

        assert_eq!(tree.remove(b"a")?, Some(b"one".to_vec()));
        assert_eq!(tree.remove(b"missing")?, None);
        assert_eq!(tree.get(b"a"), None);

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

        let forward = tree
            .iter(Some(b"b"), Some(b"d"))
            .map(|entry| (entry.key.to_vec(), entry.value.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            forward,
            vec![
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec())
            ]
        );

        let reverse = tree
            .reverse_iter(Some(b"b"), Some(b"d"))
            .map(|entry| (entry.key.to_vec(), entry.value.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            reverse,
            vec![
                (b"c".to_vec(), b"3".to_vec()),
                (b"b".to_vec(), b"2".to_vec())
            ]
        );
        assert_eq!(tree.iter(Some(b"d"), Some(b"a")).count(), 0);

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
    fn malformed_node_shapes_are_rejected() {
        let valid = super::make_inner(
            b"b".to_vec(),
            Box::new(Node::leaf(b"a".to_vec(), b"1".to_vec())),
            Box::new(Node::leaf(b"b".to_vec(), b"2".to_vec())),
        );
        assert_eq!(valid.validate_shape(1), Ok(()));
        assert_eq!(
            valid.validate_shape(0),
            Err(Error::MalformedNode("version must be greater than 0"))
        );

        assert_eq!(
            Node::leaf(Vec::new(), b"1".to_vec()).validate_shape(1),
            Err(Error::MalformedNode("key cannot be empty"))
        );

        let bad_height = InnerNode {
            key: b"b".to_vec(),
            height: 0,
            size: 2,
            left: Box::new(Node::leaf(b"a".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"b".to_vec(), b"2".to_vec())),
            hash: None,
            version: None,
        };
        assert_eq!(
            Node::Inner(bad_height).validate_shape(1),
            Err(Error::MalformedNode("height must be greater than 0"))
        );

        let bad_size = InnerNode {
            key: b"b".to_vec(),
            height: 1,
            size: 1,
            left: Box::new(Node::leaf(b"a".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"b".to_vec(), b"2".to_vec())),
            hash: None,
            version: None,
        };
        assert_eq!(
            Node::Inner(bad_size).validate_shape(1),
            Err(Error::MalformedNode("inner size must be at least 2"))
        );

        let bad_order = InnerNode {
            key: b"z".to_vec(),
            height: 1,
            size: 2,
            left: Box::new(Node::leaf(b"a".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"b".to_vec(), b"2".to_vec())),
            hash: None,
            version: None,
        };
        assert_eq!(
            Node::Inner(bad_order).validate_shape(1),
            Err(Error::MalformedNode(
                "right subtree must start at separator"
            ))
        );

        let bad_version = InnerNode {
            key: b"b".to_vec(),
            height: 1,
            size: 2,
            left: Box::new(Node::leaf(b"a".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"b".to_vec(), b"2".to_vec())),
            hash: None,
            version: Some(0),
        };
        assert_eq!(
            Node::Inner(bad_version).validate_shape(1),
            Err(Error::MalformedNode("version must be greater than 0"))
        );
    }

    #[test]
    fn avl_rotations_dirty_hashes_and_removals_cover_edge_paths() -> Result<(), Error> {
        let mut left_right = Tree::new();
        for key in [b"c", b"a", b"b"] {
            left_right.set(key.to_vec(), key.to_vec())?;
        }
        assert_ne!(left_right.root_hash(), [0; 32]);
        let left_right_commit = left_right.save_version()?;
        assert_eq!(left_right_commit.size, 3);

        let mut right_left = Tree::new();
        for key in [b"a", b"c", b"b"] {
            right_left.set(key.to_vec(), key.to_vec())?;
        }
        assert_ne!(right_left.root_hash(), [0; 32]);
        let right_left_commit = right_left.save_version()?;
        assert_eq!(right_left_commit.size, 3);

        assert_eq!(right_left.remove(b"0")?, None);
        assert_eq!(right_left.remove(b"z")?, None);
        assert_eq!(right_left.remove(b"c")?, Some(b"c".to_vec()));
        assert_eq!(right_left.remove(b"a")?, Some(b"a".to_vec()));
        assert_eq!(right_left.save_version()?.size, 1);

        let leaf = Node::leaf(b"x".to_vec(), b"1".to_vec());
        assert!(matches!(super::rotate_left_child(leaf), Node::Leaf(_)));
        let leaf = Node::leaf(b"x".to_vec(), b"1".to_vec());
        assert!(matches!(super::rotate_right_child(leaf), Node::Leaf(_)));

        let invalid_left = InnerNode {
            key: b"y".to_vec(),
            height: 1,
            size: 2,
            left: Box::new(Node::leaf(b"x".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"y".to_vec(), b"2".to_vec())),
            hash: None,
            version: None,
        };
        assert!(matches!(super::rotate_right(invalid_left), Node::Inner(_)));
        let invalid_right = InnerNode {
            key: b"y".to_vec(),
            height: 1,
            size: 2,
            left: Box::new(Node::leaf(b"x".to_vec(), b"1".to_vec())),
            right: Box::new(Node::leaf(b"y".to_vec(), b"2".to_vec())),
            hash: None,
            version: None,
        };
        assert!(matches!(super::rotate_left(invalid_right), Node::Inner(_)));

        Ok(())
    }
}
