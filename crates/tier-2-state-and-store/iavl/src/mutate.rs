//! AVL insert/remove with Go IAVL's exact rebalancing (path copying).
//!
//! Every node that is rebuilt gets a fresh dirty copy; when the replaced
//! node is persisted its node key is recorded as an orphan of the working
//! version. Unchanged subtrees are kept as their original links (never
//! loaded unless the algorithm has to inspect them).

use std::sync::Arc;

use crate::node::{Inner, Link, Node, NodeKey, make_inner};
use crate::nodedb::NodeDb;
use crate::{Error, Result};

pub(crate) struct Mutator<'a> {
    pub ndb: Option<&'a NodeDb>,
    pub orphans: &'a mut Vec<NodeKey>,
}

pub(crate) fn load(ndb: Option<&NodeDb>, link: &Link) -> Result<Arc<Node>> {
    match link {
        Link::Mem(node) => Ok(Arc::clone(node)),
        Link::Stored(r) => ndb
            .ok_or(Error::Corrupt("stored node without node db"))?
            .load(r),
    }
}

pub(crate) struct RemoveOutcome {
    pub node: Option<Link>,
    pub new_key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
    pub removed: bool,
}

impl Mutator<'_> {
    fn load(&self, link: &Link) -> Result<Arc<Node>> {
        load(self.ndb, link)
    }

    /// Records that `link` is replaced in the working tree.
    fn discard(&mut self, link: &Link) {
        if let Link::Stored(r) = link {
            self.orphans.push(r.nk);
        }
    }

    /// Inserts or replaces; returns the new subtree, whether an existing key
    /// was updated (shape unchanged) and the previous value.
    pub fn set(
        &mut self,
        link: &Link,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(Link, bool, Option<Vec<u8>>)> {
        let node = self.load(link)?;
        match &*node {
            Node::Leaf(leaf) => match key.as_slice().cmp(leaf.key.as_slice()) {
                core::cmp::Ordering::Less => Ok((
                    Link::mem(make_inner(
                        leaf.key.clone(),
                        Link::mem(Node::leaf(key, value)),
                        link.clone(),
                    )),
                    false,
                    None,
                )),
                core::cmp::Ordering::Greater => Ok((
                    Link::mem(make_inner(
                        key.clone(),
                        link.clone(),
                        Link::mem(Node::leaf(key, value)),
                    )),
                    false,
                    None,
                )),
                core::cmp::Ordering::Equal => {
                    self.discard(link);
                    Ok((
                        Link::mem(Node::leaf(key, value)),
                        true,
                        Some(leaf.value.clone()),
                    ))
                }
            },
            Node::Inner(inner) => {
                self.discard(link);
                let mut left = inner.left.clone();
                let mut right = inner.right.clone();
                let (updated, old) = if key.as_slice() < inner.key.as_slice() {
                    let (l, updated, old) = self.set(&left, key, value)?;
                    left = l;
                    (updated, old)
                } else {
                    let (r, updated, old) = self.set(&right, key, value)?;
                    right = r;
                    (updated, old)
                };
                if updated {
                    return Ok((
                        Link::mem(Node::Inner(Inner {
                            key: inner.key.clone(),
                            height: inner.height,
                            size: inner.size,
                            left,
                            right,
                            version: 0,
                            hash: None,
                        })),
                        true,
                        old,
                    ));
                }
                let node = self.balance(make_inner(inner.key.clone(), left, right))?;
                Ok((Link::mem(node), false, old))
            }
        }
    }

    pub fn remove(&mut self, link: &Link, key: &[u8]) -> Result<RemoveOutcome> {
        let node = self.load(link)?;
        let unchanged = || RemoveOutcome {
            node: Some(link.clone()),
            new_key: None,
            value: None,
            removed: false,
        };
        match &*node {
            Node::Leaf(leaf) => {
                if key == leaf.key.as_slice() {
                    self.discard(link);
                    Ok(RemoveOutcome {
                        node: None,
                        new_key: None,
                        value: Some(leaf.value.clone()),
                        removed: true,
                    })
                } else {
                    Ok(unchanged())
                }
            }
            Node::Inner(inner) => {
                if key < inner.key.as_slice() {
                    let result = self.remove(&inner.left, key)?;
                    if !result.removed {
                        return Ok(unchanged());
                    }
                    self.discard(link);
                    let Some(left) = result.node else {
                        return Ok(RemoveOutcome {
                            node: Some(inner.right.clone()),
                            new_key: Some(inner.key.clone()),
                            value: result.value,
                            removed: true,
                        });
                    };
                    let node =
                        self.balance(make_inner(inner.key.clone(), left, inner.right.clone()))?;
                    Ok(RemoveOutcome {
                        node: Some(Link::mem(node)),
                        new_key: result.new_key,
                        value: result.value,
                        removed: true,
                    })
                } else {
                    let result = self.remove(&inner.right, key)?;
                    if !result.removed {
                        return Ok(unchanged());
                    }
                    self.discard(link);
                    let Some(right) = result.node else {
                        return Ok(RemoveOutcome {
                            node: Some(inner.left.clone()),
                            new_key: None,
                            value: result.value,
                            removed: true,
                        });
                    };
                    let node_key = result.new_key.unwrap_or_else(|| inner.key.clone());
                    let node = self.balance(make_inner(node_key, inner.left.clone(), right))?;
                    Ok(RemoveOutcome {
                        node: Some(Link::mem(node)),
                        new_key: None,
                        value: result.value,
                        removed: true,
                    })
                }
            }
        }
    }

    fn child_balance(&self, link: &Link) -> Result<i16> {
        Ok(match &*self.load(link)? {
            Node::Leaf(_) => 0,
            Node::Inner(inner) => i16::from(inner.left.height()) - i16::from(inner.right.height()),
        })
    }

    pub fn balance(&mut self, node: Node) -> Result<Node> {
        let Node::Inner(mut inner) = node else {
            return Ok(node);
        };
        let balance = i16::from(inner.left.height()) - i16::from(inner.right.height());
        if balance > 1 {
            if self.child_balance(&inner.left)? >= 0 {
                return self.rotate_right(inner);
            }
            inner.left = self.rotate_left_link(&inner.left)?;
            return self.rotate_right(inner);
        }
        if balance < -1 {
            if self.child_balance(&inner.right)? <= 0 {
                return self.rotate_left(inner);
            }
            inner.right = self.rotate_right_link(&inner.right)?;
            return self.rotate_left(inner);
        }
        Ok(Node::Inner(inner))
    }

    fn rotate_right_link(&mut self, link: &Link) -> Result<Link> {
        let node = self.load(link)?;
        let Node::Inner(inner) = &*node else {
            return Ok(link.clone());
        };
        self.discard(link);
        Ok(Link::mem(self.rotate_right(inner.clone())?))
    }

    fn rotate_left_link(&mut self, link: &Link) -> Result<Link> {
        let node = self.load(link)?;
        let Node::Inner(inner) = &*node else {
            return Ok(link.clone());
        };
        self.discard(link);
        Ok(Link::mem(self.rotate_left(inner.clone())?))
    }

    /// `node` itself is already a working copy; its left child is rebuilt.
    fn rotate_right(&mut self, node: Inner) -> Result<Node> {
        let left = self.load(&node.left)?;
        let Node::Inner(l) = &*left else {
            return Ok(Node::Inner(Inner {
                version: 0,
                hash: None,
                ..node
            }));
        };
        self.discard(&node.left);
        let old_root = make_inner(node.key, l.right.clone(), node.right);
        Ok(make_inner(
            l.key.clone(),
            l.left.clone(),
            Link::mem(old_root),
        ))
    }

    fn rotate_left(&mut self, node: Inner) -> Result<Node> {
        let right = self.load(&node.right)?;
        let Node::Inner(r) = &*right else {
            return Ok(Node::Inner(Inner {
                version: 0,
                hash: None,
                ..node
            }));
        };
        self.discard(&node.right);
        let old_root = make_inner(node.key, node.left, r.left.clone());
        Ok(make_inner(
            r.key.clone(),
            Link::mem(old_root),
            r.right.clone(),
        ))
    }
}
