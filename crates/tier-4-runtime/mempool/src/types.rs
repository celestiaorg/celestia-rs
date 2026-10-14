//! Shared mempool types (`mempool/mempool.go`, `mempool/tx.go`,
//! `types.CachedTx`) and the API consensus uses.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use celestia_core_proto::abci::{ExecTxResult, ResponseCheckTx};
use tokio::sync::Notify;

use crate::errors::MempoolError;

/// `types.TxKey`: SHA-256 of the tx, or of the inner tx of a `BlobTx` /
/// `IndexWrapper`.
pub type TxKey = [u8; 32];

/// `types.Tx.Key`: a `BlobTx` (checked first) or `IndexWrapper` keys as its
/// inner sdk tx, so a PFB in the mempool (a `BlobTx`) and the same PFB in a
/// block (an `IndexWrapper`) share a key.
#[must_use]
pub fn tx_key(tx: &[u8]) -> TxKey {
    if let Some(btx) = ctypes::tx::unmarshal_blob_tx(tx) {
        return hash::sha256(&btx.tx);
    }
    if let Some(iw) = ctypes::tx::unmarshal_index_wrapper(tx) {
        return hash::sha256(&iw.tx);
    }
    hash::sha256(tx)
}

/// `types.CachedTx`: raw tx bytes with their key. Cloning is cheap.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CachedTx {
    tx: Arc<[u8]>,
    key: TxKey,
}

impl CachedTx {
    /// Wraps raw tx bytes (`Tx.ToCachedTx`), computing the key.
    #[must_use]
    pub fn new(tx: impl Into<Arc<[u8]>>) -> Self {
        let tx = tx.into();
        let key = tx_key(&tx);
        Self { tx, key }
    }

    /// `NewCachedTx(tx, hash)`: wraps tx bytes with a known key.
    #[must_use]
    pub fn with_key(tx: impl Into<Arc<[u8]>>, key: TxKey) -> Self {
        Self { tx: tx.into(), key }
    }

    /// The raw tx.
    #[must_use]
    pub fn tx(&self) -> &[u8] {
        &self.tx
    }

    /// The shared raw tx.
    #[must_use]
    pub fn bytes(&self) -> Arc<[u8]> {
        self.tx.clone()
    }

    /// `CachedTx.Key`.
    #[must_use]
    pub fn key(&self) -> TxKey {
        self.key
    }

    /// Raw size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tx.len()
    }

    /// Whether the tx is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tx.is_empty()
    }
}

impl fmt::Debug for CachedTx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CachedTx{{key: {}, len: {}}}",
            hex::encode_upper(self.key),
            self.tx.len()
        )
    }
}

/// `mempool.TxInfo`: where a tx came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TxInfo {
    /// Short mempool ID of the sending peer (0: RPC).
    pub sender_id: u16,
    /// P2P ID of the sending peer (for logs).
    pub sender_p2p_id: String,
    /// Submitted through an RPC broadcast endpoint.
    pub from_broadcast: bool,
}

/// `PreCheckFunc`: a filter run before CheckTx.
pub type PreCheckFunc = Arc<dyn Fn(&CachedTx) -> Result<(), String> + Send + Sync>;
/// `PostCheckFunc`: a filter run after CheckTx.
pub type PostCheckFunc =
    Arc<dyn Fn(&CachedTx, &ResponseCheckTx) -> Result<(), String> + Send + Sync>;

/// `types.ComputeProtoSizeForTxs([]Tx{tx})`.
#[must_use]
pub fn compute_proto_size_for_tx(tx: &[u8]) -> i64 {
    let n = 1 + prost::encoding::encoded_len_varint(tx.len() as u64) + tx.len();
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `PreCheckMaxBytes`: rejects txs whose proto size exceeds `max_bytes`.
#[must_use]
pub fn pre_check_max_bytes(max_bytes: i64) -> PreCheckFunc {
    Arc::new(move |tx: &CachedTx| {
        let tx_size = compute_proto_size_for_tx(tx.tx());
        if tx_size > max_bytes {
            return Err(format!("tx size is too big: {tx_size}, max: {max_bytes}"));
        }
        Ok(())
    })
}

/// `PostCheckMaxGas`: rejects negative gas and (unless `max_gas == -1`) gas
/// above `max_gas`.
#[must_use]
pub fn post_check_max_gas(max_gas: i64) -> PostCheckFunc {
    Arc::new(move |_tx: &CachedTx, res: &ResponseCheckTx| {
        if res.gas_wanted < 0 {
            return Err(format!("gas wanted {} is negative", res.gas_wanted));
        }
        if max_gas == -1 {
            return Ok(());
        }
        if res.gas_wanted > max_gas {
            return Err(format!(
                "gas wanted {} is greater than max gas {max_gas}",
                res.gas_wanted
            ));
        }
        Ok(())
    })
}

/// The `[mempool]` section of `config.toml` used by the CAT mempool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolConfig {
    /// Recheck remaining txs after every block.
    pub recheck: bool,
    /// Gossip txs to peers (`!ListenOnly`).
    pub broadcast: bool,
    /// Maximum number of txs.
    pub size: usize,
    /// Maximum total bytes of txs.
    pub max_txs_bytes: i64,
    /// Size of the rejected-tx cache (the evicted cache is a fifth of it).
    pub cache_size: usize,
    /// Maximum size of one tx.
    pub max_tx_bytes: usize,
    /// Time after which a tx is purged (zero: off).
    pub ttl_duration: Duration,
    /// Blocks after which a tx is purged (0: off).
    pub ttl_num_blocks: i64,
    /// Time to wait for a requested tx before asking another peer (zero:
    /// the reactor default, 60s).
    pub max_gossip_delay: Duration,
    /// Persistent peers guaranteed to receive SeenTx per signer.
    pub max_persistent_sticky_peers: i32,
}

impl Default for MempoolConfig {
    /// `DefaultMempoolConfig`.
    fn default() -> Self {
        Self {
            recheck: true,
            broadcast: true,
            size: 5000,
            max_txs_bytes: 1024 * 1024 * 1024,
            cache_size: 10000,
            max_tx_bytes: 1024 * 1024,
            ttl_duration: Duration::ZERO,
            ttl_num_blocks: 0,
            max_gossip_delay: Duration::ZERO,
            max_persistent_sticky_peers: 4,
        }
    }
}

impl MempoolConfig {
    /// `TestMempoolConfig`: the default with a 1000-entry cache.
    #[must_use]
    pub fn test() -> Self {
        Self {
            cache_size: 1000,
            ..Self::default()
        }
    }

    /// celestia-app v10's overrides (`app.DefaultConsensusConfig`):
    /// 36-block TTL, 8 MiB max tx, 400 MiB pool, 20s gossip delay.
    #[must_use]
    pub fn celestia_app() -> Self {
        Self {
            ttl_num_blocks: 36,
            ttl_duration: Duration::ZERO,
            max_tx_bytes: 8 * 1024 * 1024,
            max_txs_bytes: 400 * 1024 * 1024,
            max_gossip_delay: Duration::from_secs(20),
            ..Self::default()
        }
    }
}

/// The mempool API consensus and RPC use (`mempool.Mempool`).
///
/// Updates must be synchronized with block commits: consensus takes
/// [`Mempool::lock`], flushes the app connection, commits the app, then
/// calls [`MempoolLock::update`] before releasing the lock (Go's
/// `BlockExecutor.Commit`), passing celestia-core's `TxPreCheck(state)`
/// (`pre_check_max_bytes(max_data_bytes_no_evidence(Block.MaxBytes,
/// validators))`) and `TxPostCheck(state)` (`post_check_max_gas(Block.MaxGas)`).
/// Block txs are keyed blob-aware, so a block's `IndexWrapper` removes the
/// pool's `BlobTx`.
pub trait Mempool: Send + Sync {
    /// `CheckTx`: runs the tx through the application and adds it if
    /// valid. `Ok` carries the CheckTx response (which may have a non-zero
    /// code); `Err` is a mempool-level rejection.
    fn check_tx(&self, tx: &[u8], tx_info: TxInfo) -> Result<ResponseCheckTx, MempoolError>;

    /// `RemoveTxByKey`.
    fn remove_tx_by_key(&self, key: &TxKey) -> Result<(), MempoolError>;

    /// `ReapMaxBytesMaxGas`: highest-priority txs fitting both limits
    /// (negative: unlimited), grouped by signer in sequence order. Sizes are
    /// raw tx bytes (a PFB counts as its whole `BlobTx`). celestia-core's
    /// `CreateProposalBlock` reaps `maxDataBytes + maxDataBytes / 4` (-1
    /// when `Block.MaxBytes` is -1) and `Block.MaxGas`, leaving the final
    /// selection to PrepareProposal.
    fn reap_max_bytes_max_gas(&self, max_bytes: i64, max_gas: i64) -> Vec<CachedTx>;

    /// `ReapMaxTxs` (negative: all).
    fn reap_max_txs(&self, max: i64) -> Vec<CachedTx>;

    /// `Lock`: blocks new txs until the returned guard is dropped
    /// (`Unlock`).
    fn lock(&self) -> Box<dyn MempoolLock + '_>;

    /// `Flush`: removes every tx and clears the caches.
    fn flush(&self);

    /// `EnableTxsAvailable`.
    fn enable_txs_available(&self);

    /// `TxsAvailable`: notified at most once per height, when txs are
    /// available (`None` unless enabled).
    fn txs_available(&self) -> Option<Arc<Notify>>;

    /// `Size`.
    fn size(&self) -> usize;

    /// `SizeBytes`.
    fn size_bytes(&self) -> i64;

    /// `GetTxByKey`.
    fn get_tx_by_key(&self, key: &TxKey) -> Option<CachedTx>;

    /// `WasRecentlyEvicted`.
    fn was_recently_evicted(&self, key: &TxKey) -> bool;

    /// `WasRecentlyRejected`: `(rejected, code, log)`.
    fn was_recently_rejected(&self, key: &TxKey) -> (bool, u32, String);
}

/// The locked mempool (held between `Lock` and `Unlock`).
pub trait MempoolLock {
    /// `FlushAppConn`.
    fn flush_app_conn(&mut self) -> Result<(), MempoolError>;

    /// `Update`: removes the committed `block_txs`, advances the height,
    /// purges expired txs and rechecks the rest. `results` must have one
    /// entry per tx. `None` keeps the current pre/post-check filter.
    ///
    /// # Panics
    /// If `block_txs` and `results` differ in length (as Go does).
    fn update(
        &mut self,
        height: i64,
        block_txs: &[CachedTx],
        results: &[ExecTxResult],
        pre_check: Option<PreCheckFunc>,
        post_check: Option<PostCheckFunc>,
    ) -> Result<(), MempoolError>;
}
