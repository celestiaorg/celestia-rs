//! `crypto/merkle.Proof` with Go's exact field types and error strings
//! (the tier-0 `merkle` crate provides the hashing).

use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::proto::crypto::Proof as ProtoProof;

/// `merkle.MaxAunts`.
pub const MAX_AUNTS: usize = 100;
/// `tmhash.Size`.
pub const HASH_SIZE: usize = 32;

/// A simple Merkle tree inclusion proof.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Proof {
    /// Total number of items.
    pub total: i64,
    /// Index of the proven item.
    pub index: i64,
    /// Leaf hash of the item.
    pub leaf_hash: Vec<u8>,
    /// Hashes from the leaf's sibling up to a child of the root.
    pub aunts: Vec<Vec<u8>>,
}

/// `merkle.HashFromByteSlices`.
#[must_use]
pub fn hash_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> Vec<u8> {
    merkle::root_from_byte_slices(items).to_vec()
}

/// `merkle.ProofsFromByteSlices` (and `ParallelProofsFromByteSlices`, which
/// computes the same values).
#[must_use]
pub fn proofs_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> (Vec<u8>, Vec<Proof>) {
    let (root, proofs) = merkle::proofs_from_byte_slices(items);
    let proofs = proofs
        .into_iter()
        .map(|p| Proof {
            total: i64::try_from(p.total).unwrap_or(i64::MAX),
            index: i64::try_from(p.index).unwrap_or(i64::MAX),
            leaf_hash: p.leaf_hash.to_vec(),
            aunts: p.aunts.iter().map(|a| a.to_vec()).collect(),
        })
        .collect();
    (root.to_vec(), proofs)
}

fn leaf_hash(leaf: &[u8]) -> Vec<u8> {
    merkle::leaf_hash(leaf).to_vec()
}

fn inner_hash(left: &[u8], right: &[u8]) -> Vec<u8> {
    let mut h = hash::Sha256Hasher::new();
    h.update(&[1]);
    h.update(left);
    h.update(right);
    h.finalize().to_vec()
}

/// `getSplitPoint`: largest power of two strictly less than `length`.
fn split_point(length: i64) -> i64 {
    let bitlen = 64 - length.leading_zeros();
    let mut k = 1i64 << (bitlen - 1);
    if k == length {
        k >>= 1;
    }
    k
}

fn compute_hash_from_aunts(
    index: i64,
    total: i64,
    leaf_hash: &[u8],
    inner_hashes: &[Vec<u8>],
) -> Result<Vec<u8>> {
    if index >= total || index < 0 || total <= 0 {
        return Err(Error::msg(format!(
            "invalid index {index} and/or total {total}"
        )));
    }
    if total == 1 {
        if !inner_hashes.is_empty() {
            return Err(Error::msg("unexpected inner hashes"));
        }
        return Ok(leaf_hash.to_vec());
    }
    let Some((last, rest)) = inner_hashes.split_last() else {
        return Err(Error::msg("expected at least one inner hash"));
    };
    let num_left = split_point(total);
    if index < num_left {
        let left = compute_hash_from_aunts(index, num_left, leaf_hash, rest)?;
        return Ok(inner_hash(&left, last));
    }
    let right = compute_hash_from_aunts(index - num_left, total - num_left, leaf_hash, rest)?;
    Ok(inner_hash(last, &right))
}

impl Proof {
    /// `Proof.Verify(rootHash, leaf)`. `root_hash == None` is Go's nil root.
    pub fn verify(&self, root_hash: Option<&[u8]>, leaf: &[u8]) -> Result<()> {
        let Some(root_hash) = root_hash else {
            return Err(Error::msg("invalid root hash: cannot be nil"));
        };
        if self.total < 0 {
            return Err(Error::msg("proof total must be positive"));
        }
        if self.index < 0 {
            return Err(Error::msg("proof index cannot be negative"));
        }
        let lh = leaf_hash(leaf);
        if self.leaf_hash != lh {
            return Err(Error::msg(format!(
                "invalid leaf hash: wanted {} got {}",
                hex_upper(&lh),
                hex_upper(&self.leaf_hash)
            )));
        }
        let computed = self
            .compute_root_hash()
            .map_err(|e| e.wrap("compute root hash: "))?;
        if computed != root_hash {
            return Err(Error::msg(format!(
                "invalid root hash: wanted {} got {}",
                hex_upper(root_hash),
                hex_upper(&computed)
            )));
        }
        Ok(())
    }

    /// `Proof.computeRootHash`.
    pub fn compute_root_hash(&self) -> Result<Vec<u8>> {
        compute_hash_from_aunts(self.index, self.total, &self.leaf_hash, &self.aunts)
    }

    /// `Proof.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.total < 0 {
            return Err(Error::msg("negative Total"));
        }
        if self.index < 0 {
            return Err(Error::msg("negative Index"));
        }
        if self.leaf_hash.len() != HASH_SIZE {
            return Err(Error::msg(format!(
                "expected LeafHash size to be {HASH_SIZE}, got {}",
                self.leaf_hash.len()
            )));
        }
        if self.aunts.len() > MAX_AUNTS {
            return Err(Error::msg(format!(
                "expected no more than {MAX_AUNTS} aunts, got {}",
                self.aunts.len()
            )));
        }
        for (i, aunt) in self.aunts.iter().enumerate() {
            if aunt.len() != HASH_SIZE {
                return Err(Error::msg(format!(
                    "expected Aunts#{i} size to be {HASH_SIZE}, got {}",
                    aunt.len()
                )));
            }
        }
        Ok(())
    }

    /// `Proof.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> ProtoProof {
        ProtoProof {
            total: self.total,
            index: self.index,
            leaf_hash: self.leaf_hash.clone(),
            aunts: self.aunts.clone(),
        }
    }

    /// `merkle.ProofFromProto(pb, optional)`. `Ok(None)` only when optional.
    pub fn from_proto(pb: Option<&ProtoProof>, optional: bool) -> Result<Option<Self>> {
        let Some(pb) = pb.filter(|pb| !(pb.leaf_hash.is_empty() && pb.aunts.is_empty())) else {
            if optional {
                return Ok(None);
            }
            return Err(Error::msg("nil proof"));
        };
        let sp = Self {
            total: pb.total,
            index: pb.index,
            leaf_hash: pb.leaf_hash.clone(),
            aunts: pb.aunts.clone(),
        };
        sp.validate_basic()?;
        Ok(Some(sp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_verify() {
        let items: Vec<Vec<u8>> = (0u8..7).map(|i| vec![i; 3]).collect();
        let (root, proofs) = proofs_from_byte_slices(&items);
        for (i, p) in proofs.iter().enumerate() {
            p.verify(Some(&root), &items[i])
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(split_point(2), 1);
        assert_eq!(split_point(5), 4);
        assert_eq!(split_point(8), 4);
    }
}
