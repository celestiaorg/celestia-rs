//! Ordered namespaced Merkle tree construction and proof generation.

use crate::namespace::{NAMESPACE_SIZE, Namespace};
use crate::node::{NamespacedHash, empty_root, hash_leaf, hash_node};
use crate::proof::Proof;
use crate::{Error, Result};

/// Options controlling NMT hash behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeOptions {
    /// Whether parent namespace ranges ignore a right child whose range starts
    /// at the maximum namespace.
    pub ignore_max_namespace: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            ignore_max_namespace: true,
        }
    }
}

/// Ordered Celestia namespaced Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespacedMerkleTree {
    options: TreeOptions,
    leaves: Vec<Vec<u8>>,
    leaf_hashes: Vec<NamespacedHash>,
}

impl NamespacedMerkleTree {
    /// Construct an empty tree with default options.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(TreeOptions::default())
    }

    /// Construct an empty tree with explicit options.
    #[must_use]
    pub fn with_options(options: TreeOptions) -> Self {
        Self {
            options,
            leaves: Vec::new(),
            leaf_hashes: Vec::new(),
        }
    }

    /// Return this tree's hash options.
    #[must_use]
    pub const fn options(&self) -> TreeOptions {
        self.options
    }

    /// Return the number of leaves.
    #[must_use]
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Return true when the tree has no leaves.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Borrow namespace-prefixed leaves in insertion order.
    #[must_use]
    pub fn leaves(&self) -> &[Vec<u8>] {
        &self.leaves
    }

    /// Push one namespace-prefixed leaf.
    pub fn push(&mut self, leaf: Vec<u8>) -> Result<()> {
        let namespace = namespace_from_leaf(&leaf)?;
        if let Some(previous) = self.leaves.last() {
            let previous_namespace = namespace_from_leaf(previous)?;
            if namespace < previous_namespace {
                return Err(Error::UnorderedLeaves);
            }
        }

        let leaf_hash = hash_leaf(&leaf)?;
        self.leaves.push(leaf);
        self.leaf_hashes.push(leaf_hash);
        Ok(())
    }

    /// Push all leaves from an iterator.
    pub fn extend<I>(&mut self, leaves: I) -> Result<()>
    where
        I: IntoIterator<Item = Vec<u8>>,
    {
        for leaf in leaves {
            self.push(leaf)?;
        }
        Ok(())
    }

    /// Compute the current tree root.
    pub fn root(&self) -> Result<NamespacedHash> {
        self.compute_root(0, self.len())
    }

    /// Return the root minimum namespace.
    pub fn min_namespace(&self) -> Result<Namespace> {
        Ok(self.root()?.min_namespace)
    }

    /// Return the root maximum namespace.
    pub fn max_namespace(&self) -> Result<Namespace> {
        Ok(self.root()?.max_namespace)
    }

    /// Return leaves exactly matching `namespace`.
    #[must_use]
    pub fn get(&self, namespace: Namespace) -> Vec<Vec<u8>> {
        self.leaves
            .iter()
            .filter(|leaf| {
                namespace_from_leaf(leaf).is_ok_and(|leaf_namespace| leaf_namespace == namespace)
            })
            .cloned()
            .collect()
    }

    /// Prove the leaf range `[start, end)`.
    pub fn prove_range(&self, start: usize, end: usize) -> Result<Proof> {
        self.validate_range(start, end)?;
        let nodes = self.build_range_proof(start, end)?;
        Proof::inclusion(start, end, nodes, self.options.ignore_max_namespace)
    }

    /// Prove inclusion or absence for a namespace.
    pub fn prove_namespace(&self, namespace: Namespace) -> Result<Proof> {
        if self.is_empty() {
            return Ok(Proof::empty(self.options.ignore_max_namespace));
        }

        let root = self.root()?;
        if namespace < root.min_namespace || root.max_namespace < namespace {
            return Ok(Proof::empty(self.options.ignore_max_namespace));
        }

        if let Some((start, end)) = self.find_namespace_range(namespace) {
            let nodes = self.build_range_proof(start, end)?;
            return Proof::inclusion(start, end, nodes, self.options.ignore_max_namespace);
        }

        let index = self
            .calculate_absence_index(namespace)
            .ok_or(Error::InvalidProof)?;
        let nodes = self.build_range_proof(index, index + 1)?;
        Proof::absence(
            index,
            index + 1,
            nodes,
            self.leaf_hashes[index],
            self.options.ignore_max_namespace,
        )
    }

    /// Compute the subtree root for `[start, end)`.
    pub fn subtree_root(&self, start: usize, end: usize) -> Result<NamespacedHash> {
        self.validate_range(start, end)?;
        let start_u64 = u64::try_from(start).map_err(|_| Error::InvalidRange)?;
        let end_u64 = u64::try_from(end).map_err(|_| Error::InvalidRange)?;
        if next_subtree_size(start_u64, end_u64) != end - start {
            return Err(Error::InvalidRange);
        }
        self.compute_root(start, end)
    }

    /// Remove all leaves.
    pub fn reset(&mut self) {
        self.leaves.clear();
        self.leaf_hashes.clear();
    }

    fn validate_range(&self, start: usize, end: usize) -> Result<()> {
        if start >= end || end > self.len() {
            return Err(Error::InvalidRange);
        }
        Ok(())
    }

    fn compute_root(&self, start: usize, end: usize) -> Result<NamespacedHash> {
        if start > end || end > self.len() {
            return Err(Error::InvalidRange);
        }

        match end - start {
            0 => Ok(empty_root()),
            1 => Ok(self.leaf_hashes[start]),
            len => {
                let split = split_point(len);
                let left = self.compute_root(start, start + split)?;
                let right = self.compute_root(start + split, end)?;
                hash_node(&left, &right, self.options)
            }
        }
    }

    fn build_range_proof(
        &self,
        proof_start: usize,
        proof_end: usize,
    ) -> Result<Vec<NamespacedHash>> {
        self.validate_range(proof_start, proof_end)?;

        let mut proof = Vec::new();
        let full_tree_size = split_point(self.len()).saturating_mul(2).max(1);
        self.range_proof_recurse(0, full_tree_size, true, proof_start, proof_end, &mut proof)?;
        Ok(proof)
    }

    fn range_proof_recurse(
        &self,
        start: usize,
        end: usize,
        include_node: bool,
        proof_start: usize,
        proof_end: usize,
        proof: &mut Vec<NamespacedHash>,
    ) -> Result<Option<NamespacedHash>> {
        if start >= self.len() {
            return Ok(None);
        }

        if end - start == 1 {
            let leaf_hash = self.leaf_hashes[start];
            if (start < proof_start || start >= proof_end) && include_node {
                proof.push(leaf_hash);
            }
            return Ok(Some(leaf_hash));
        }

        let mut next_include_node = include_node;
        if (end <= proof_start || start >= proof_end) && include_node {
            next_include_node = false;
        }

        let split = split_point(end - start);
        let left = self.range_proof_recurse(
            start,
            start + split,
            next_include_node,
            proof_start,
            proof_end,
            proof,
        )?;
        let right = self.range_proof_recurse(
            start + split,
            end,
            next_include_node,
            proof_start,
            proof_end,
            proof,
        )?;

        let hash = match (left, right) {
            (Some(left), Some(right)) => hash_node(&left, &right, self.options)?,
            (Some(left), None) => left,
            (None, None) => return Ok(None),
            (None, Some(_)) => return Err(Error::InvalidProof),
        };

        if include_node && !next_include_node {
            proof.push(hash);
        }

        Ok(Some(hash))
    }

    fn find_namespace_range(&self, namespace: Namespace) -> Option<(usize, usize)> {
        let mut start = None;
        let mut end = 0;

        for (index, leaf) in self.leaves.iter().enumerate() {
            let leaf_namespace = namespace_from_leaf(leaf).ok()?;
            if leaf_namespace == namespace {
                if start.is_none() {
                    start = Some(index);
                }
                end = index + 1;
            } else if start.is_some() && leaf_namespace > namespace {
                break;
            }
        }

        start.map(|start| (start, end))
    }

    fn calculate_absence_index(&self, namespace: Namespace) -> Option<usize> {
        for index in 1..self.leaves.len() {
            let previous_namespace = namespace_from_leaf(&self.leaves[index - 1]).ok()?;
            let current_namespace = namespace_from_leaf(&self.leaves[index]).ok()?;
            if previous_namespace < namespace && namespace < current_namespace {
                return Some(index);
            }
        }
        None
    }
}

impl Default for NamespacedMerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

fn namespace_from_leaf(leaf: &[u8]) -> Result<Namespace> {
    if leaf.len() < NAMESPACE_SIZE {
        return Err(Error::InvalidLeafLength {
            actual: leaf.len(),
            min: NAMESPACE_SIZE,
        });
    }
    Namespace::from_slice(&leaf[..NAMESPACE_SIZE])
}

pub(crate) fn split_point(length: usize) -> usize {
    debug_assert!(length > 0);
    let bit_len = usize::BITS - length.leading_zeros();
    let mut split = 1usize << (bit_len - 1);
    if split == length {
        split >>= 1;
    }
    split
}

pub(crate) fn next_subtree_size(start: u64, end: u64) -> usize {
    debug_assert!(start < end);
    let ideal = start.trailing_zeros();
    let max_bits = u64::BITS - (end - start).leading_zeros() - 1;
    1usize << ideal.min(max_bits)
}
