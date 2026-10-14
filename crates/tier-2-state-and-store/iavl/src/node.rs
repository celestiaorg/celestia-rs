//! Tree nodes, child links and the on-disk node encoding.
//!
//! A child is referenced through a [`Link`]: either an in-memory node
//! (`Mem`) — a dirty node of the working tree, or a saved node of a purely
//! in-memory tree — or a persisted node (`Stored`) identified by its IAVL v1
//! node key `(version, nonce)`. A stored link carries the child's hash,
//! height and size, so hashing a dirty parent and AVL rebalancing never
//! have to load an unchanged sibling from disk.

use std::sync::Arc;

use crate::{Error, Hash, Result, Version, inner_hash, leaf_hash};

/// IAVL v1 node key: the version that created the node and a per-version
/// nonce. Encoded big-endian so keys sort by version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey {
    /// Version that created the node.
    pub version: Version,
    /// Nonce, unique within `version`.
    pub nonce: u32,
}

/// Encoded length of a [`NodeKey`].
pub const NODE_KEY_LEN: usize = 12;

impl NodeKey {
    /// Big-endian `version || nonce`.
    #[must_use]
    pub fn to_bytes(self) -> [u8; NODE_KEY_LEN] {
        let mut out = [0u8; NODE_KEY_LEN];
        out[..8].copy_from_slice(&self.version.to_be_bytes());
        out[8..].copy_from_slice(&self.nonce.to_be_bytes());
        out
    }

    /// Inverse of [`Self::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != NODE_KEY_LEN {
            return Err(Error::Corrupt("node key length"));
        }
        let mut version = [0u8; 8];
        version.copy_from_slice(&bytes[..8]);
        let mut nonce = [0u8; 4];
        nonce.copy_from_slice(&bytes[8..]);
        Ok(Self {
            version: Version::from_be_bytes(version),
            nonce: u32::from_be_bytes(nonce),
        })
    }
}

/// Reference to a persisted node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredRef {
    pub nk: NodeKey,
    pub hash: Hash,
    pub height: i8,
    pub size: i64,
}

/// A child pointer.
#[derive(Debug, Clone)]
pub(crate) enum Link {
    Mem(Arc<Node>),
    Stored(StoredRef),
}

impl Link {
    pub fn mem(node: Node) -> Self {
        Self::Mem(Arc::new(node))
    }

    pub fn height(&self) -> i8 {
        match self {
            Self::Mem(node) => node.height(),
            Self::Stored(r) => r.height,
        }
    }

    pub fn size(&self) -> i64 {
        match self {
            Self::Mem(node) => node.size(),
            Self::Stored(r) => r.size,
        }
    }

    /// Hash of the subtree; dirty nodes are hashed as if saved at `version`.
    pub fn hash_for(&self, version: Version) -> Hash {
        match self {
            Self::Mem(node) => node.hash_for(version),
            Self::Stored(r) => r.hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Leaf {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    /// 0 while dirty.
    pub version: Version,
    pub hash: Option<Hash>,
}

#[derive(Debug, Clone)]
pub(crate) struct Inner {
    pub key: Vec<u8>,
    pub height: i8,
    pub size: i64,
    pub left: Link,
    pub right: Link,
    /// 0 while dirty.
    pub version: Version,
    pub hash: Option<Hash>,
}

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Leaf(Leaf),
    Inner(Inner),
}

impl Node {
    pub fn leaf(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self::Leaf(Leaf {
            key,
            value,
            version: 0,
            hash: None,
        })
    }

    pub fn height(&self) -> i8 {
        match self {
            Self::Leaf(_) => 0,
            Self::Inner(inner) => inner.height,
        }
    }

    pub fn size(&self) -> i64 {
        match self {
            Self::Leaf(_) => 1,
            Self::Inner(inner) => inner.size,
        }
    }

    pub fn saved_hash(&self) -> Option<Hash> {
        match self {
            Self::Leaf(leaf) => leaf.hash,
            Self::Inner(inner) => inner.hash,
        }
    }

    /// Saved version, or `None` for a dirty node.
    pub fn saved_version(&self) -> Option<Version> {
        let version = match self {
            Self::Leaf(leaf) => leaf.version,
            Self::Inner(inner) => inner.version,
        };
        (version > 0).then_some(version)
    }

    pub fn hash_for(&self, version: Version) -> Hash {
        if let Some(hash) = self.saved_hash() {
            return hash;
        }
        match self {
            Self::Leaf(leaf) => leaf_hash(version, &leaf.key, &leaf.value),
            Self::Inner(inner) => inner_hash(
                i64::from(inner.height),
                inner.size,
                version,
                inner.left.hash_for(version),
                inner.right.hash_for(version),
            ),
        }
    }
}

/// Builds a dirty inner node over two children.
pub(crate) fn make_inner(key: Vec<u8>, left: Link, right: Link) -> Node {
    Node::Inner(Inner {
        key,
        height: left.height().max(right.height()) + 1,
        size: left.size() + right.size(),
        left,
        right,
        version: 0,
        hash: None,
    })
}

// ---------------------------------------------------------------------------
// Encoding (our own format; only hashes are consensus relevant)
//
//   leaf  = 0x00 | uvarint(len) key | uvarint(len) value
//   inner = 0x01 | height:u8 | uvarint size | uvarint(len) key | ref | ref
//   ref   = uvarint version | uvarint nonce | hash[32] | height:u8 | uvarint size
//
// A node's own version is its node key's version; its hash is carried by
// the reference used to reach it (parent or root record).
// ---------------------------------------------------------------------------

const TAG_LEAF: u8 = 0;
const TAG_INNER: u8 = 1;

pub(crate) fn encode_leaf(key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() + value.len() + 8);
    out.push(TAG_LEAF);
    put_bytes(&mut out, key);
    put_bytes(&mut out, value);
    out
}

pub(crate) fn encode_inner(
    key: &[u8],
    height: i8,
    size: i64,
    left: &StoredRef,
    right: &StoredRef,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() + 2 * 48 + 12);
    out.push(TAG_INNER);
    out.push(height.to_ne_bytes()[0]);
    put_uvarint(&mut out, u64::try_from(size).unwrap_or(0));
    put_bytes(&mut out, key);
    encode_ref(&mut out, left);
    encode_ref(&mut out, right);
    out
}

pub(crate) fn encode_ref(out: &mut Vec<u8>, r: &StoredRef) {
    put_uvarint(out, u64::try_from(r.nk.version).unwrap_or(0));
    put_uvarint(out, u64::from(r.nk.nonce));
    out.extend_from_slice(&r.hash);
    out.push(r.height.to_ne_bytes()[0]);
    put_uvarint(out, u64::try_from(r.size).unwrap_or(0));
}

/// Root record: empty for an empty tree, else one encoded reference.
pub(crate) fn encode_root(root: Option<&StoredRef>) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(r) = root {
        encode_ref(&mut out, r);
    }
    out
}

pub(crate) fn decode_root(bytes: &[u8]) -> Result<Option<StoredRef>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let mut cursor = Cursor { bytes, pos: 0 };
    let r = cursor.stored_ref()?;
    cursor.finish()?;
    Ok(Some(r))
}

/// Decodes a persisted node reached through `r`.
pub(crate) fn decode_node(bytes: &[u8], r: &StoredRef) -> Result<Node> {
    let mut cursor = Cursor { bytes, pos: 0 };
    let node = match cursor.byte()? {
        TAG_LEAF => {
            let key = cursor.bytes()?;
            let value = cursor.bytes()?;
            Node::Leaf(Leaf {
                key,
                value,
                version: r.nk.version,
                hash: Some(r.hash),
            })
        }
        TAG_INNER => {
            let height = i8::from_ne_bytes([cursor.byte()?]);
            let size = i64::try_from(cursor.uvarint()?).map_err(|_| Error::Corrupt("size"))?;
            let key = cursor.bytes()?;
            let left = cursor.stored_ref()?;
            let right = cursor.stored_ref()?;
            Node::Inner(Inner {
                key,
                height,
                size,
                left: Link::Stored(left),
                right: Link::Stored(right),
                version: r.nk.version,
                hash: Some(r.hash),
            })
        }
        _ => return Err(Error::Corrupt("node tag")),
    };
    cursor.finish()?;
    if node.height() != r.height || node.size() != r.size {
        return Err(Error::Corrupt("node does not match its reference"));
    }
    Ok(node)
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_uvarint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

#[allow(clippy::cast_possible_truncation)]
fn put_uvarint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or(Error::Corrupt("truncated node"))?;
        self.pos += 1;
        Ok(b)
    }

    fn uvarint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            value |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Ok(value);
            }
        }
        Err(Error::Corrupt("varint overflow"))
    }

    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Corrupt("truncated node"))?;
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let len = usize::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("length"))?;
        Ok(self.take(len)?.to_vec())
    }

    fn stored_ref(&mut self) -> Result<StoredRef> {
        let version = Version::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("version"))?;
        let nonce = u32::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("nonce"))?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(self.take(32)?);
        let height = i8::from_ne_bytes([self.byte()?]);
        let size = i64::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("size"))?;
        Ok(StoredRef {
            nk: NodeKey { version, nonce },
            hash,
            height,
            size,
        })
    }

    fn finish(&self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::Corrupt("trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn node_and_root_encoding_round_trips() {
        let left = StoredRef {
            nk: NodeKey {
                version: 3,
                nonce: 7,
            },
            hash: [1; 32],
            height: 0,
            size: 1,
        };
        let right = StoredRef {
            nk: NodeKey {
                version: 300,
                nonce: 70_000,
            },
            hash: [2; 32],
            height: 2,
            size: 3,
        };
        let me = StoredRef {
            nk: NodeKey {
                version: 301,
                nonce: 1,
            },
            hash: [3; 32],
            height: 3,
            size: 4,
        };
        let bytes = encode_inner(b"sep", 3, 4, &left, &right);
        let Node::Inner(inner) = decode_node(&bytes, &me).unwrap() else {
            unreachable!()
        };
        assert_eq!(inner.key, b"sep");
        assert_eq!(inner.version, 301);
        assert!(matches!(inner.right, Link::Stored(r) if r == right));
        assert!(decode_node(&bytes[..bytes.len() - 1], &me).is_err());
        let leaf = encode_leaf(b"k", b"");
        assert!(matches!(decode_node(&leaf, &left).unwrap(), Node::Leaf(l) if l.value.is_empty()));
        assert!(decode_node(&leaf, &me).is_err());
        assert_eq!(decode_root(&encode_root(Some(&me))).unwrap(), Some(me));
        assert_eq!(decode_root(&encode_root(None)).unwrap(), None);
        let nk = me.nk;
        assert_eq!(NodeKey::from_bytes(&nk.to_bytes()).unwrap(), nk);
        assert!(NodeKey::from_bytes(&[0; 3]).is_err());
    }
}
