//! Celestia namespaced Merkle tree primitives.
//!
//! See `docs/specs/m1/nmt.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod namespace;
pub mod node;
pub mod proof;
pub mod subroot;
pub mod tree;

pub use error::{Error, Result};
pub use namespace::{MAX_NAMESPACE, MIN_NAMESPACE, NAMESPACE_SIZE, Namespace};
pub use node::{
    HASH_SIZE, LEAF_PREFIX, NODE_PREFIX, NODE_SIZE, NamespacedHash, empty_root, hash_leaf,
    hash_node,
};
pub use proof::{Proof, ProofKind};
pub use subroot::{SubrootPath, subroot_paths};
pub use tree::{NamespacedMerkleTree, TreeOptions};
