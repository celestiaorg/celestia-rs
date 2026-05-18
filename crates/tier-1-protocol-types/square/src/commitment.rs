//! Blob share commitment helpers.

use hash::Sha256Hasher;
use shares::{NAMESPACE_SIZE, Namespace, PARITY_SHARES_NAMESPACE};

use crate::inclusion::{round_down_power_of_two, sub_tree_width};
use crate::{Error, Result};

/// Size in bytes of a SHA-256 commitment hash.
pub const COMMITMENT_HASH_SIZE: usize = 32;

/// Size in bytes of a 29-byte-namespace NMT root.
pub const NMT_NODE_SIZE: usize = (2 * NAMESPACE_SIZE) + COMMITMENT_HASH_SIZE;

const NMT_LEAF_PREFIX: u8 = 0;
const NMT_NODE_PREFIX: u8 = 1;

/// Merkle root callback used over NMT subtree roots.
pub type MerkleRootFn = fn(&[Vec<u8>]) -> [u8; COMMITMENT_HASH_SIZE];

/// Create the share commitment for one blob.
pub fn create_commitment(
    blob: &shares::Blob,
    merkle_root_fn: MerkleRootFn,
    subtree_root_threshold: usize,
) -> Result<[u8; COMMITMENT_HASH_SIZE]> {
    blob.validate_basic()?;
    let shares = shares::split_blobs(core::slice::from_ref(blob))?;
    let subtree_width = sub_tree_width(shares.len(), subtree_root_threshold)?;
    let tree_sizes = merkle_mountain_range_sizes(shares.len() as u64, subtree_width as u64)?;

    let mut subtree_roots = Vec::with_capacity(tree_sizes.len());
    let mut cursor = 0usize;
    for tree_size in tree_sizes {
        let tree_size = usize::try_from(tree_size).map_err(|_| Error::ArithmeticOverflow)?;
        let end = cursor
            .checked_add(tree_size)
            .ok_or(Error::ArithmeticOverflow)?;
        let mut leaves = Vec::with_capacity(tree_size);
        for share in &shares[cursor..end] {
            let mut leaf = Vec::with_capacity(NAMESPACE_SIZE + share.as_bytes().len());
            leaf.extend_from_slice(blob.namespace().as_bytes());
            leaf.extend_from_slice(share.as_bytes());
            leaves.push(leaf);
        }
        subtree_roots.push(namespaced_merkle_root(&leaves)?.to_vec());
        cursor = end;
    }

    Ok(merkle_root_fn(&subtree_roots))
}

/// Create share commitments for several blobs.
pub fn create_commitments(
    blobs: &[shares::Blob],
    merkle_root_fn: MerkleRootFn,
    subtree_root_threshold: usize,
) -> Result<Vec<[u8; COMMITMENT_HASH_SIZE]>> {
    blobs
        .iter()
        .map(|blob| create_commitment(blob, merkle_root_fn, subtree_root_threshold))
        .collect()
}

/// Return the tree sizes used by a Merkle mountain range.
pub fn merkle_mountain_range_sizes(total_size: u64, max_tree_size: u64) -> Result<Vec<u64>> {
    if max_tree_size == 0 {
        return Err(Error::InvalidNamespacedMerkleTree(
            "max tree size must be non-zero",
        ));
    }

    let mut total_size = total_size;
    let mut tree_sizes = Vec::new();
    while total_size != 0 {
        if total_size >= max_tree_size {
            tree_sizes.push(max_tree_size);
            total_size -= max_tree_size;
        } else {
            let tree_size = round_down_power_of_two(
                usize::try_from(total_size).map_err(|_| Error::ArithmeticOverflow)?,
            )?;
            let tree_size = u64::try_from(tree_size).map_err(|_| Error::ArithmeticOverflow)?;
            tree_sizes.push(tree_size);
            total_size -= tree_size;
        }
    }
    Ok(tree_sizes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NamespacedRoot {
    min_namespace: Namespace,
    max_namespace: Namespace,
    digest: [u8; COMMITMENT_HASH_SIZE],
}

impl NamespacedRoot {
    fn leaf(leaf: &[u8]) -> Result<Self> {
        if leaf.len() < NAMESPACE_SIZE {
            return Err(Error::InvalidNamespacedMerkleTree(
                "leaf shorter than namespace",
            ));
        }
        let namespace = Namespace::from_slice(&leaf[..NAMESPACE_SIZE])?;
        let mut hasher = Sha256Hasher::new();
        hasher.update(&[NMT_LEAF_PREFIX]);
        hasher.update(leaf);
        Ok(Self {
            min_namespace: namespace,
            max_namespace: namespace,
            digest: hasher.finalize(),
        })
    }

    fn node(left: &Self, right: &Self) -> Result<Self> {
        if right.min_namespace < left.max_namespace {
            return Err(Error::InvalidNamespacedMerkleTree(
                "siblings are not namespace ordered",
            ));
        }
        let max_namespace = if right.min_namespace == PARITY_SHARES_NAMESPACE {
            left.max_namespace
        } else {
            right.max_namespace
        };

        let left_bytes = left.to_bytes();
        let right_bytes = right.to_bytes();
        let mut hasher = Sha256Hasher::new();
        hasher.update(&[NMT_NODE_PREFIX]);
        hasher.update(&left_bytes);
        hasher.update(&right_bytes);

        Ok(Self {
            min_namespace: left.min_namespace,
            max_namespace,
            digest: hasher.finalize(),
        })
    }

    fn to_bytes(self) -> [u8; NMT_NODE_SIZE] {
        let mut out = [0; NMT_NODE_SIZE];
        out[..NAMESPACE_SIZE].copy_from_slice(self.min_namespace.as_bytes());
        out[NAMESPACE_SIZE..2 * NAMESPACE_SIZE].copy_from_slice(self.max_namespace.as_bytes());
        out[2 * NAMESPACE_SIZE..].copy_from_slice(&self.digest);
        out
    }
}

fn nmt_root<T: AsRef<[u8]>>(leaves: &[T]) -> Result<NamespacedRoot> {
    match leaves {
        [] => Err(Error::InvalidNamespacedMerkleTree("empty tree")),
        [leaf] => NamespacedRoot::leaf(leaf.as_ref()),
        _ => {
            let split = nmt_split_point(leaves.len())?;
            let left = nmt_root(&leaves[..split])?;
            let right = nmt_root(&leaves[split..])?;
            NamespacedRoot::node(&left, &right)
        }
    }
}

pub(crate) fn namespaced_merkle_root<T: AsRef<[u8]>>(leaves: &[T]) -> Result<[u8; NMT_NODE_SIZE]> {
    Ok(nmt_root(leaves)?.to_bytes())
}

fn nmt_split_point(len: usize) -> Result<usize> {
    if len < 2 {
        return Err(Error::InvalidNamespacedMerkleTree(
            "split point requires at least two leaves",
        ));
    }
    let rounded_down = round_down_power_of_two(len)?;
    if rounded_down == len {
        Ok(rounded_down / 2)
    } else {
        Ok(rounded_down)
    }
}

#[cfg(test)]
mod tests {
    use shares::{Namespace, PARITY_SHARES_NAMESPACE};

    use super::*;

    #[test]
    fn nmt_helpers_reject_malformed_inputs() -> Result<()> {
        assert!(nmt_root::<Vec<u8>>(&[]).is_err());
        assert!(NamespacedRoot::leaf(b"too short").is_err());
        assert!(nmt_split_point(1).is_err());
        assert_eq!(nmt_split_point(4)?, 2);
        assert_eq!(nmt_split_point(5)?, 4);

        let high = namespaced_leaf(Namespace::new_v0(&[2])?, b"high");
        let low = namespaced_leaf(Namespace::new_v0(&[1])?, b"low");
        assert!(nmt_root(&[high, low]).is_err());
        Ok(())
    }

    #[test]
    fn nmt_node_ignores_parity_namespace_as_max_namespace() -> Result<()> {
        let left_namespace = Namespace::new_v0(&[1])?;
        let left = NamespacedRoot::leaf(&namespaced_leaf(left_namespace, b"left"))?;
        let right = NamespacedRoot::leaf(&namespaced_leaf(PARITY_SHARES_NAMESPACE, b"right"))?;

        let root = NamespacedRoot::node(&left, &right)?;
        assert_eq!(root.min_namespace, left_namespace);
        assert_eq!(root.max_namespace, left_namespace);
        assert_eq!(root.to_bytes().len(), NMT_NODE_SIZE);
        Ok(())
    }

    #[test]
    fn nmt_root_covers_balanced_and_unbalanced_trees() -> Result<()> {
        let namespace = Namespace::new_v0(&[9])?;
        let leaves = [
            namespaced_leaf(namespace, b"one"),
            namespaced_leaf(namespace, b"two"),
            namespaced_leaf(namespace, b"three"),
        ];

        let root = nmt_root(&leaves)?;
        assert_eq!(root.min_namespace, namespace);
        assert_eq!(root.max_namespace, namespace);
        assert_eq!(root.to_bytes().len(), NMT_NODE_SIZE);
        Ok(())
    }

    fn namespaced_leaf(namespace: Namespace, data: &[u8]) -> Vec<u8> {
        let mut leaf = Vec::with_capacity(NAMESPACE_SIZE + data.len());
        leaf.extend_from_slice(namespace.as_bytes());
        leaf.extend_from_slice(data);
        leaf
    }
}
