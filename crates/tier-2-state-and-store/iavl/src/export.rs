//! IAVL export/import (the node stream used by Cosmos SDK state-sync
//! snapshots): every node in post-order as `(key, value, version, height)`.
//! Inner nodes carry their separator key and no value.

use std::collections::HashMap;
use std::sync::Arc;

use kvdb::WriteBatch;

use crate::mutate::load;
use crate::node::{
    Inner, Leaf, Link, Node, NodeKey, StoredRef, encode_inner, encode_leaf, encode_root,
};
use crate::nodedb::{NodeDb, db_err};
use crate::{Error, Result, Tree, Version, inner_hash, leaf_hash};

/// One exported node (Go `iavl.ExportNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportNode {
    /// Leaf key, or the inner node's separator key.
    pub key: Vec<u8>,
    /// Leaf value; empty for inner nodes.
    pub value: Vec<u8>,
    /// Version the node was created at.
    pub version: Version,
    /// Subtree height (0 for leaves).
    pub height: i8,
}

/// Post-order node iterator over one committed version.
#[derive(Debug)]
pub struct Exporter {
    ndb: Option<Arc<NodeDb>>,
    stack: Vec<(Arc<Node>, bool)>,
}

impl Exporter {
    pub(crate) fn new(ndb: Option<Arc<NodeDb>>, root: Option<&Link>) -> Result<Self> {
        let mut stack = Vec::new();
        if let Some(root) = root {
            stack.push((load(ndb.as_deref(), root)?, false));
        }
        Ok(Self { ndb, stack })
    }
}

impl Iterator for Exporter {
    type Item = Result<ExportNode>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (node, expanded) = self.stack.pop()?;
            if let (Node::Inner(inner), false) = (&*node, expanded) {
                let children = load(self.ndb.as_deref(), &inner.left)
                    .and_then(|l| Ok((l, load(self.ndb.as_deref(), &inner.right)?)));
                let (left, right) = match children {
                    Ok(children) => children,
                    Err(err) => {
                        self.stack.clear();
                        return Some(Err(err));
                    }
                };
                self.stack.push((Arc::clone(&node), true));
                self.stack.push((right, false));
                self.stack.push((left, false));
                continue;
            }
            let Some(version) = node.saved_version() else {
                self.stack.clear();
                return Some(Err(Error::MalformedNode("cannot export an unsaved node")));
            };
            return Some(Ok(match &*node {
                Node::Leaf(leaf) => ExportNode {
                    key: leaf.key.clone(),
                    value: leaf.value.clone(),
                    version,
                    height: 0,
                },
                Node::Inner(inner) => ExportNode {
                    key: inner.key.clone(),
                    value: Vec::new(),
                    version,
                    height: inner.height,
                },
            }));
        }
    }
}

/// Rebuilds a tree at `version` from a post-order [`ExportNode`] stream.
///
/// Storage-backed imports flush node writes to the backend in bounded
/// batches while importing; the root record is only queued by
/// [`Importer::commit`], so an interrupted import never yields a readable
/// version.
#[derive(Debug)]
pub struct Importer {
    ndb: Option<Arc<NodeDb>>,
    version: Version,
    stack: Vec<Link>,
    nonces: HashMap<Version, u32>,
    batch: WriteBatch,
}

const IMPORT_FLUSH_OPS: usize = 50_000;

impl Importer {
    /// Starts an import at `version` into `ndb` (or into memory). The node
    /// store must not hold any version yet.
    pub fn new(ndb: Option<Arc<NodeDb>>, version: Version) -> Result<Self> {
        if version <= 0 {
            return Err(Error::InvalidVersion { version });
        }
        if let Some(ndb) = &ndb {
            if !ndb.roots()?.is_empty() {
                return Err(Error::InvalidVersion { version });
            }
        }
        Ok(Self {
            ndb,
            version,
            stack: Vec::new(),
            nonces: HashMap::new(),
            batch: WriteBatch::new(),
        })
    }

    /// Adds the next node of the post-order stream.
    pub fn add(&mut self, node: ExportNode) -> Result<()> {
        if node.version > self.version || node.version <= 0 {
            return Err(Error::InvalidVersion {
                version: node.version,
            });
        }
        let version = node.version;
        let (bytes, saved, height, size, hash) = if node.height == 0 {
            if node.key.is_empty() {
                return Err(Error::EmptyKey);
            }
            let hash = leaf_hash(version, &node.key, &node.value);
            let bytes = self
                .ndb
                .as_ref()
                .map(|_| encode_leaf(&node.key, &node.value));
            let saved = Node::Leaf(Leaf {
                key: node.key,
                value: node.value,
                version,
                hash: Some(hash),
            });
            (bytes, saved, 0, 1, hash)
        } else {
            let n = self.stack.len();
            if n < 2
                || self.stack[n - 1].height() >= node.height
                || self.stack[n - 2].height() >= node.height
            {
                return Err(Error::MalformedNode(
                    "inner node without two lower children",
                ));
            }
            let right = self.stack.pop().ok_or(Error::MalformedNode("stack"))?;
            let left = self.stack.pop().ok_or(Error::MalformedNode("stack"))?;
            let size = left.size() + right.size();
            let hash = inner_hash(
                i64::from(node.height),
                size,
                version,
                left.hash_for(version),
                right.hash_for(version),
            );
            let bytes = match (&left, &right) {
                (Link::Stored(l), Link::Stored(r)) => {
                    Some(encode_inner(&node.key, node.height, size, l, r))
                }
                _ => None,
            };
            let saved = Node::Inner(Inner {
                key: node.key,
                height: node.height,
                size,
                left,
                right,
                version,
                hash: Some(hash),
            });
            (bytes, saved, node.height, size, hash)
        };
        let link = match (&self.ndb, bytes) {
            (Some(ndb), Some(bytes)) => {
                let nonce = self.nonces.entry(version).or_insert(0);
                *nonce += 1;
                let r = StoredRef {
                    nk: NodeKey {
                        version,
                        nonce: *nonce,
                    },
                    hash,
                    height,
                    size,
                };
                ndb.put_node(&mut self.batch, r.nk, bytes, Arc::new(saved));
                if self.batch.len() >= IMPORT_FLUSH_OPS {
                    ndb.backend()
                        .write(std::mem::take(&mut self.batch))
                        .map_err(db_err)?;
                }
                Link::Stored(r)
            }
            (None, None) => Link::mem(saved),
            _ => return Err(Error::Corrupt("mixed import links")),
        };
        self.stack.push(link);
        Ok(())
    }

    /// Finishes the import: queues the remaining nodes and the root record
    /// into `batch` (which the caller must write) and returns the tree.
    pub fn commit(mut self, batch: &mut WriteBatch) -> Result<Tree> {
        if self.stack.len() > 1 {
            return Err(Error::MalformedNode("import stream left several roots"));
        }
        let root = self.stack.pop();
        if let Some(ndb) = &self.ndb {
            let root_ref = match &root {
                Some(Link::Stored(r)) => Some(*r),
                Some(Link::Mem(_)) => return Err(Error::Corrupt("mixed import links")),
                None => None,
            };
            batch.extend(std::mem::take(&mut self.batch));
            batch.put(ndb.root_key(self.version), encode_root(root_ref.as_ref()));
        }
        Ok(Tree::from_import(self.ndb, self.version, root))
    }
}
