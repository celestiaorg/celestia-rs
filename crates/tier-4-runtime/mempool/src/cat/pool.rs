//! `mempool/cat/pool.go`: the CAT `TxPool`.
//!
//! The application assigns each tx a priority, gas, signer and sequence in
//! its CheckTx response. Txs are grouped per signer (ordered by sequence)
//! and sets are ordered by gas-weighted aggregated priority: reaping takes
//! sets highest first, eviction removes sets lowest first (highest sequence
//! first within a set). Rejected txs are remembered in an LRU cache;
//! evicted and expired ones in another.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::SystemTime;

use celestia_core_proto::abci::{CheckTxType, ExecTxResult, RequestCheckTx, ResponseCheckTx};
use tokio::sync::Notify;

use super::seen::SeenTracker;
use super::store::Store;
use super::tx::{TxSet, WrappedTx, aggregate_priority_across_sets};
use crate::cache::{LruTxCache, RejectedTxCache};
use crate::errors::MempoolError;
use crate::proxy::AppConnMempool;
use crate::types::{
    CachedTx, Mempool, MempoolConfig, MempoolLock, PostCheckFunc, PreCheckFunc, TxInfo, TxKey,
};

/// The state Go guards with `TxPool.mtx` (modified once per height).
#[derive(Default)]
pub struct PoolState {
    notified_txs_available: bool,
    pre_check: Option<PreCheckFunc>,
    post_check: Option<PostCheckFunc>,
    height: i64,
    last_purge_time: Option<SystemTime>,
}

/// The CAT transaction pool.
pub struct TxPool {
    config: MempoolConfig,
    app: Arc<dyn AppConnMempool>,
    state: Mutex<PoolState>,
    rejected_tx_cache: RejectedTxCache,
    evicted_tx_cache: LruTxCache,
    /// Peers known to have txs; future-sequence SeenTx queue.
    pub(crate) seen_tracker: SeenTracker,
    pub(crate) store: Store,
    broadcast_queue: Mutex<VecDeque<TxKey>>,
    broadcast_notify: Notify,
    height_signal: Arc<Notify>,
    txs_available: OnceLock<Arc<Notify>>,
}

impl std::fmt::Debug for TxPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TxPool")
            .field("size", &self.size())
            .field("bytes", &self.size_bytes())
            .finish_non_exhaustive()
    }
}

/// `defer store.release(key)`.
struct Release<'a>(&'a Store, TxKey);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.release(&self.1);
    }
}

fn tx_key_string_hex(key: &TxKey) -> String {
    // Go formats `txKey.String()` ("TxKey{<HEX>}") with %X.
    hex::encode_upper(format!("TxKey{{{}}}", hex::encode_upper(key)))
}

impl TxPool {
    /// `NewTxPool` at `height` (the last committed height).
    #[must_use]
    pub fn new(config: MempoolConfig, app: Arc<dyn AppConnMempool>, height: i64) -> Self {
        Self {
            rejected_tx_cache: RejectedTxCache::new(config.cache_size),
            evicted_tx_cache: LruTxCache::new(config.cache_size / 5),
            config,
            app,
            state: Mutex::new(PoolState {
                height,
                ..PoolState::default()
            }),
            seen_tracker: SeenTracker::new(),
            store: Store::new(),
            broadcast_queue: Mutex::new(VecDeque::new()),
            broadcast_notify: Notify::new(),
            height_signal: Arc::new(Notify::new()),
            txs_available: OnceLock::new(),
        }
    }

    /// `WithPreCheck`.
    #[must_use]
    pub fn with_pre_check(self, f: PreCheckFunc) -> Self {
        self.lock_state().pre_check = Some(f);
        self
    }

    /// `WithPostCheck`.
    #[must_use]
    pub fn with_post_check(self, f: PostCheckFunc) -> Self {
        self.lock_state().post_check = Some(f);
        self
    }

    /// Replaces the seen tracker (tests: limits, mock clock).
    #[must_use]
    pub fn with_seen_tracker(mut self, t: SeenTracker) -> Self {
        self.seen_tracker = t;
        self
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &MempoolConfig {
        &self.config
    }

    /// The application connection.
    #[must_use]
    pub fn app(&self) -> &Arc<dyn AppConnMempool> {
        &self.app
    }

    fn lock_state(&self) -> MutexGuard<'_, PoolState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Lock`: the returned guard is `Unlock` on drop.
    pub fn lock(&self) -> TxPoolGuard<'_> {
        TxPoolGuard {
            pool: self,
            state: self.lock_state(),
        }
    }

    /// `Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.store.size()
    }

    /// `SizeBytes`.
    #[must_use]
    pub fn size_bytes(&self) -> i64 {
        self.store.total_bytes()
    }

    /// `Height`: the last height passed to `Update`.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.lock_state().height
    }

    /// `HeightSignal`: notified whenever the height advances.
    #[must_use]
    pub fn height_signal(&self) -> Arc<Notify> {
        self.height_signal.clone()
    }

    /// `Has`.
    #[must_use]
    pub fn has(&self, key: &TxKey) -> bool {
        self.store.has(key)
    }

    /// `GetTxByKey`.
    #[must_use]
    pub fn get_tx_by_key(&self, key: &TxKey) -> Option<CachedTx> {
        self.store.get(key).map(|w| w.tx.clone())
    }

    /// The wrapped tx (priority, signer, sequence, ...).
    #[must_use]
    pub fn get_wrapped(&self, key: &TxKey) -> Option<Arc<WrappedTx>> {
        self.store.get(key)
    }

    /// All txs in reap order.
    #[must_use]
    pub fn ordered_txs(&self) -> Vec<Arc<WrappedTx>> {
        self.store.get_ordered_txs()
    }

    /// `WasRecentlyEvicted`.
    #[must_use]
    pub fn was_recently_evicted(&self, key: &TxKey) -> bool {
        self.evicted_tx_cache.has(key)
    }

    /// `WasRecentlyRejected`: `(rejected, code, log)`.
    #[must_use]
    pub fn was_recently_rejected(&self, key: &TxKey) -> (bool, u32, String) {
        match self.rejected_tx_cache.get(key) {
            Some((code, log)) => (true, code, log),
            None => (false, 0, String::new()),
        }
    }

    /// The rejected-tx cache.
    #[must_use]
    pub fn rejected_tx_cache(&self) -> &RejectedTxCache {
        &self.rejected_tx_cache
    }

    /// The seen tracker.
    #[must_use]
    pub fn seen_tracker(&self) -> &SeenTracker {
        &self.seen_tracker
    }

    /// `CheckTx`: the entry point for txs from RPC. `Ok` carries the CheckTx
    /// response (what Go passes to the callback).
    pub fn check_tx(&self, tx: &[u8], tx_info: TxInfo) -> Result<ResponseCheckTx, MempoolError> {
        if tx.len() > self.config.max_tx_bytes {
            return Err(MempoolError::TxTooLarge {
                max: self.config.max_tx_bytes,
                actual: tx.len(),
            });
        }
        let cached = CachedTx::new(tx.to_vec());
        let key = cached.key();
        let rsp = self.try_add_new_tx(cached, key, &tx_info)?;
        self.mark_to_be_broadcast(key);
        Ok(rsp)
    }

    /// `next`: waits for the next locally submitted tx still in the pool.
    pub async fn next_to_broadcast(&self) -> Arc<WrappedTx> {
        loop {
            loop {
                let key = {
                    let mut q = self
                        .broadcast_queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    q.pop_front()
                };
                let Some(key) = key else { break };
                if let Some(w) = self.store.get(&key) {
                    return w;
                }
            }
            self.broadcast_notify.notified().await;
        }
    }

    /// `markToBeBroadcast`.
    fn mark_to_be_broadcast(&self, key: TxKey) {
        if !self.config.broadcast || !self.store.has(&key) {
            return;
        }
        self.broadcast_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(key);
        self.broadcast_notify.notify_one();
    }

    /// `TryAddNewTx`: checks a tx not seen before with the app and adds it
    /// if valid and there is room. `Ok` with a non-zero code means the app
    /// rejected it.
    pub fn try_add_new_tx(
        &self,
        tx: CachedTx,
        key: TxKey,
        tx_info: &TxInfo,
    ) -> Result<ResponseCheckTx, MempoolError> {
        if self.has(&key) {
            return Err(MempoolError::TxInMempool);
        }
        if !self.store.reserve(key) {
            tracing::trace!(tx = %hex::encode_upper(key), "mempool already attempting to verify and add transaction");
            self.peer_has_tx(tx_info.sender_id, key);
            return Err(MempoolError::TxInMempool);
        }
        let _release = Release(&self.store, key);

        let pre = self.lock_state().pre_check.clone();
        if let Some(pre) = pre
            && let Err(e) = pre(&tx)
        {
            self.rejected_tx_cache.push(key, 0, &e);
            return Err(MempoolError::PreCheck(e));
        }

        self.app.error().map_err(MempoolError::App)?;

        let mut st = self.lock_state();
        let rsp = self
            .app
            .check_tx(RequestCheckTx {
                tx: tx.tx().to_vec(),
                r#type: CheckTxType::New as i32,
            })
            .map_err(MempoolError::App)?;
        if rsp.code != 0 {
            self.rejected_tx_cache.push(key, rsp.code, &rsp.log);
            return Ok(rsp);
        }

        let wtx = WrappedTx::new(
            tx,
            st.height,
            rsp.gas_wanted,
            rsp.priority,
            rsp.address.clone(),
            rsp.sequence,
            tx_info.from_broadcast,
        );
        if let Some(post) = st.post_check.clone()
            && let Err(e) = post(&wtx.tx, &rsp)
        {
            self.rejected_tx_cache.push(key, 0, &e);
            return Err(MempoolError::PostCheck(e));
        }
        self.add_new_transaction(&mut st, Arc::new(wtx))?;
        Ok(rsp)
    }

    /// `RemoveTxByKey`: also caches the key as rejected.
    pub fn remove_tx_by_key(&self, key: &TxKey) {
        self.rejected_tx_cache.push(*key, 0, "");
        self.store.remove(key);
        self.seen_tracker.remove_key(key);
    }

    /// `Flush`.
    pub fn flush(&self) {
        self.store.reset();
        self.seen_tracker.reset();
        self.rejected_tx_cache.reset();
        self.evicted_tx_cache.reset();
        self.broadcast_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// `PeerHasTx`.
    pub fn peer_has_tx(&self, peer: u16, key: TxKey) {
        self.seen_tracker.add_peer(key, peer);
    }

    /// `ReapMaxBytesMaxGas`: whole signer sets, highest aggregated priority
    /// first; the first set that does not fit is broken down by
    /// `determineLeftoverTxs`. Reaping does not remove txs.
    #[must_use]
    pub fn reap_max_bytes_max_gas(&self, max_bytes: i64, max_gas: i64) -> Vec<CachedTx> {
        self.store.process_ordered_tx_sets(|sets| {
            let (mut total_gas, mut total_bytes) = (0i64, 0i64);
            let mut keep = Vec::new();
            for (idx, set) in sets.iter().enumerate() {
                if (max_bytes >= 0 && total_bytes.wrapping_add(set.bytes) > max_bytes)
                    || (max_gas >= 0 && total_gas.wrapping_add(set.total_gas_wanted) > max_gas)
                {
                    keep.extend(determine_leftover_txs(
                        &sets[idx..],
                        max_bytes.wrapping_sub(total_bytes),
                        max_gas.wrapping_sub(total_gas),
                    ));
                    break;
                }
                total_bytes = total_bytes.wrapping_add(set.bytes);
                total_gas = total_gas.wrapping_add(set.total_gas_wanted);
                keep.extend(set.raw_txs());
            }
            keep
        })
    }

    /// `ReapMaxTxs` (negative: all).
    #[must_use]
    pub fn reap_max_txs(&self, max: i64) -> Vec<CachedTx> {
        self.store.process_ordered_tx_sets(|sets| {
            let mut keep = Vec::new();
            for set in sets {
                for tx in &set.txs {
                    if max >= 0 && i64::try_from(keep.len()).unwrap_or(i64::MAX) >= max {
                        return keep;
                    }
                    keep.push(tx.tx.clone());
                }
            }
            keep
        })
    }

    /// `addNewTransaction`: makes room by evicting lower-priority sets if
    /// needed, then stores the tx.
    fn add_new_transaction(
        &self,
        st: &mut PoolState,
        wtx: Arc<WrappedTx>,
    ) -> Result<(), MempoolError> {
        if !self.can_add_tx(wtx.size()) {
            let new_agg = self.store.aggregated_priority_after_add(&wtx);
            let (mut victims, victim_bytes) = self.store.get_tx_sets_below_priority(new_agg);
            if victims.is_empty() || victim_bytes < wtx.size() {
                let key = wtx.key();
                self.evicted_tx_cache.push(key);
                return Err(MempoolError::MempoolFull(format!(
                    "rejected valid incoming transaction; mempool is full ({}). Size: ({}:{})",
                    tx_key_string_hex(&key),
                    self.size(),
                    self.size_bytes()
                )));
            }
            tracing::debug!(
                new_tx = %hex::encode_upper(wtx.key()),
                new_set_priority = new_agg,
                "evicting lower-priority tx sets"
            );
            // Lowest aggregated priority first; ties: newer sets first.
            victims.sort_by(|a, b| {
                a.aggregated_priority
                    .cmp(&b.aggregated_priority)
                    .then_with(|| b.first_timestamp.cmp(&a.first_timestamp))
            });
            let mut available = self.available_bytes();
            for set in &victims {
                // Highest sequences first.
                for tx in set.txs.iter().rev() {
                    if available >= wtx.size() {
                        break;
                    }
                    self.evict_tx(tx);
                    available = available.wrapping_add(tx.size());
                }
                if available >= wtx.size() {
                    break;
                }
            }
        }
        self.store.set(wtx);
        self.notify_txs_available(st);
        Ok(())
    }

    fn evict_tx(&self, wtx: &WrappedTx) {
        let key = wtx.key();
        self.store.remove(&key);
        self.evicted_tx_cache.push(key);
        tracing::debug!(old_tx = %hex::encode_upper(key), old_priority = wtx.priority, "evicted valid existing transaction; mempool full");
    }

    /// `handleRecheckResult`.
    fn handle_recheck_result(
        &self,
        post: Option<&PostCheckFunc>,
        wtx: &WrappedTx,
        rsp: &ResponseCheckTx,
    ) {
        let err = post.and_then(|f| f(&wtx.tx, rsp).err());
        if rsp.code == 0 && err.is_none() {
            return;
        }
        tracing::debug!(
            priority = wtx.priority,
            tx = %hex::encode_upper(wtx.key()),
            err = ?err,
            code = rsp.code,
            "existing transaction no longer valid; failed re-CheckTx callback"
        );
        let key = wtx.key();
        self.store.remove(&key);
        self.rejected_tx_cache.push(key, rsp.code, &rsp.log);
    }

    /// `recheckTransactions`.
    fn recheck_transactions(&self, st: &mut PoolState) {
        let wtxs = self.store.get_ordered_txs();
        for wtx in wtxs {
            match self.app.check_tx(RequestCheckTx {
                tx: wtx.tx.tx().to_vec(),
                r#type: CheckTxType::Recheck as i32,
            }) {
                Err(e) => {
                    tracing::error!(err = %e, key = %hex::encode(wtx.key()), "failed to execute CheckTx during recheck");
                }
                Ok(rsp) => self.handle_recheck_result(st.post_check.as_ref(), &wtx, &rsp),
            }
        }
        let _ = self.app.flush();
        self.notify_txs_available(st);
    }

    fn available_bytes(&self) -> i64 {
        self.config.max_txs_bytes.wrapping_sub(self.size_bytes())
    }

    /// `canAddTx` (Go allows `Size + 1` txs: it checks `numTxs > Size`).
    fn can_add_tx(&self, size: i64) -> bool {
        let num = self.size();
        let bytes = self.size_bytes();
        !(num > self.config.size || size.wrapping_add(bytes) > self.config.max_txs_bytes)
    }

    /// `purgeExpiredTxs`.
    fn purge_expired_txs(&self, block_height: i64) {
        let ttl_blocks = self.config.ttl_num_blocks;
        let ttl = self.config.ttl_duration;
        if ttl_blocks == 0 && ttl.is_zero() {
            return;
        }
        let expiration_height = if ttl_blocks == 0 {
            0
        } else {
            block_height.wrapping_sub(ttl_blocks)
        };
        let expiration_time = (!ttl.is_zero()).then(|| {
            SystemTime::now()
                .checked_sub(ttl)
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });
        let (purged, _) = self
            .store
            .purge_expired_txs(expiration_height, expiration_time);
        for tx in purged {
            self.evicted_tx_cache.push(tx.key());
        }
        self.seen_tracker.prune_expired();
    }

    fn notify_txs_available(&self, st: &mut PoolState) {
        if self.size() == 0 {
            return;
        }
        if let Some(n) = self.txs_available.get()
            && !st.notified_txs_available
        {
            st.notified_txs_available = true;
            n.notify_one();
        }
    }

    /// `EnableTxsAvailable`.
    pub fn enable_txs_available(&self) {
        let _ = self.txs_available.set(Arc::new(Notify::new()));
    }

    /// `TxsAvailable`.
    #[must_use]
    pub fn txs_available(&self) -> Option<Arc<Notify>> {
        self.txs_available.get().cloned()
    }
}

/// `determineLeftoverTxs`: tries every starting offset into the remaining
/// sets and keeps the subset with the highest aggregated priority (first
/// wins on ties).
fn determine_leftover_txs(sets: &[&TxSet], rem_bytes: i64, rem_gas: i64) -> Vec<CachedTx> {
    let mut best: Option<(i64, Vec<CachedTx>)> = None;
    for i in 0..sets.len() {
        let (prio, txs) = aggregated_priority_and_txs(&sets[i..], rem_bytes, rem_gas);
        if best.as_ref().is_none_or(|(p, _)| prio > *p) {
            best = Some((prio, txs));
        }
    }
    best.map(|(_, t)| t).unwrap_or_default()
}

/// `getAggregatedPriorityAndTxs`.
fn aggregated_priority_and_txs(
    sets: &[&TxSet],
    mut rem_bytes: i64,
    mut rem_gas: i64,
) -> (i64, Vec<CachedTx>) {
    let mut subsets: Vec<TxSet> = Vec::with_capacity(sets.len());
    for set in sets {
        let sliced = set.slice_by_bytes_and_gas(rem_bytes, rem_gas);
        let full = sliced.txs.len() == set.txs.len();
        subsets.push(sliced);
        if !full {
            break;
        }
        rem_bytes = rem_bytes.wrapping_sub(set.bytes);
        rem_gas = rem_gas.wrapping_sub(set.total_gas_wanted);
    }
    let prio = aggregate_priority_across_sets(&subsets);
    let txs = subsets.iter().flat_map(TxSet::raw_txs).collect();
    (prio, txs)
}

/// The locked pool (`Lock` ... `Unlock`).
pub struct TxPoolGuard<'a> {
    pool: &'a TxPool,
    state: MutexGuard<'a, PoolState>,
}

impl TxPoolGuard<'_> {
    /// `FlushAppConn`.
    pub fn flush_app_conn(&mut self) -> Result<(), MempoolError> {
        self.pool.app.flush().map_err(MempoolError::App)
    }

    /// `Update`: see [`MempoolLock::update`].
    ///
    /// # Panics
    /// If `block_txs` and `results` differ in length.
    pub fn update(
        &mut self,
        height: i64,
        block_txs: &[CachedTx],
        results: &[ExecTxResult],
        pre_check: Option<PreCheckFunc>,
        post_check: Option<PostCheckFunc>,
    ) -> Result<(), MempoolError> {
        assert_eq!(
            block_txs.len(),
            results.len(),
            "mempool: got {} transactions but {} DeliverTx responses",
            block_txs.len(),
            results.len()
        );
        tracing::debug!(height, txs = block_txs.len(), "updating mempool");
        let pool = self.pool;
        let st = &mut *self.state;
        st.height = height;
        st.notified_txs_available = false;
        if pre_check.is_some() {
            st.pre_check = pre_check;
        }
        if post_check.is_some() {
            st.post_check = post_check;
        }
        st.last_purge_time = Some(SystemTime::now());

        for tx in block_txs {
            // Regardless of the result, the tx leaves the mempool.
            pool.remove_tx_by_key(&tx.key());
        }

        // TTL purging only happens here, to keep the check state consistent.
        pool.purge_expired_txs(height);

        if pool.size() > 0 {
            if pool.config.recheck {
                pool.recheck_transactions(st);
            } else {
                pool.notify_txs_available(st);
            }
        }
        pool.height_signal.notify_one();
        Ok(())
    }

    /// The height (under the lock).
    #[must_use]
    pub fn height(&self) -> i64 {
        self.state.height
    }
}

impl MempoolLock for TxPoolGuard<'_> {
    fn flush_app_conn(&mut self) -> Result<(), MempoolError> {
        TxPoolGuard::flush_app_conn(self)
    }

    fn update(
        &mut self,
        height: i64,
        block_txs: &[CachedTx],
        results: &[ExecTxResult],
        pre_check: Option<PreCheckFunc>,
        post_check: Option<PostCheckFunc>,
    ) -> Result<(), MempoolError> {
        TxPoolGuard::update(self, height, block_txs, results, pre_check, post_check)
    }
}

impl Mempool for TxPool {
    fn check_tx(&self, tx: &[u8], tx_info: TxInfo) -> Result<ResponseCheckTx, MempoolError> {
        TxPool::check_tx(self, tx, tx_info)
    }

    fn remove_tx_by_key(&self, key: &TxKey) -> Result<(), MempoolError> {
        TxPool::remove_tx_by_key(self, key);
        Ok(())
    }

    fn reap_max_bytes_max_gas(&self, max_bytes: i64, max_gas: i64) -> Vec<CachedTx> {
        TxPool::reap_max_bytes_max_gas(self, max_bytes, max_gas)
    }

    fn reap_max_txs(&self, max: i64) -> Vec<CachedTx> {
        TxPool::reap_max_txs(self, max)
    }

    fn lock(&self) -> Box<dyn MempoolLock + '_> {
        Box::new(TxPool::lock(self))
    }

    fn flush(&self) {
        TxPool::flush(self);
    }

    fn enable_txs_available(&self) {
        TxPool::enable_txs_available(self);
    }

    fn txs_available(&self) -> Option<Arc<Notify>> {
        TxPool::txs_available(self)
    }

    fn size(&self) -> usize {
        TxPool::size(self)
    }

    fn size_bytes(&self) -> i64 {
        TxPool::size_bytes(self)
    }

    fn get_tx_by_key(&self, key: &TxKey) -> Option<CachedTx> {
        TxPool::get_tx_by_key(self, key)
    }

    fn was_recently_evicted(&self, key: &TxKey) -> bool {
        TxPool::was_recently_evicted(self, key)
    }

    fn was_recently_rejected(&self, key: &TxKey) -> (bool, u32, String) {
        TxPool::was_recently_rejected(self, key)
    }
}
