//! `CometBFT` simple Merkle tree primitives for celestia-rs.
//!
//! This crate implements the deterministic binary Merkle tree from `CometBFT`'s
//! `crypto/merkle` package. It is intended for short ordered byte-slice lists
//! such as Tendermint protocol fields, not for mutable stores or namespaced
//! data availability trees.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use hash::{Sha256Hasher, sha256};

/// Size in bytes of a simple Merkle hash.
pub const HASH_LENGTH: usize = 32;

/// Maximum number of aunt hashes accepted in a proof.
pub const MAX_AUNTS: usize = 100;

/// A 32-byte simple Merkle hash.
pub type Hash = [u8; HASH_LENGTH];

/// Errors returned by simple Merkle proof helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The proof has `total == 0`, or an operation requiring leaves was called
    /// for an empty tree.
    #[error("invalid Merkle proof total")]
    InvalidTotal,
    /// The proof index is outside `0..total`.
    #[error("invalid Merkle proof index")]
    InvalidIndex,
    /// The proof contains more than [`MAX_AUNTS`] aunt hashes.
    #[error("too many Merkle proof aunts: max {max}, actual {actual}")]
    TooManyAunts {
        /// Maximum accepted aunt count.
        max: usize,
        /// Actual aunt count.
        actual: usize,
    },
    /// The proof has fewer or more aunt hashes than the tree shape requires.
    #[error("unexpected Merkle proof aunt count")]
    UnexpectedAuntCount,
    /// The proof leaf hash does not match the provided leaf bytes.
    #[error("Merkle proof leaf hash mismatch")]
    LeafHashMismatch,
    /// The computed root does not match the expected root.
    #[error("Merkle proof root mismatch")]
    RootMismatch,
}

/// An inclusion proof for one leaf in a `CometBFT` simple Merkle tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// Total number of leaves in the tree.
    pub total: u64,
    /// Zero-based index of the proved leaf.
    pub index: u64,
    /// `CometBFT` leaf hash of the proved leaf.
    pub leaf_hash: Hash,
    /// Sibling hashes from leaf-adjacent to root-adjacent.
    pub aunts: Vec<Hash>,
}

impl Proof {
    /// Construct and validate a proof from raw parts.
    pub fn new(total: u64, index: u64, leaf_hash: Hash, aunts: Vec<Hash>) -> Result<Self, Error> {
        let proof = Self {
            total,
            index,
            leaf_hash,
            aunts,
        };
        proof.validate_basic()?;
        Ok(proof)
    }

    /// Validate proof shape without checking a root or leaf preimage.
    pub fn validate_basic(&self) -> Result<(), Error> {
        if self.total == 0 {
            return Err(Error::InvalidTotal);
        }
        if self.index >= self.total {
            return Err(Error::InvalidIndex);
        }
        if self.aunts.len() > MAX_AUNTS {
            return Err(Error::TooManyAunts {
                max: MAX_AUNTS,
                actual: self.aunts.len(),
            });
        }
        Ok(())
    }

    /// Compute the root hash implied by this proof.
    pub fn compute_root_hash(&self) -> Result<Hash, Error> {
        self.validate_basic()?;
        compute_hash_from_aunts(self.index, self.total, &self.leaf_hash, &self.aunts)
    }

    /// Verify that this proof proves `leaf` is included in `root`.
    pub fn verify(&self, root: &Hash, leaf: &[u8]) -> Result<(), Error> {
        if leaf_hash(leaf) != self.leaf_hash {
            return Err(Error::LeafHashMismatch);
        }
        if self.compute_root_hash()? != *root {
            return Err(Error::RootMismatch);
        }
        Ok(())
    }
}

/// Return `CometBFT`'s empty-tree hash: SHA-256([]).
#[must_use]
pub fn empty_hash() -> Hash {
    sha256(&[])
}

/// Return `CometBFT`'s leaf hash: SHA-256(0x00 || leaf).
#[must_use]
pub fn leaf_hash(leaf: &[u8]) -> Hash {
    let mut hasher = Sha256Hasher::new();
    hasher.update(&[0]);
    hasher.update(leaf);
    hasher.finalize()
}

/// Return `CometBFT`'s inner hash: SHA-256(0x01 || left || right).
#[must_use]
pub fn inner_hash(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256Hasher::new();
    hasher.update(&[1]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize()
}

/// Return the root hash for an ordered list of byte slices.
#[must_use]
pub fn root_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> Hash {
    match items {
        [] => empty_hash(),
        [leaf] => leaf_hash(leaf.as_ref()),
        _ => {
            let split = split_point_usize(items.len());
            let left = root_from_byte_slices(&items[..split]);
            let right = root_from_byte_slices(&items[split..]);
            inner_hash(&left, &right)
        }
    }
}

/// Return the root hash and inclusion proof for every item.
#[must_use]
pub fn proofs_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> (Hash, Vec<Proof>) {
    if items.is_empty() {
        return (empty_hash(), Vec::new());
    }

    let total = usize_to_u64(items.len());
    let (root, partials) = build_proofs(items);
    let proofs = partials
        .into_iter()
        .enumerate()
        .map(|(index, partial)| Proof {
            total,
            index: usize_to_u64(index),
            leaf_hash: partial.leaf_hash,
            aunts: partial.aunts,
        })
        .collect();
    (root, proofs)
}

#[derive(Debug)]
struct PartialProof {
    leaf_hash: Hash,
    aunts: Vec<Hash>,
}

fn build_proofs<T: AsRef<[u8]>>(items: &[T]) -> (Hash, Vec<PartialProof>) {
    match items {
        [] => (empty_hash(), Vec::new()),
        [leaf] => {
            let leaf_hash = leaf_hash(leaf.as_ref());
            (
                leaf_hash,
                vec![PartialProof {
                    leaf_hash,
                    aunts: Vec::new(),
                }],
            )
        }
        _ => {
            let split = split_point_usize(items.len());
            let (left_root, mut left_proofs) = build_proofs(&items[..split]);
            let (right_root, mut right_proofs) = build_proofs(&items[split..]);

            for proof in &mut left_proofs {
                proof.aunts.push(right_root);
            }
            for proof in &mut right_proofs {
                proof.aunts.push(left_root);
            }

            left_proofs.append(&mut right_proofs);
            (inner_hash(&left_root, &right_root), left_proofs)
        }
    }
}

fn compute_hash_from_aunts(
    index: u64,
    total: u64,
    leaf_hash: &Hash,
    inner_hashes: &[Hash],
) -> Result<Hash, Error> {
    if total == 0 {
        return Err(Error::InvalidTotal);
    }
    if index >= total {
        return Err(Error::InvalidIndex);
    }

    if total == 1 {
        if inner_hashes.is_empty() {
            return Ok(*leaf_hash);
        }
        return Err(Error::UnexpectedAuntCount);
    }

    let Some((last, rest)) = inner_hashes.split_last() else {
        return Err(Error::UnexpectedAuntCount);
    };
    let left_count = split_point_u64(total);
    if index < left_count {
        let left_hash = compute_hash_from_aunts(index, left_count, leaf_hash, rest)?;
        Ok(inner_hash(&left_hash, last))
    } else {
        let right_hash =
            compute_hash_from_aunts(index - left_count, total - left_count, leaf_hash, rest)?;
        Ok(inner_hash(last, &right_hash))
    }
}

fn split_point_usize(length: usize) -> usize {
    debug_assert!(length > 1);
    let bit_len = usize::BITS - (length - 1).leading_zeros();
    1usize << (bit_len - 1)
}

fn split_point_u64(length: u64) -> u64 {
    debug_assert!(length > 1);
    let bit_len = u64::BITS - (length - 1).leading_zeros();
    1u64 << (bit_len - 1)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn h(hex: &str) -> Hash {
        hex::decode(hex)
            .expect("valid hex")
            .try_into()
            .expect("32-byte hash")
    }

    #[test]
    fn empty_tree_hash_matches_sha256_empty() {
        assert_eq!(
            empty_hash(),
            h("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let items: [&[u8]; 0] = [];
        assert_eq!(root_from_byte_slices(&items), empty_hash());
    }

    #[test]
    fn leaf_hash_uses_rfc6962_prefix() {
        assert_eq!(
            leaf_hash(b""),
            h("6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d")
        );
        assert_eq!(
            leaf_hash(b"L123456"),
            h("395aa064aa4c29f7010acfe3f25db9485bbd4b91897b6ad7ad547639252b4d56")
        );
    }

    #[test]
    fn inner_hash_uses_rfc6962_prefix() {
        let left = leaf_hash(b"N123");
        let right = leaf_hash(b"N456");
        let expected = sha256(&[&[1], left.as_slice(), right.as_slice()].concat());
        assert_eq!(inner_hash(&left, &right), expected);
    }

    #[test]
    fn root_for_single_leaf_is_leaf_hash() {
        let items = [b"celestia".as_slice()];
        assert_eq!(root_from_byte_slices(&items), leaf_hash(b"celestia"));
    }

    #[test]
    fn root_is_order_sensitive() {
        let first = [b"left".as_slice(), b"right".as_slice()];
        let second = [b"right".as_slice(), b"left".as_slice()];
        assert_ne!(
            root_from_byte_slices(&first),
            root_from_byte_slices(&second)
        );
    }

    #[test]
    fn split_point_matches_cometbft_shape() {
        let cases = [
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (6, 4),
            (7, 4),
            (8, 4),
            (9, 8),
            (16, 8),
            (17, 16),
        ];
        for (length, split) in cases {
            assert_eq!(split_point_usize(length), split);
        }
    }

    #[test]
    fn empty_proofs_are_empty() {
        let items: [&[u8]; 0] = [];
        assert_eq!(proofs_from_byte_slices(&items), (empty_hash(), Vec::new()));
    }

    #[test]
    fn generated_proofs_verify_for_each_leaf() {
        let items = [
            b"one".as_slice(),
            b"two".as_slice(),
            b"three".as_slice(),
            b"four".as_slice(),
            b"five".as_slice(),
        ];
        let (root, proofs) = proofs_from_byte_slices(&items);
        assert_eq!(root, root_from_byte_slices(&items));
        assert_eq!(proofs.len(), items.len());

        for (index, proof) in proofs.iter().enumerate() {
            assert_eq!(proof.total, usize_to_u64(items.len()));
            assert_eq!(proof.index, usize_to_u64(index));
            assert_eq!(proof.leaf_hash, leaf_hash(items[index]));
            proof.verify(&root, items[index]).expect("proof verifies");
        }
    }

    #[test]
    fn proof_aunts_are_leaf_adjacent_first() {
        let items = [b"a".as_slice(), b"b".as_slice(), b"c".as_slice()];
        let (_, proofs) = proofs_from_byte_slices(&items);
        assert_eq!(proofs[0].aunts, vec![leaf_hash(b"b"), leaf_hash(b"c")]);
        assert_eq!(
            proofs[2].aunts,
            vec![inner_hash(&leaf_hash(b"a"), &leaf_hash(b"b"))]
        );
    }

    #[test]
    fn proof_new_validates_basic_shape() {
        let proof = Proof::new(1, 0, leaf_hash(b"a"), Vec::new()).expect("valid proof");
        assert_eq!(proof.compute_root_hash().expect("root"), leaf_hash(b"a"));

        assert_eq!(
            Proof::new(0, 0, leaf_hash(b"a"), Vec::new()),
            Err(Error::InvalidTotal)
        );
        assert_eq!(
            Proof::new(1, 1, leaf_hash(b"a"), Vec::new()),
            Err(Error::InvalidIndex)
        );
        assert_eq!(
            Proof::new(1, 0, leaf_hash(b"a"), vec![empty_hash(); MAX_AUNTS + 1]),
            Err(Error::TooManyAunts {
                max: MAX_AUNTS,
                actual: MAX_AUNTS + 1
            })
        );
    }

    #[test]
    fn proof_verify_rejects_leaf_and_root_mismatch() {
        let items = [b"a".as_slice(), b"b".as_slice()];
        let (root, proofs) = proofs_from_byte_slices(&items);
        assert_eq!(
            proofs[0].verify(&root, b"not-a"),
            Err(Error::LeafHashMismatch)
        );

        let mut wrong_root = root;
        wrong_root[0] ^= 0xff;
        assert_eq!(
            proofs[0].verify(&wrong_root, b"a"),
            Err(Error::RootMismatch)
        );
    }

    #[test]
    fn proof_compute_rejects_wrong_aunt_counts() {
        let items = [b"a".as_slice(), b"b".as_slice()];
        let (_, proofs) = proofs_from_byte_slices(&items);
        let mut missing = proofs[0].clone();
        missing.aunts.clear();
        assert_eq!(missing.compute_root_hash(), Err(Error::UnexpectedAuntCount));

        let extra = Proof::new(1, 0, leaf_hash(b"a"), vec![empty_hash()]).expect("basic shape");
        assert_eq!(extra.compute_root_hash(), Err(Error::UnexpectedAuntCount));
    }

    #[test]
    fn display_messages_are_readable() {
        assert_eq!(
            Error::InvalidTotal.to_string(),
            "invalid Merkle proof total"
        );
        assert_eq!(
            Error::InvalidIndex.to_string(),
            "invalid Merkle proof index"
        );
        assert_eq!(
            Error::TooManyAunts { max: 1, actual: 2 }.to_string(),
            "too many Merkle proof aunts: max 1, actual 2"
        );
        assert_eq!(
            Error::UnexpectedAuntCount.to_string(),
            "unexpected Merkle proof aunt count"
        );
        assert_eq!(
            Error::LeafHashMismatch.to_string(),
            "Merkle proof leaf hash mismatch"
        );
        assert_eq!(
            Error::RootMismatch.to_string(),
            "Merkle proof root mismatch"
        );
    }
}
