//! Namespaced hash values and NMT hashing functions.

use hash::{Sha256Hasher, sha256};

use crate::namespace::{MAX_NAMESPACE, NAMESPACE_SIZE, Namespace};
use crate::tree::TreeOptions;
use crate::{Error, Result};

/// Size in bytes of the SHA-256 digest inside an NMT node.
pub const HASH_SIZE: usize = 32;

/// Encoded namespaced hash size in bytes.
pub const NODE_SIZE: usize = (2 * NAMESPACE_SIZE) + HASH_SIZE;

/// NMT leaf hash domain-separation prefix.
pub const LEAF_PREFIX: u8 = 0;

/// NMT inner-node hash domain-separation prefix.
pub const NODE_PREFIX: u8 = 1;

/// A Celestia namespaced hash: `min_namespace || max_namespace || digest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespacedHash {
    /// Minimum namespace covered by this hash.
    pub min_namespace: Namespace,
    /// Maximum namespace covered by this hash.
    pub max_namespace: Namespace,
    /// SHA-256 digest component.
    pub digest: [u8; HASH_SIZE],
}

impl NamespacedHash {
    /// Construct a namespaced hash from already split parts.
    #[must_use]
    pub const fn new(
        min_namespace: Namespace,
        max_namespace: Namespace,
        digest: [u8; HASH_SIZE],
    ) -> Self {
        Self {
            min_namespace,
            max_namespace,
            digest,
        }
    }

    /// Validate the namespace ordering inside this namespaced hash.
    pub fn validate_basic(&self) -> Result<()> {
        if self.max_namespace < self.min_namespace {
            return Err(Error::InvalidNodeNamespaceOrder);
        }
        Ok(())
    }

    /// Parse an encoded namespaced hash.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != NODE_SIZE {
            return Err(Error::InvalidNodeLength {
                actual: bytes.len(),
                expected: NODE_SIZE,
            });
        }

        let min_namespace = Namespace::from_slice(&bytes[..NAMESPACE_SIZE])?;
        let max_namespace = Namespace::from_slice(&bytes[NAMESPACE_SIZE..2 * NAMESPACE_SIZE])?;
        let mut digest = [0u8; HASH_SIZE];
        digest.copy_from_slice(&bytes[2 * NAMESPACE_SIZE..]);

        let hash = Self::new(min_namespace, max_namespace, digest);
        hash.validate_basic()?;
        Ok(hash)
    }

    /// Encode this namespaced hash as `min_namespace || max_namespace || digest`.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; NODE_SIZE] {
        let mut bytes = [0u8; NODE_SIZE];
        bytes[..NAMESPACE_SIZE].copy_from_slice(self.min_namespace.as_bytes());
        bytes[NAMESPACE_SIZE..2 * NAMESPACE_SIZE].copy_from_slice(self.max_namespace.as_bytes());
        bytes[2 * NAMESPACE_SIZE..].copy_from_slice(&self.digest);
        bytes
    }
}

/// Return Go NMT's empty root for an 8-byte namespace ID.
#[must_use]
pub fn empty_root() -> NamespacedHash {
    NamespacedHash::new(
        Namespace::new([0; NAMESPACE_SIZE]),
        Namespace::new([0; NAMESPACE_SIZE]),
        sha256(&[]),
    )
}

/// Hash namespace-prefixed leaf data.
pub fn hash_leaf(leaf: &[u8]) -> Result<NamespacedHash> {
    if leaf.len() < NAMESPACE_SIZE {
        return Err(Error::InvalidLeafLength {
            actual: leaf.len(),
            min: NAMESPACE_SIZE,
        });
    }

    let namespace = Namespace::from_slice(&leaf[..NAMESPACE_SIZE])?;
    let mut hasher = Sha256Hasher::new();
    hasher.update(&[LEAF_PREFIX]);
    hasher.update(leaf);

    Ok(NamespacedHash::new(namespace, namespace, hasher.finalize()))
}

/// Hash two ordered child namespaced hashes into a parent namespaced hash.
pub fn hash_node(
    left: &NamespacedHash,
    right: &NamespacedHash,
    options: TreeOptions,
) -> Result<NamespacedHash> {
    left.validate_basic()?;
    right.validate_basic()?;

    if right.min_namespace < left.max_namespace {
        return Err(Error::UnorderedSiblings);
    }

    let max_namespace = if options.ignore_max_namespace && right.min_namespace == MAX_NAMESPACE {
        left.max_namespace
    } else {
        right.max_namespace
    };

    let left_bytes = left.to_bytes();
    let right_bytes = right.to_bytes();
    let mut hasher = Sha256Hasher::new();
    hasher.update(&[NODE_PREFIX]);
    hasher.update(&left_bytes);
    hasher.update(&right_bytes);

    Ok(NamespacedHash::new(
        left.min_namespace,
        max_namespace,
        hasher.finalize(),
    ))
}
