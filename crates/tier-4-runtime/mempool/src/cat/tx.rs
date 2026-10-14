//! `mempool/cat/tx.go`: the wrapped tx and the per-signer tx set.
//!
//! Go identifies txs and sets by pointer; here txs are `Arc<WrappedTx>`
//! compared with `Arc::ptr_eq`. Arithmetic on priorities and gas wraps like
//! Go's `int64`.

use std::sync::Arc;
use std::time::SystemTime;

use super::go_search;
use crate::types::{CachedTx, TxKey};

/// `wrappedTx`: a tx with the metadata the pool indexes it by.
#[derive(Debug, Clone)]
pub struct WrappedTx {
    /// The tx.
    pub tx: CachedTx,
    /// Height at which the tx was first checked (TTL by blocks).
    pub height: i64,
    /// Arrival time (TTL by time, tie-breaks).
    pub timestamp: SystemTime,
    /// App: gas wanted.
    pub gas_wanted: i64,
    /// App: priority.
    pub priority: i64,
    /// App: signer address.
    pub sender: Vec<u8>,
    /// App: signer sequence.
    pub sequence: u64,
    /// Submitted through RPC.
    pub from_broadcast: bool,
}

impl WrappedTx {
    /// `newWrappedTx` (timestamp: now).
    #[must_use]
    pub fn new(
        tx: CachedTx,
        height: i64,
        gas_wanted: i64,
        priority: i64,
        sender: Vec<u8>,
        sequence: u64,
        from_broadcast: bool,
    ) -> Self {
        Self {
            tx,
            height,
            timestamp: SystemTime::now(),
            gas_wanted,
            priority,
            sender,
            sequence,
            from_broadcast,
        }
    }

    /// Raw size in bytes.
    #[must_use]
    pub fn size(&self) -> i64 {
        i64::try_from(self.tx.len()).unwrap_or(i64::MAX)
    }

    /// The tx key.
    #[must_use]
    pub fn key(&self) -> TxKey {
        self.tx.key()
    }
}

/// `txSet`: one signer's txs ordered by sequence (then arrival), with a
/// gas-weighted aggregated priority.
#[derive(Debug, Clone)]
pub struct TxSet {
    /// Store-assigned identity (Go compares set pointers).
    pub(crate) id: u64,
    /// The signer.
    pub signer: Vec<u8>,
    /// Txs, ascending by sequence.
    pub txs: Vec<Arc<WrappedTx>>,
    /// `weightedPrioritySum / totalGasWanted` (0 without gas).
    pub aggregated_priority: i64,
    /// Total raw bytes.
    pub bytes: i64,
    /// Earliest arrival time.
    pub first_timestamp: SystemTime,
    /// Lowest check height.
    pub first_height: i64,
    /// Sum of gas wanted.
    pub total_gas_wanted: i64,
    /// Sum of `priority * gas_wanted`.
    pub weighted_priority_sum: i64,
}

impl Default for TxSet {
    fn default() -> Self {
        Self {
            id: 0,
            signer: Vec::new(),
            txs: Vec::new(),
            aggregated_priority: 0,
            bytes: 0,
            first_timestamp: SystemTime::UNIX_EPOCH,
            first_height: 0,
            total_gas_wanted: 0,
            weighted_priority_sum: 0,
        }
    }
}

impl TxSet {
    /// `newTxSet(wtx...)`.
    #[must_use]
    pub fn new(txs: &[Arc<WrappedTx>]) -> Self {
        let mut set = Self::default();
        for w in txs {
            set.add_tx(w.clone());
        }
        set
    }

    /// `addTxToSet`: inserts keeping sequence order (ties: earlier arrival
    /// first) and updates the aggregates.
    pub fn add_tx(&mut self, wtx: Arc<WrappedTx>) {
        if self.txs.is_empty() {
            self.signer.clone_from(&wtx.sender);
            self.total_gas_wanted = self.total_gas_wanted.wrapping_add(wtx.gas_wanted);
            self.weighted_priority_sum = self
                .weighted_priority_sum
                .wrapping_add(wtx.priority.wrapping_mul(wtx.gas_wanted));
            self.bytes = self.bytes.wrapping_add(wtx.size());
            self.first_timestamp = wtx.timestamp;
            self.first_height = wtx.height;
            // The app need not return positive gas; avoid dividing by zero.
            if self.total_gas_wanted > 0 {
                self.aggregated_priority = self
                    .weighted_priority_sum
                    .wrapping_div(self.total_gas_wanted);
            }
            self.txs.push(wtx);
            return;
        }
        let idx = go_search(self.txs.len(), |i| {
            if self.txs[i].sequence == wtx.sequence {
                return wtx.timestamp < self.txs[i].timestamp;
            }
            self.txs[i].sequence > wtx.sequence
        });
        self.total_gas_wanted = self.total_gas_wanted.wrapping_add(wtx.gas_wanted);
        self.weighted_priority_sum = self
            .weighted_priority_sum
            .wrapping_add(wtx.priority.wrapping_mul(wtx.gas_wanted));
        if self.total_gas_wanted > 0 {
            self.aggregated_priority = self
                .weighted_priority_sum
                .wrapping_div(self.total_gas_wanted);
        }
        self.bytes = self.bytes.wrapping_add(wtx.size());
        if wtx.timestamp < self.first_timestamp {
            self.first_timestamp = wtx.timestamp;
        }
        if wtx.height < self.first_height {
            self.first_height = wtx.height;
        }
        self.txs.insert(idx, wtx);
    }

    /// `removeTx`: removes `wtx` (by identity) and recomputes the
    /// aggregates.
    pub fn remove_tx(&mut self, wtx: &Arc<WrappedTx>) -> bool {
        let Some(i) = self.txs.iter().position(|t| Arc::ptr_eq(t, wtx)) else {
            return false;
        };
        self.bytes = self.bytes.wrapping_sub(wtx.size());
        self.total_gas_wanted = self.total_gas_wanted.wrapping_sub(wtx.gas_wanted);
        self.weighted_priority_sum = self
            .weighted_priority_sum
            .wrapping_sub(wtx.priority.wrapping_mul(wtx.gas_wanted));
        self.txs.remove(i);
        if self.txs.is_empty() || self.total_gas_wanted <= 0 {
            self.aggregated_priority = 0;
            self.first_timestamp = SystemTime::UNIX_EPOCH;
            self.first_height = 0;
            return true;
        }
        let mut earliest = self.txs[0].timestamp;
        let mut lowest = self.txs[0].height;
        for t in &self.txs {
            if t.timestamp < earliest {
                earliest = t.timestamp;
            }
            if t.height < lowest {
                lowest = t.height;
            }
        }
        self.first_timestamp = earliest;
        self.first_height = lowest;
        self.aggregated_priority = self
            .weighted_priority_sum
            .wrapping_div(self.total_gas_wanted);
        true
    }

    /// `sliceTxsByBytesAndGas`. Go never accumulates the bytes/gas used
    /// inside the loop, so each tx is compared against the whole budget on
    /// its own and the loop stops at the first tx that does not fit
    /// (reproduced as is); a negative byte budget with a gas limit yields
    /// an empty set.
    #[must_use]
    pub fn slice_by_bytes_and_gas(&self, num_bytes: i64, num_gas: i64) -> TxSet {
        if num_bytes == 0 || num_gas == 0 {
            return TxSet::default();
        }
        if (num_bytes < 0 || num_bytes >= self.bytes)
            && (num_gas < 0 || num_gas >= self.total_gas_wanted)
        {
            return self.clone();
        }
        let bytes_used: i64 = 0;
        let total_gas_wanted: i64 = 0;
        let mut out = TxSet::default();
        for tx in &self.txs {
            let tx_size = tx.size();
            if bytes_used.wrapping_add(tx_size) > num_bytes
                || (num_gas >= 0 && total_gas_wanted.wrapping_add(tx.gas_wanted) > num_gas)
            {
                break;
            }
            out.add_tx(tx.clone());
        }
        out
    }

    /// `rawTxs`.
    #[must_use]
    pub fn raw_txs(&self) -> Vec<CachedTx> {
        self.txs.iter().map(|t| t.tx.clone()).collect()
    }
}

/// `aggregatePriorityAcrossSets`.
#[must_use]
pub fn aggregate_priority_across_sets(sets: &[TxSet]) -> i64 {
    let mut total_gas: i64 = 0;
    let mut weighted: i64 = 0;
    for s in sets {
        total_gas = total_gas.wrapping_add(s.total_gas_wanted);
        weighted = weighted.wrapping_add(s.weighted_priority_sum);
    }
    if total_gas > 0 {
        return weighted.wrapping_div(total_gas);
    }
    0
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
    use std::time::Duration;

    use super::*;

    fn wtx(tx: &str, height: i64, gas: i64, priority: i64, signer: &[u8], seq: u64) -> WrappedTx {
        WrappedTx::new(
            CachedTx::new(tx.as_bytes().to_vec()),
            height,
            gas,
            priority,
            signer.to_vec(),
            seq,
            false,
        )
    }

    #[test]
    fn zero_gas_does_not_panic() {
        let w = Arc::new(wtx("zero-gas tx", 0, 0, 10, b"", 0));
        let set = TxSet::new(&[w]);
        assert_eq!(set.aggregated_priority, 0);
    }

    #[test]
    fn add_remove_properties() {
        let base = SystemTime::now();
        let mk = |tx: &str, h, gas, prio, seq, secs| {
            let mut w = wtx(tx, h, gas, prio, b"S", seq);
            w.timestamp = base + Duration::from_secs(secs);
            Arc::new(w)
        };
        let w1 = mk("tx1", 10, 2, 10, 1, 1);
        let w2 = mk("tx2", 12, 3, 4, 2, 2);
        let w3 = mk("tx3", 15, 5, 6, 3, 3);
        let mut set = TxSet::new(std::slice::from_ref(&w2));
        set.add_tx(w1.clone());
        set.add_tx(w3.clone());
        let seqs: Vec<u64> = set.txs.iter().map(|t| t.sequence).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!(set.first_timestamp, w1.timestamp);
        // (10*2 + 4*3 + 6*5) / 10
        assert_eq!(set.aggregated_priority, 6);

        assert!(set.remove_tx(&w1));
        assert_eq!(set.txs.len(), 2);
        assert_eq!(set.first_timestamp, w2.timestamp);
        assert_eq!(set.aggregated_priority, 5);

        assert!(set.remove_tx(&w2));
        assert_eq!(set.first_timestamp, w3.timestamp);
        assert_eq!(set.aggregated_priority, 6);

        assert!(set.remove_tx(&w3));
        assert!(set.txs.is_empty());
        assert_eq!(set.first_timestamp, SystemTime::UNIX_EPOCH);
        assert_eq!(set.aggregated_priority, 0);
    }

    #[test]
    fn first_height_recalculated_on_remove() {
        let tx1 = Arc::new(wtx("tx1", 100, 1, 10, b"alice", 1));
        let tx2 = Arc::new(wtx("tx2", 150, 1, 10, b"alice", 2));
        let tx3 = Arc::new(wtx("tx3", 120, 1, 10, b"alice", 3));
        let mut set = TxSet::new(std::slice::from_ref(&tx1));
        assert_eq!(set.first_height, 100);
        set.add_tx(tx2.clone());
        set.add_tx(tx3.clone());
        assert_eq!(set.first_height, 100);
        assert!(set.remove_tx(&tx1));
        assert_eq!(set.first_height, 120);
        assert!(set.remove_tx(&tx3));
        assert_eq!(set.first_height, 150);
        assert!(set.remove_tx(&tx2));
        assert_eq!(set.first_height, 0);
    }

    #[test]
    fn slice_ignores_accumulated_usage_like_go() {
        // three 3-byte txs, 1 gas each; a 4-byte budget keeps every tx
        // that individually fits (Go never accumulates bytesUsed).
        let txs: Vec<_> = ["aaa", "bbb", "ccc"]
            .iter()
            .enumerate()
            .map(|(i, t)| Arc::new(wtx(t, 1, 1, 1, b"s", i as u64)))
            .collect();
        let set = TxSet::new(&txs);
        assert_eq!(set.slice_by_bytes_and_gas(4, -1).txs.len(), 3);
        assert_eq!(set.slice_by_bytes_and_gas(2, -1).txs.len(), 0);
        assert_eq!(set.slice_by_bytes_and_gas(-1, 2).txs.len(), 0);
        assert_eq!(set.slice_by_bytes_and_gas(-1, -1).txs.len(), 3);
        assert_eq!(set.slice_by_bytes_and_gas(0, 5).txs.len(), 0);
    }
}
