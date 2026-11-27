//! Tree nodes, child links and the on-disk node encoding.
//!
//! A child is referenced through a [`Link`]: either an in-memory node
//! (`Mem`) — a dirty node of the working tree, or a saved node of a purely
//! in-memory tree — or a persisted node (`Stored`) identified by its IAVL v1
//! node key `(version, nonce)`. A stored link carries the child's hash,
//! height and size, so hashing a dirty parent and AVL rebalancing never
//! have to load an unchanged sibling from disk.
//!
//! A compact persisted parent stores only its children's node keys. Decoding
//! it gives `Lazy` links: the child's hash, height and size are read from the
//! child record on first use (see [`Link::resolve`]), so a read that walks
//! one path does not read the records of the siblings it passes.
//!
//! Invariant: a dirty node never holds an unresolved `Lazy` link. The
//! mutator converts every link it moves into a dirty node with
//! [`Link::settle`], and those links are needed for the new node's hash.

use std::sync::{Arc, OnceLock};

use crate::nodedb::NodeDb;
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

/// Reference to a persisted node known by key only, resolved on first use.
#[derive(Debug, Clone)]
pub(crate) struct LazyRef {
    pub nk: NodeKey,
    resolved: OnceLock<StoredRef>,
}

impl LazyRef {
    pub fn new(nk: NodeKey) -> Self {
        Self {
            nk,
            resolved: OnceLock::new(),
        }
    }

    pub fn get(&self) -> Option<&StoredRef> {
        self.resolved.get()
    }

    /// Records the reference read from the child record.
    pub fn set(&self, r: StoredRef) -> Result<StoredRef> {
        if r.nk != self.nk {
            return Err(Error::Corrupt("resolved reference has another node key"));
        }
        Ok(*self.resolved.get_or_init(|| r))
    }
}

/// A child pointer.
#[derive(Debug, Clone)]
pub(crate) enum Link {
    Mem(Arc<Node>),
    Stored(StoredRef),
    Lazy(LazyRef),
}

impl Link {
    pub fn mem(node: Node) -> Self {
        Self::Mem(Arc::new(node))
    }

    /// Node key of a persisted child.
    pub fn node_key(&self) -> Option<NodeKey> {
        match self {
            Self::Mem(_) => None,
            Self::Stored(r) => Some(r.nk),
            Self::Lazy(l) => Some(l.nk),
        }
    }

    /// Reads the reference of a `Lazy` link if it is not known yet.
    pub fn resolve(&self, ndb: Option<&NodeDb>) -> Result<&Self> {
        if let Self::Lazy(l) = self
            && l.get().is_none()
        {
            let ndb = ndb.ok_or(Error::Corrupt("stored node without node db"))?;
            l.set(ndb.child_ref(l.nk)?)?;
        }
        Ok(self)
    }

    /// The same child as a `Mem` or `Stored` link, for a dirty node.
    pub fn settle(&self, ndb: Option<&NodeDb>) -> Result<Self> {
        Ok(match self.resolve(ndb)? {
            Self::Lazy(l) => Self::Stored(*l.get().ok_or(UNRESOLVED)?),
            other => other.clone(),
        })
    }

    pub fn stored(&self) -> Option<&StoredRef> {
        match self {
            Self::Mem(_) => None,
            Self::Stored(r) => Some(r),
            Self::Lazy(l) => l.get(),
        }
    }

    pub fn height(&self) -> Result<i8> {
        match self {
            Self::Mem(node) => Ok(node.height()),
            _ => Ok(self.stored().ok_or(UNRESOLVED)?.height),
        }
    }

    pub fn size(&self) -> Result<i64> {
        match self {
            Self::Mem(node) => Ok(node.size()),
            _ => Ok(self.stored().ok_or(UNRESOLVED)?.size),
        }
    }

    /// Hash of the subtree; dirty nodes are hashed as if saved at `version`.
    pub fn hash_for(&self, version: Version) -> Result<Hash> {
        match self {
            Self::Mem(node) => node.hash_for(version),
            _ => Ok(self.stored().ok_or(UNRESOLVED)?.hash),
        }
    }
}

const UNRESOLVED: Error = Error::Corrupt("child reference used before it was read");

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

    /// Saved hash, or the hash a dirty node gets when saved at `version`.
    /// Fails only if a dirty node breaks the invariant in the module docs.
    pub fn hash_for(&self, version: Version) -> Result<Hash> {
        if let Some(hash) = self.saved_hash() {
            return Ok(hash);
        }
        Ok(match self {
            Self::Leaf(leaf) => leaf_hash(version, &leaf.key, &leaf.value),
            Self::Inner(inner) => inner_hash(
                i64::from(inner.height),
                inner.size,
                version,
                inner.left.hash_for(version)?,
                inner.right.hash_for(version)?,
            ),
        })
    }
}

/// Builds a dirty inner node over two settled children (see
/// [`Link::settle`]).
pub(crate) fn make_inner(key: Vec<u8>, left: Link, right: Link) -> Result<Node> {
    Ok(Node::Inner(Inner {
        key,
        height: left.height()?.max(right.height()?) + 1,
        size: left.size()? + right.size()?,
        left,
        right,
        version: 0,
        hash: None,
    }))
}

// ---------------------------------------------------------------------------
// Encoding (our own format; only hashes are consensus relevant)
//
//   leaf  = 0x00 | uvarint(len) key | uvarint(len) value
//   inner = 0x02 | height:u8 | uvarint size | hash[32] | uvarint(len) key | nk | nk
//   nk = uvarint version | uvarint nonce
//   legacy inner = 0x01 | height:u8 | uvarint size | uvarint(len) key | ref | ref
//   ref   = uvarint version | uvarint nonce | hash[32] | height:u8 | uvarint size
//
// The node key supplies the version. Root records still carry a full ref.
// ---------------------------------------------------------------------------

const TAG_LEAF: u8 = 0;
const TAG_LEGACY_INNER: u8 = 1;
const TAG_INNER: u8 = 2;

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
    hash: &Hash,
    left: NodeKey,
    right: NodeKey,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() + 56);
    out.push(TAG_INNER);
    out.push(height.to_ne_bytes()[0]);
    put_uvarint(&mut out, u64::try_from(size).unwrap_or(0));
    out.extend_from_slice(hash);
    put_bytes(&mut out, key);
    put_node_key(&mut out, left);
    put_node_key(&mut out, right);
    out
}

fn put_node_key(out: &mut Vec<u8>, nk: NodeKey) {
    put_uvarint(out, u64::try_from(nk.version).unwrap_or(0));
    put_uvarint(out, u64::from(nk.nonce));
}

fn encode_ref(out: &mut Vec<u8>, r: &StoredRef) {
    put_node_key(out, r.nk);
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

/// Resolve the reference from one read, including when a new parent points
/// at a node written by an older binary. Legacy parents contain both hashes.
pub(crate) fn read_ref(bytes: &[u8], nk: NodeKey) -> Result<StoredRef> {
    let mut cursor = Cursor { bytes, pos: 0 };
    let tag = cursor.byte()?;
    if tag == TAG_LEAF {
        let key = cursor.byte_slice()?;
        let value = cursor.byte_slice()?;
        cursor.finish()?;
        return Ok(StoredRef {
            nk,
            height: 0,
            size: 1,
            hash: leaf_hash(nk.version, key, value),
        });
    }
    let (height, size) = cursor.dimensions()?;
    let hash = match tag {
        TAG_INNER => cursor.hash()?,
        TAG_LEGACY_INNER => {
            cursor.byte_slice()?;
            let left = cursor.stored_ref()?;
            let right = cursor.stored_ref()?;
            cursor.finish()?;
            inner_hash(i64::from(height), size, nk.version, left.hash, right.hash)
        }
        _ => return Err(Error::Corrupt("node tag")),
    };
    Ok(StoredRef {
        nk,
        hash,
        height,
        size,
    })
}

/// Decodes a persisted node reached through `r`. `child` makes the link of
/// a child that a compact record names by key only.
pub(crate) fn decode_node(
    bytes: &[u8],
    r: &StoredRef,
    mut child: impl FnMut(NodeKey) -> Result<Link>,
) -> Result<Node> {
    let mut cursor = Cursor { bytes, pos: 0 };
    let tag = cursor.byte()?;
    let node = match tag {
        TAG_LEAF => {
            let key = cursor.byte_slice()?.to_vec();
            let value = cursor.byte_slice()?.to_vec();
            Node::Leaf(Leaf {
                key,
                value,
                version: r.nk.version,
                hash: Some(r.hash),
            })
        }
        TAG_INNER | TAG_LEGACY_INNER => {
            let (height, size) = cursor.dimensions()?;
            if tag == TAG_INNER && cursor.hash()? != r.hash {
                return Err(Error::Corrupt("node hash does not match its reference"));
            }
            let key = cursor.byte_slice()?.to_vec();
            let (left, right) = if tag == TAG_LEGACY_INNER {
                (
                    Link::Stored(cursor.stored_ref()?),
                    Link::Stored(cursor.stored_ref()?),
                )
            } else {
                (child(cursor.node_key()?)?, child(cursor.node_key()?)?)
            };
            Node::Inner(Inner {
                key,
                height,
                size,
                left,
                right,
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

impl<'a> Cursor<'a> {
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

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Corrupt("truncated node"))?;
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn byte_slice(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("length"))?;
        self.take(len)
    }

    fn node_key(&mut self) -> Result<NodeKey> {
        let version = Version::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("version"))?;
        let nonce = u32::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("nonce"))?;
        Ok(NodeKey { version, nonce })
    }

    fn hash(&mut self) -> Result<Hash> {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(self.take(32)?);
        Ok(hash)
    }

    fn dimensions(&mut self) -> Result<(i8, i64)> {
        let height = i8::from_ne_bytes([self.byte()?]);
        let size = i64::try_from(self.uvarint()?).map_err(|_| Error::Corrupt("size"))?;
        Ok((height, size))
    }

    fn stored_ref(&mut self) -> Result<StoredRef> {
        let nk = self.node_key()?;
        let hash = self.hash()?;
        let (height, size) = self.dimensions()?;
        Ok(StoredRef {
            nk,
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
    fn legacy_inner_supplies_its_hash_to_a_compact_parent() {
        // Original tag-1 layout with a one-byte separator and two leaf refs.
        let mut legacy = vec![1, 1, 2, 1, b'b', 1, 1];
        legacy.extend([0xab; 32]);
        legacy.extend([0, 1, 1, 2]);
        legacy.extend([0xcd; 32]);
        legacy.extend([0, 1]);
        let nk = NodeKey {
            version: 1,
            nonce: 3,
        };
        let reference = read_ref(&legacy, nk).unwrap();
        assert_eq!(reference.hash, inner_hash(1, 2, 1, [0xab; 32], [0xcd; 32]));
        let Node::Inner(node) = decode_node(&legacy, &reference, |_| unreachable!()).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(node.key, b"b");
        assert_eq!(node.left.hash_for(1).unwrap(), [0xab; 32]);
        assert_eq!(node.right.hash_for(1).unwrap(), [0xcd; 32]);
    }

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
        let bytes = encode_inner(b"sep", 3, 4, &me.hash, left.nk, right.nk);
        assert_eq!(read_ref(&bytes, me.nk).unwrap(), me);
        let child = |nk| Ok(Link::Stored(if nk == left.nk { left } else { right }));
        let Node::Inner(inner) = decode_node(&bytes, &me, child).unwrap() else {
            unreachable!()
        };
        assert_eq!(inner.key, b"sep");
        assert_eq!(inner.version, 301);
        assert!(matches!(inner.right, Link::Stored(r) if r == right));
        assert!(decode_node(&bytes[..bytes.len() - 1], &me, child).is_err());
        let leaf = encode_leaf(b"k", b"");
        assert!(
            matches!(decode_node(&leaf, &left, child).unwrap(), Node::Leaf(l) if l.value.is_empty())
        );
        assert!(decode_node(&leaf, &me, child).is_err());
        assert_eq!(decode_root(&encode_root(Some(&me))).unwrap(), Some(me));
        assert_eq!(decode_root(&encode_root(None)).unwrap(), None);
        let nk = me.nk;
        assert_eq!(NodeKey::from_bytes(&nk.to_bytes()).unwrap(), nk);
        assert!(NodeKey::from_bytes(&[0; 3]).is_err());
    }
}
