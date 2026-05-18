//! Namespace and range proof verification.

use crate::namespace::Namespace;
use crate::node::{NODE_SIZE, NamespacedHash, empty_root, hash_leaf, hash_node};
use crate::tree::{TreeOptions, next_subtree_size, split_point};
use crate::{Error, Result};

/// Proof category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofKind {
    /// Empty proof for namespaces outside the root range.
    Empty,
    /// Inclusion proof for one or more matching leaves.
    Inclusion,
    /// Absence proof for a namespace inside the root range.
    Absence,
}

/// Namespace or range proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    /// First proved leaf index.
    pub start: usize,
    /// End proved leaf index, exclusive.
    pub end: usize,
    /// In-order proof nodes.
    pub nodes: Vec<NamespacedHash>,
    /// Adjacent leaf hash used for absence proofs.
    pub leaf_hash: Option<NamespacedHash>,
    /// Whether the source tree ignored max-namespace right children.
    pub is_max_namespace_ignored: bool,
}

impl Proof {
    /// Construct an empty proof.
    #[must_use]
    pub fn empty(ignore_max_namespace: bool) -> Self {
        Self {
            start: 0,
            end: 0,
            nodes: Vec::new(),
            leaf_hash: None,
            is_max_namespace_ignored: ignore_max_namespace,
        }
    }

    /// Construct an inclusion proof.
    pub fn inclusion(
        start: usize,
        end: usize,
        nodes: Vec<NamespacedHash>,
        ignore_max_namespace: bool,
    ) -> Result<Self> {
        let proof = Self {
            start,
            end,
            nodes,
            leaf_hash: None,
            is_max_namespace_ignored: ignore_max_namespace,
        };
        proof.validate_basic()?;
        Ok(proof)
    }

    /// Construct an absence proof.
    pub fn absence(
        start: usize,
        end: usize,
        nodes: Vec<NamespacedHash>,
        leaf_hash: NamespacedHash,
        ignore_max_namespace: bool,
    ) -> Result<Self> {
        let proof = Self {
            start,
            end,
            nodes,
            leaf_hash: Some(leaf_hash),
            is_max_namespace_ignored: ignore_max_namespace,
        };
        proof.validate_basic()?;
        Ok(proof)
    }

    /// Return this proof's category.
    #[must_use]
    pub fn kind(&self) -> ProofKind {
        if self.start == self.end && self.nodes.is_empty() && self.leaf_hash.is_none() {
            ProofKind::Empty
        } else if self.leaf_hash.is_some() {
            ProofKind::Absence
        } else {
            ProofKind::Inclusion
        }
    }

    /// Validate proof shape and node encoding.
    pub fn validate_basic(&self) -> Result<()> {
        if self.start == self.end {
            if self.start == 0 && self.nodes.is_empty() && self.leaf_hash.is_none() {
                return Ok(());
            }
            return Err(Error::InvalidProof);
        }

        if self.start > self.end {
            return Err(Error::InvalidRange);
        }

        for node in &self.nodes {
            node.validate_basic()?;
        }
        if let Some(leaf_hash) = self.leaf_hash {
            leaf_hash.validate_basic()?;
        }
        Ok(())
    }

    /// Encode proof nodes as fixed-size byte arrays.
    #[must_use]
    pub fn nodes_as_bytes(&self) -> Vec<[u8; NODE_SIZE]> {
        self.nodes.iter().map(NamespacedHash::to_bytes).collect()
    }

    /// Verify this proof for namespace-prefixed leaves.
    pub fn verify_namespace(
        &self,
        namespace: Namespace,
        leaves: &[Vec<u8>],
        root: &NamespacedHash,
    ) -> Result<bool> {
        if self.start == self.end {
            return self.verify_empty_namespace(namespace, leaves, root);
        }

        let leaf_hashes = if let Some(leaf_hash) = self.leaf_hash {
            if !leaves.is_empty() {
                return Err(Error::InvalidProof);
            }
            vec![leaf_hash]
        } else {
            compute_and_validate_leaf_hashes(namespace, leaves)?
        };

        self.verify_leaf_hashes(namespace, &leaf_hashes, root, true)
    }

    /// Verify this proof from already computed leaf hashes.
    pub fn verify_leaf_hashes(
        &self,
        namespace: Namespace,
        leaf_hashes: &[NamespacedHash],
        root: &NamespacedHash,
        verify_completeness: bool,
    ) -> Result<bool> {
        self.validate_proof_structure(namespace, leaf_hashes)?;
        root.validate_basic()?;

        if self.leaf_hash.is_none() {
            Self::validate_namespace(namespace, leaf_hashes)?;
        }

        if verify_completeness {
            self.validate_completeness(namespace)?;
        }

        let computed_root = self.compute_root(leaf_hashes)?;
        Ok(computed_root == *root)
    }

    fn verify_empty_namespace(
        &self,
        namespace: Namespace,
        leaves: &[Vec<u8>],
        root: &NamespacedHash,
    ) -> Result<bool> {
        if self.kind() != ProofKind::Empty || !leaves.is_empty() {
            return Ok(false);
        }
        root.validate_basic()?;
        Ok(namespace < root.min_namespace
            || root.max_namespace < namespace
            || *root == empty_root())
    }

    fn validate_proof_structure(
        &self,
        namespace: Namespace,
        leaf_hashes: &[NamespacedHash],
    ) -> Result<()> {
        self.validate_basic()?;

        let expected = self.end - self.start;
        if leaf_hashes.len() != expected {
            return Err(Error::InvalidProof);
        }

        if let Some(leaf_hash) = self.leaf_hash {
            leaf_hash.validate_basic()?;
            if namespace >= leaf_hash.min_namespace {
                return Err(Error::InvalidProof);
            }
        }

        for node in &self.nodes {
            node.validate_basic()?;
        }
        for leaf_hash in leaf_hashes {
            leaf_hash.validate_basic()?;
        }

        Ok(())
    }

    fn validate_namespace(namespace: Namespace, leaf_hashes: &[NamespacedHash]) -> Result<()> {
        for leaf_hash in leaf_hashes {
            if leaf_hash.min_namespace != namespace || leaf_hash.max_namespace != namespace {
                return Err(Error::InvalidProof);
            }
        }
        Ok(())
    }

    fn validate_completeness(&self, namespace: Namespace) -> Result<()> {
        let mut leaf_index = 0u64;
        let mut node_index = 0usize;

        while leaf_index != u64::try_from(self.start).map_err(|_| Error::InvalidProof)?
            && node_index < self.nodes.len()
        {
            let subtree_size = next_subtree_size(
                leaf_index,
                u64::try_from(self.start).map_err(|_| Error::InvalidProof)?,
            );
            leaf_index += u64::try_from(subtree_size).map_err(|_| Error::InvalidProof)?;
            node_index += 1;
        }

        if leaf_index != u64::try_from(self.start).map_err(|_| Error::InvalidProof)? {
            return Err(Error::FailedCompletenessCheck);
        }

        for subtree in &self.nodes[..node_index] {
            if namespace <= subtree.max_namespace {
                return Err(Error::FailedCompletenessCheck);
            }
        }
        for subtree in &self.nodes[node_index..] {
            if subtree.min_namespace <= namespace {
                return Err(Error::FailedCompletenessCheck);
            }
        }
        Ok(())
    }

    fn compute_root(&self, leaf_hashes: &[NamespacedHash]) -> Result<NamespacedHash> {
        let mut node_index = 0usize;
        let mut leaf_index = 0usize;
        let options = TreeOptions {
            ignore_max_namespace: self.is_max_namespace_ignored,
        };
        let estimate = split_point(self.end).saturating_mul(2).max(1);
        let mut root = self
            .compute_root_recurse(0, estimate, leaf_hashes, &mut leaf_index, &mut node_index)?
            .ok_or(Error::InvalidProof)?;

        for node in &self.nodes[node_index..] {
            root = hash_node(&root, node, options)?;
        }
        Ok(root)
    }

    fn compute_root_recurse(
        &self,
        start: usize,
        end: usize,
        leaf_hashes: &[NamespacedHash],
        leaf_index: &mut usize,
        node_index: &mut usize,
    ) -> Result<Option<NamespacedHash>> {
        if end - start == 1 {
            if start >= self.start && start < self.end {
                return pop_leaf_hash(leaf_hashes, leaf_index);
            }
            return Ok(pop_proof_node(&self.nodes, node_index));
        }

        if end <= self.start || start >= self.end {
            return Ok(pop_proof_node(&self.nodes, node_index));
        }

        let split = split_point(end - start);
        let left =
            self.compute_root_recurse(start, start + split, leaf_hashes, leaf_index, node_index)?;
        let right =
            self.compute_root_recurse(start + split, end, leaf_hashes, leaf_index, node_index)?;

        match (left, right) {
            (Some(left), Some(right)) => hash_node(
                &left,
                &right,
                TreeOptions {
                    ignore_max_namespace: self.is_max_namespace_ignored,
                },
            )
            .map(Some),
            (Some(left), None) => Ok(Some(left)),
            (None, None) => Ok(None),
            (None, Some(_)) => Err(Error::InvalidProof),
        }
    }
}

fn compute_and_validate_leaf_hashes(
    namespace: Namespace,
    leaves: &[Vec<u8>],
) -> Result<Vec<NamespacedHash>> {
    let mut hashes = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let hash = hash_leaf(leaf)?;
        if hash.min_namespace != namespace || hash.max_namespace != namespace {
            return Err(Error::InvalidProof);
        }
        hashes.push(hash);
    }
    Ok(hashes)
}

fn pop_leaf_hash(
    leaf_hashes: &[NamespacedHash],
    index: &mut usize,
) -> Result<Option<NamespacedHash>> {
    let leaf_hash = leaf_hashes
        .get(*index)
        .copied()
        .ok_or(Error::InvalidProof)?;
    *index += 1;
    Ok(Some(leaf_hash))
}

fn pop_proof_node(nodes: &[NamespacedHash], index: &mut usize) -> Option<NamespacedHash> {
    let node = nodes.get(*index).copied();
    if node.is_some() {
        *index += 1;
    }
    node
}
