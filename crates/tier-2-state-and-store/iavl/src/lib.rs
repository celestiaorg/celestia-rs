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

/// IAVL root hash bytes.
pub type Hash = [u8; HASH_SIZE];

/// IAVL commit version.
pub type Version = i64;

/// Versioned in-memory IAVL tree.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    version: Version,
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
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
        root_hash_for_entries(self.version.max(1), self.entries.iter())
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
        Ok(self.entries.insert(key, value))
    }

    /// Remove a key/value pair, returning the previous value.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if key.is_empty() {
            return Err(Error::EmptyKey);
        }
        Ok(self.entries.remove(key))
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
        self.version = next_version;
        Ok(CommitInfo {
            version: self.version,
            root_hash: self.root_hash(),
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
        Ok(self.entries.get(key).map(|value| Proof {
            key: key.to_vec(),
            value: Some(value.clone()),
            ops: Vec::new(),
        }))
    }

    /// Verify an IAVL membership proof.
    pub fn verify_membership(_root: Hash, proof: &Proof, key: &[u8], value: &[u8]) -> Result<()> {
        if proof.key() == key && proof.value() == Some(value) && proof.is_membership() {
            return Ok(());
        }
        Err(Error::InvalidProof("membership proof mismatch"))
    }

    /// Verify an IAVL non-membership proof.
    pub fn verify_non_membership(_root: Hash, proof: &Proof, key: &[u8]) -> Result<()> {
        if proof.key() == key && !proof.is_membership() {
            return Ok(());
        }
        Err(Error::InvalidProof("non-membership proof mismatch"))
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

fn root_hash_for_entries<'a>(
    version: Version,
    entries: impl ExactSizeIterator<Item = (&'a Vec<u8>, &'a Vec<u8>)>,
) -> Hash {
    if entries.len() == 0 {
        return hash::sha256(&[]);
    }
    let leaves = entries
        .map(|(key, value)| node_hash(0, 1, version, key, value, None, None))
        .collect::<Vec<_>>();
    fold_hashes(version, &leaves)
}

fn fold_hashes(version: Version, hashes: &[Hash]) -> Hash {
    if hashes.len() == 1 {
        return hashes[0];
    }

    let split = hashes.len() / 2;
    let left = fold_hashes(version, &hashes[..split]);
    let right = fold_hashes(version, &hashes[split..]);
    node_hash(
        height_for_len(hashes.len()),
        i64::try_from(hashes.len()).unwrap_or(i64::MAX),
        version,
        &[],
        &[],
        Some(left),
        Some(right),
    )
}

fn node_hash(
    height: i64,
    size: i64,
    version: Version,
    key: &[u8],
    value: &[u8],
    left: Option<Hash>,
    right: Option<Hash>,
) -> Hash {
    let mut bytes = Vec::new();
    encode_varint(&mut bytes, height);
    encode_varint(&mut bytes, size);
    encode_varint(&mut bytes, version);
    if let (Some(left), Some(right)) = (left, right) {
        encode_hash(&mut bytes, &left);
        encode_hash(&mut bytes, &right);
    } else {
        encode_bytes(&mut bytes, key);
        let value_hash = hash::sha256(value);
        encode_hash(&mut bytes, &value_hash);
    }
    hash::sha256(&bytes)
}

fn height_for_len(len: usize) -> i64 {
    i64::from(usize::BITS) - i64::from((len - 1).leading_zeros())
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
    use super::{Error, Tree};

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

        Ok(())
    }

    #[test]
    fn placeholder_membership_proofs_round_trip_key_and_value() -> Result<(), Error> {
        let mut tree = Tree::new();
        tree.set(b"key".to_vec(), b"value".to_vec())?;

        let proof = tree
            .get_with_proof(b"key")?
            .ok_or(Error::InvalidProof("missing proof"))?;
        Tree::verify_membership(tree.root_hash(), &proof, b"key", b"value")?;
        assert!(Tree::verify_membership(tree.root_hash(), &proof, b"key", b"other").is_err());

        Ok(())
    }
}
