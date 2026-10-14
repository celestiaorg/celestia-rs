//! `mempool/cat/received_buffer.go`: requested txs that arrived ahead of
//! their signer's expected sequence, held until the gap closes.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use super::{MAX_RECEIVED_BUFFER_SIZE, MAX_REQUESTS_PER_PEER};
use crate::types::{CachedTx, TxInfo, TxKey};

/// `bufferedTx`.
#[derive(Debug, Clone)]
pub struct BufferedTx {
    /// The tx.
    pub tx: CachedTx,
    /// Its key.
    pub tx_key: TxKey,
    /// Sender info.
    pub tx_info: TxInfo,
    /// Sending peer (P2P ID).
    pub peer_id: String,
    /// Signer sequence.
    pub sequence: u64,
    /// When it was buffered.
    pub added_at: SystemTime,
}

#[derive(Debug, Default)]
struct Inner {
    buffers: HashMap<Vec<u8>, HashMap<u64, BufferedTx>>,
    count_by_peer: HashMap<String, usize>,
}

/// `receivedTxBuffer`.
#[derive(Debug, Default)]
pub struct ReceivedTxBuffer {
    inner: Mutex<Inner>,
}

impl ReceivedTxBuffer {
    /// `newReceivedTxBuffer`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `add`: false for an empty signer, a peer at capacity, a duplicate
    /// sequence or a full signer buffer.
    pub fn add(
        &self,
        signer: &[u8],
        seq: u64,
        tx: CachedTx,
        tx_key: TxKey,
        tx_info: TxInfo,
        peer_id: &str,
    ) -> bool {
        if signer.is_empty() {
            return false;
        }
        let mut s = self.lock();
        if s.count_by_peer.get(peer_id).copied().unwrap_or(0) >= MAX_REQUESTS_PER_PEER {
            return false;
        }
        let buf = s.buffers.entry(signer.to_vec()).or_default();
        if buf.contains_key(&seq) || buf.len() >= MAX_RECEIVED_BUFFER_SIZE {
            return false;
        }
        buf.insert(
            seq,
            BufferedTx {
                tx,
                tx_key,
                tx_info,
                peer_id: peer_id.to_owned(),
                sequence: seq,
                added_at: SystemTime::now(),
            },
        );
        *s.count_by_peer.entry(peer_id.to_owned()).or_default() += 1;
        true
    }

    /// `get`.
    #[must_use]
    pub fn get(&self, signer: &[u8], seq: u64) -> Option<BufferedTx> {
        if signer.is_empty() {
            return None;
        }
        self.lock()
            .buffers
            .get(signer)
            .and_then(|b| b.get(&seq))
            .cloned()
    }

    /// `removeLowerSeqs`: drops every buffered tx with sequence `<= seq`.
    pub fn remove_lower_seqs(&self, signer: &[u8], seq: u64) {
        if signer.is_empty() {
            return;
        }
        let mut s = self.lock();
        let Some(buf) = s.buffers.get_mut(signer) else {
            return;
        };
        let mut removed_peers = Vec::new();
        buf.retain(|&entry_seq, entry| {
            if entry_seq <= seq {
                if !entry.peer_id.is_empty() {
                    removed_peers.push(entry.peer_id.clone());
                }
                return false;
            }
            true
        });
        let empty = buf.is_empty();
        for p in removed_peers {
            if let Some(c) = s.count_by_peer.get_mut(&p) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    s.count_by_peer.remove(&p);
                }
            }
        }
        if empty {
            s.buffers.remove(signer);
        }
    }

    /// `signerKeys`.
    #[must_use]
    pub fn signer_keys(&self) -> Vec<Vec<u8>> {
        self.lock().buffers.keys().cloned().collect()
    }

    /// `countForPeer`.
    #[must_use]
    pub fn count_for_peer(&self, peer_id: &str) -> usize {
        self.lock().count_by_peer.get(peer_id).copied().unwrap_or(0)
    }
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

    fn mk(name: &str) -> (CachedTx, TxKey) {
        let tx = CachedTx::new(name.as_bytes().to_vec());
        let k = tx.key();
        (tx, k)
    }

    #[test]
    fn add_and_get() {
        let buf = ReceivedTxBuffer::new();
        let (tx, key) = mk("tx1");
        let info = TxInfo {
            sender_id: 1,
            ..TxInfo::default()
        };
        assert!(buf.add(b"signer1", 5, tx.clone(), key, info, "peer1"));
        let got = buf.get(b"signer1", 5).unwrap();
        assert_eq!(got.tx, tx);
        assert_eq!(got.tx_key, key);
        assert_eq!(got.sequence, 5);
        assert_eq!(got.peer_id, "peer1");
        assert!(buf.get(b"signer2", 5).is_none());
        assert!(buf.get(b"signer1", 6).is_none());
        assert!(buf.get(b"", 5).is_none());
    }

    #[test]
    fn duplicates_and_empty_signer() {
        let buf = ReceivedTxBuffer::new();
        let (tx1, k1) = mk("tx1");
        let (tx2, k2) = mk("tx2");
        assert!(buf.add(b"signer1", 5, tx1.clone(), k1, TxInfo::default(), "peer1"));
        assert!(!buf.add(b"signer1", 5, tx2, k2, TxInfo::default(), "peer2"));
        assert_eq!(buf.get(b"signer1", 5).unwrap().tx, tx1);
        assert!(!buf.add(b"", 5, tx1, k1, TxInfo::default(), "peer1"));
    }

    #[test]
    fn max_buffer_per_signer() {
        let buf = ReceivedTxBuffer::new();
        for i in 0..MAX_RECEIVED_BUFFER_SIZE as u64 {
            let (tx, k) = mk(&format!("tx{i}"));
            let peer = format!("peer{}", i % 10);
            assert!(buf.add(b"signer1", i + 100, tx, k, TxInfo::default(), &peer));
        }
        let (tx, k) = mk("overflow");
        assert!(!buf.add(
            b"signer1",
            200,
            tx.clone(),
            k,
            TxInfo::default(),
            "peer-new"
        ));
        assert!(buf.add(b"signer2", 100, tx, k, TxInfo::default(), "peer-new"));
    }

    #[test]
    fn max_buffer_per_peer() {
        let buf = ReceivedTxBuffer::new();
        for i in 0..MAX_REQUESTS_PER_PEER as u64 {
            let (tx, k) = mk(&format!("tx{i}"));
            let signer = format!("signer{i}");
            assert!(buf.add(
                signer.as_bytes(),
                i + 100,
                tx,
                k,
                TxInfo::default(),
                "peer1"
            ));
        }
        assert_eq!(buf.count_for_peer("peer1"), MAX_REQUESTS_PER_PEER);
        let (tx, k) = mk("overflow");
        assert!(!buf.add(
            b"new-signer",
            200,
            tx.clone(),
            k,
            TxInfo::default(),
            "peer1"
        ));
        assert!(buf.add(b"new-signer", 200, tx, k, TxInfo::default(), "peer2"));
    }

    #[test]
    fn remove_lower_seqs() {
        let buf = ReceivedTxBuffer::new();
        for seq in [5u64, 6, 7, 10] {
            let (tx, k) = mk(&format!("tx{seq}"));
            assert!(buf.add(b"signer1", seq, tx, k, TxInfo::default(), "peer1"));
        }
        buf.remove_lower_seqs(b"signer1", 6);
        assert!(buf.get(b"signer1", 5).is_none());
        assert!(buf.get(b"signer1", 6).is_none());
        assert!(buf.get(b"signer1", 7).is_some());
        assert_eq!(buf.count_for_peer("peer1"), 2);
        buf.remove_lower_seqs(b"signer1", 10);
        assert!(buf.signer_keys().is_empty());
        assert_eq!(buf.count_for_peer("peer1"), 0);
    }
}
