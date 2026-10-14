//! `mempool/cat/sticky_peers.go`: rendezvous (highest random weight)
//! ranking of peers per signer, so one signer's SeenTx announcements keep
//! flowing through the same peers.

use std::collections::HashMap;
use std::sync::Arc;

use super::peers::MempoolPeer;

/// `stickyHashNamespace`.
pub const STICKY_HASH_NAMESPACE: &str = "cat/sticky/v1";

/// `stickyPeer`.
#[derive(Clone)]
pub struct StickyPeer {
    /// Mempool ID.
    pub id: u16,
    /// The peer.
    pub peer: Arc<dyn MempoolPeer>,
}

impl std::fmt::Debug for StickyPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StickyPeer")
            .field("id", &self.id)
            .field("peer", &self.peer.id())
            .finish()
    }
}

/// `stickyScore64`: `sha256(namespace || salt || signer || 0x00 ||
/// peerID)[:8]` big-endian.
#[must_use]
pub fn sticky_score64(signer: &[u8], peer_id: &str, salt: &[u8]) -> u64 {
    let mut h = hash::Sha256Hasher::new();
    h.update(STICKY_HASH_NAMESPACE.as_bytes());
    if !salt.is_empty() {
        h.update(salt);
    }
    if !signer.is_empty() {
        h.update(signer);
    }
    h.update(&[0]);
    h.update(peer_id.as_bytes());
    let sum = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&sum[..8]);
    u64::from_be_bytes(b)
}

/// `selectStickyPeers`: up to `limit` peers, highest score first (ties:
/// peer ID, then mempool ID).
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn select_sticky_peers(
    signer: &[u8],
    peers: &HashMap<u16, Arc<dyn MempoolPeer>>,
    limit: usize,
    salt: &[u8],
) -> Vec<StickyPeer> {
    if limit == 0 || peers.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(StickyPeer, String, u64)> = peers
        .iter()
        .map(|(&id, peer)| {
            let pid = peer.id();
            let score = sticky_score64(signer, &pid, salt);
            (
                StickyPeer {
                    id,
                    peer: peer.clone(),
                },
                pid,
                score,
            )
        })
        .collect();
    scored.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(p, _, _)| p).collect()
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]
mod tests {
    use super::*;
    use crate::cat::peers::PeerState;

    struct Named(String);

    impl MempoolPeer for Named {
        fn id(&self) -> String {
            self.0.clone()
        }
        fn is_persistent(&self) -> bool {
            false
        }
        fn peer_state(&self) -> Option<Arc<dyn PeerState>> {
            None
        }
        fn send(&self, _: u8, _: Vec<u8>) -> bool {
            true
        }
        fn try_send(&self, _: u8, _: Vec<u8>) -> bool {
            true
        }
    }

    fn peers(ids: &[(u16, &str)]) -> HashMap<u16, Arc<dyn MempoolPeer>> {
        ids.iter()
            .map(|&(i, n)| (i, Arc::new(Named(n.into())) as Arc<dyn MempoolPeer>))
            .collect()
    }

    fn ids(p: &[StickyPeer]) -> Vec<u16> {
        p.iter().map(|s| s.id).collect()
    }

    #[test]
    fn limit_and_empty() {
        assert!(select_sticky_peers(b"signer", &HashMap::new(), 5, b"").is_empty());
        assert!(select_sticky_peers(b"signer", &peers(&[(1, "peer-1")]), 0, b"").is_empty());
        let out = select_sticky_peers(b"signer", &peers(&[(1, "peer-1"), (2, "peer-2")]), 5, b"");
        let mut got = ids(&out);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn deterministic() {
        let all: Vec<(u16, String)> = (1..=8).map(|i| (i, format!("peer-{i}"))).collect();
        let refs: Vec<(u16, &str)> = all.iter().map(|(i, s)| (*i, s.as_str())).collect();
        let p = peers(&refs);
        let first = select_sticky_peers(b"signer-A", &p, 3, b"salt");
        assert_eq!(first.len(), 3);
        let second = select_sticky_peers(b"signer-A", &p, 3, b"salt");
        assert_eq!(ids(&first), ids(&second));
        let mut rev = refs.clone();
        rev.reverse();
        let third = select_sticky_peers(b"signer-A", &peers(&rev), 3, b"salt");
        assert_eq!(ids(&first), ids(&third));
    }

    #[test]
    fn different_salt() {
        let p = peers(&[(1, "peer-1"), (2, "peer-2"), (3, "peer-3"), (4, "peer-4")]);
        let a = select_sticky_peers(b"signer-A", &p, 3, b"salt-a");
        let b = select_sticky_peers(b"signer-A", &p, 3, b"salt-b");
        assert_eq!((a.len(), b.len()), (3, 3));
        assert_ne!(ids(&a), ids(&b));
    }

    #[test]
    fn score_depends_on_signer_and_salt() {
        let a = sticky_score64(b"signer-A", "peer-1", b"");
        assert_ne!(a, sticky_score64(b"signer-B", "peer-1", b""));
        assert_ne!(a, sticky_score64(b"signer-A", "peer-1", b"salt"));
    }

    #[test]
    fn score_matches_go_construction() {
        // sha256("cat/sticky/v1" || "salt" || "signer" || 0x00 || "peer")[:8]
        let mut pre = b"cat/sticky/v1saltsigner".to_vec();
        pre.push(0);
        pre.extend_from_slice(b"peer");
        let h = hash::sha256(&pre);
        let want = u64::from_be_bytes(h[..8].try_into().unwrap());
        assert_eq!(sticky_score64(b"signer", "peer", b"salt"), want);
    }
}
