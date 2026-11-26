//! The server's lifecycle (Go `server.go`, `server_prune.go`).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use celestia_proto::celestia::fibre::v1::{
    DownloadShardRequest, DownloadShardResponse, UploadShardRequest, UploadShardResponse,
};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tonic::Status;

use crate::assignment::assigned_rows;
use crate::codec::CodecLimits;
use crate::config::{ConfigError, HealthSettings, ServerConfig};
use crate::health::{HealthDeps, HealthManager};
use crate::metrics::ServerMetrics;
use crate::occupancy::{Occupancy, UploadCoordinator};
use crate::params::{BlobConfig, DEFAULT_PROTOCOL_PARAMS};
use crate::proto::ServingStatus;
use crate::service::CallContext;
use crate::state::{ChainState, Signer};
use crate::store::{self, Store};
use crate::tlsid::{self, RawBytesSigner};
use crate::transport::{self, TransportOptions};
use crate::verify::ShardVerifier;

/// A failure of [`Server::new`] or [`Server::start`]; the texts are Go's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// Invalid configuration.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Any other startup failure.
    #[error("{0}")]
    Other(String),
}

/// State shared by the handlers, the transport and the background tasks.
pub(crate) struct Inner {
    pub(crate) cfg: ServerConfig,
    pub(crate) state: Arc<dyn ChainState>,
    pub(crate) signer: Arc<dyn Signer>,
    pub(crate) metrics: Arc<ServerMetrics>,
    pub(crate) occupancy: Arc<Occupancy>,
    pub(crate) uploads: UploadCoordinator,
    pub(crate) verifier: ShardVerifier,
    /// Go's verifier pool: caps concurrent verifications.
    pub(crate) verify_slots: Arc<Semaphore>,
    pub(crate) health: HealthManager,
    pub(crate) codec_limits: CodecLimits,
    min_upload_size: AtomicI64,
    chain_id: OnceLock<String>,
    store: OnceLock<Arc<Store>>,
}

impl Inner {
    pub(crate) fn chain_id(&self) -> String {
        self.chain_id
            .get()
            .cloned()
            .unwrap_or_else(|| self.state.chain_id())
    }

    pub(crate) fn min_upload_size(&self) -> i64 {
        self.min_upload_size.load(Ordering::Relaxed)
    }

    pub(crate) fn store(&self) -> Result<Arc<Store>, Status> {
        self.store
            .get()
            .cloned()
            .ok_or_else(|| Status::unavailable("fibre server is not started"))
    }
}

/// Handles of a started server.
struct Running {
    /// Stops accepting and drains connections gracefully.
    shutdown: CancellationToken,
    /// Drops every connection at once.
    force: CancellationToken,
    /// Stops the prune and health loops.
    background: CancellationToken,
    connections: TaskTracker,
    serve: JoinHandle<()>,
    prune: JoinHandle<()>,
    health: JoinHandle<()>,
}

#[derive(Default)]
struct Lifecycle {
    listener: Option<std::net::TcpListener>,
    running: Option<Running>,
    started: bool,
    stopped: bool,
}

/// The Fibre server (Go `fibre.Server`).
///
/// [`Server::new`] binds the listener, so [`Server::listen_address`] is
/// known before [`Server::start`]; `start` follows Go's order (signer,
/// TLS certificate, store, occupancy, budget, prune loop, serving, health
/// loop) and spawns its tasks on the current Tokio runtime.
/// [`Server::stop`] publishes NOT_SERVING, drains in-flight requests and
/// closes the store. [`Server::done`] fires when serving ended, by stop or
/// because the listener failed ([`Server::serve_error`]).
pub struct Server {
    inner: Arc<Inner>,
    addr: SocketAddr,
    lifecycle: Mutex<Lifecycle>,
    done: CancellationToken,
    serve_error: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// Go `deriveBudget`: `full_stake * assigned_rows / original_rows` in 128
/// bits; non-positive inputs yield 0 (no limit).
#[must_use]
pub fn derive_budget(full_stake: i64, assigned_rows: i64, original_rows: i64) -> i64 {
    if full_stake <= 0 || assigned_rows <= 0 || original_rows <= 0 {
        return 0;
    }
    let q = i128::from(full_stake) * i128::from(assigned_rows) / i128::from(original_rows);
    i64::try_from(q).unwrap_or(i64::MAX)
}

/// The validator's signer as tlsid's synchronous signer, for the one
/// certificate signature at start (runs on a blocking thread).
struct BlockingSigner {
    signer: Arc<dyn Signer>,
    handle: tokio::runtime::Handle,
}

impl RawBytesSigner for BlockingSigner {
    fn sign_raw_bytes(
        &self,
        chain_id: &str,
        unique_id: &str,
        raw: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.handle
            .block_on(self.signer.sign_raw_bytes(chain_id, unique_id, raw))
            .map_err(|e| e.0)
    }
}

impl Server {
    /// Go `NewServer`: validates `cfg` and binds the listener.
    pub fn new(
        cfg: ServerConfig,
        state: Arc<dyn ChainState>,
        signer: Arc<dyn Signer>,
    ) -> Result<Self, StartError> {
        let health_settings: HealthSettings = cfg.validate()?;
        let listener = std::net::TcpListener::bind(&cfg.server_listen_address)
            .and_then(|l| l.set_nonblocking(true).map(|()| l))
            .map_err(|e| {
                StartError::Other(format!(
                    "opening gRPC listener: listen on {}: {e}",
                    cfg.server_listen_address
                ))
            })?;
        let addr = listener
            .local_addr()
            .map_err(|e| StartError::Other(format!("opening gRPC listener: {e}")))?;
        let occupancy = Arc::new(Occupancy::new(0));
        let metrics = Arc::new(ServerMetrics::new(Arc::clone(&occupancy)));
        let workers = usize::try_from(cfg.upload_verify_workers).unwrap_or(1);
        let inner = Inner {
            state,
            signer,
            occupancy,
            uploads: UploadCoordinator::new(),
            verifier: ShardVerifier::new(&BlobConfig::default_v0()),
            verify_slots: Arc::new(Semaphore::new(workers)),
            health: HealthManager::new(health_settings, Arc::clone(&metrics)),
            metrics,
            codec_limits: CodecLimits {
                max_shard_rows: DEFAULT_PROTOCOL_PARAMS.max_rows_per_validator(),
                max_proof_segments: DEFAULT_PROTOCOL_PARAMS.merkle_proof_depth(),
            },
            min_upload_size: AtomicI64::new(cfg.min_upload_size),
            chain_id: OnceLock::new(),
            store: OnceLock::new(),
            cfg,
        };
        Ok(Self {
            inner: Arc::new(inner),
            addr,
            lifecycle: Mutex::new(Lifecycle {
                listener: Some(listener),
                ..Lifecycle::default()
            }),
            done: CancellationToken::new(),
            serve_error: Arc::new(Mutex::new(None)),
        })
    }

    fn lifecycle(&self) -> std::sync::MutexGuard<'_, Lifecycle> {
        self.lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The address the server listens on (Go `ListenAddress`).
    #[must_use]
    pub fn listen_address(&self) -> SocketAddr {
        self.addr
    }

    /// The chain ID (Go `ChainID`).
    #[must_use]
    pub fn chain_id(&self) -> String {
        self.inner.chain_id()
    }

    /// The configuration the server runs with.
    #[must_use]
    pub fn config(&self) -> &ServerConfig {
        &self.inner.cfg
    }

    /// The server's metrics.
    #[must_use]
    pub fn metrics(&self) -> Arc<ServerMetrics> {
        Arc::clone(&self.inner.metrics)
    }

    /// The storage-budget counter.
    #[must_use]
    pub fn occupancy(&self) -> Arc<Occupancy> {
        Arc::clone(&self.inner.occupancy)
    }

    /// The store, once started (Go `Store`).
    #[must_use]
    pub fn store(&self) -> Option<Arc<Store>> {
        self.inner.store.get().cloned()
    }

    /// Changes the local minimum upload size at run time (Go tests assign
    /// `server.Config.MinUploadSize`).
    pub fn set_min_upload_size(&self, bytes: i64) {
        self.inner.min_upload_size.store(bytes, Ordering::Relaxed);
    }

    /// Fires once serving ended: after [`Server::stop`], or when the
    /// listener failed (Go `Done`).
    #[must_use]
    pub fn done(&self) -> CancellationToken {
        self.done.clone()
    }

    /// Why serving ended on its own (Go `Err`); `None` while serving and
    /// after a stop.
    #[must_use]
    pub fn serve_error(&self) -> Option<String> {
        self.serve_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The status the health service reports for `service` (Go's health
    /// `Check`).
    pub fn health_check(&self, service: &str) -> Result<ServingStatus, Status> {
        self.inner.health.check(service)
    }

    /// Whether every check passes, and the failing checks (Go
    /// `healthManager.evaluate`).
    #[must_use]
    pub fn readiness(&self) -> (bool, Vec<&'static str>) {
        self.inner.health.evaluate()
    }

    /// The last result of a health check (`app`, `fibre_module`, `signer`,
    /// `validator`, `registration`, `store`).
    #[must_use]
    pub fn health_result(&self, check: &str) -> Option<crate::health::CheckResult> {
        self.inner.health.result(check)
    }

    /// Go `Server.UploadShard`, called in process.
    pub async fn upload_shard(
        &self,
        req: UploadShardRequest,
    ) -> Result<UploadShardResponse, Status> {
        self.upload_shard_with(&CallContext::new(), req).await
    }

    /// [`Self::upload_shard`] under a caller's [`CallContext`].
    pub async fn upload_shard_with(
        &self,
        ctx: &CallContext,
        req: UploadShardRequest,
    ) -> Result<UploadShardResponse, Status> {
        self.inner.upload_shard(ctx, req).await
    }

    /// Go `Server.DownloadShard`, called in process.
    pub async fn download_shard(
        &self,
        req: DownloadShardRequest,
    ) -> Result<DownloadShardResponse, Status> {
        self.inner.download_shard(req).await
    }

    /// Go `Server.Start`.
    #[allow(clippy::too_many_lines)]
    pub async fn start(&self) -> Result<(), StartError> {
        let listener = {
            let mut lc = self.lifecycle();
            if lc.started || lc.stopped {
                return Err(StartError::Other("fibre server already started".into()));
            }
            lc.started = true;
            lc.listener
                .take()
                .ok_or_else(|| StartError::Other("listener closed".into()))?
        };
        let inner = &self.inner;
        let chain_id = inner.state.chain_id();
        let _ = inner.chain_id.set(chain_id.clone());
        tracing::info!("signer ready");

        let cert = {
            let signer = BlockingSigner {
                signer: Arc::clone(&inner.signer),
                handle: tokio::runtime::Handle::current(),
            };
            let chain_id = chain_id.clone();
            tokio::task::spawn_blocking(move || tlsid::build_server_cert(&signer, &chain_id))
                .await
                .map_err(|e| StartError::Other(format!("building TLS cert: {e}")))?
                .map_err(|e| StartError::Other(format!("building TLS cert: {e}")))?
        };
        let tls = transport::tls_config(&cert)
            .map_err(|e| StartError::Other(format!("building TLS cert: {e}")))?;

        let pub_key = inner
            .signer
            .public_key()
            .await
            .map_err(|e| StartError::Other(format!("getting validator public key: {e}")))?;

        let store_cfg = inner.cfg.store.clone();
        let store = tokio::task::spawn_blocking(move || Store::open(&store_cfg))
            .await
            .map_err(|e| StartError::Other(format!("opening store: {e}")))?
            .map_err(|e| StartError::Other(format!("opening store: {e}")))?;
        let store = Arc::new(store);
        let _ = inner.store.set(Arc::clone(&store));

        {
            let store = Arc::clone(&store);
            let occ = Arc::clone(&inner.occupancy);
            tokio::task::spawn_blocking(move || {
                store::seed_occupancy(&store, &occ, &CancellationToken::new())
            })
            .await
            .map_err(|e| StartError::Other(format!("getting store size: {e}")))?
            .map_err(|e| StartError::Other(format!("getting store size: {e}")))?;
        }

        inner
            .recompute_budget()
            .await
            .map_err(|e| StartError::Other(format!("deriving initial storage budget: {e}")))?;
        if !inner.cfg.unlimited_budget && inner.occupancy.budget_bytes() <= 0 {
            tracing::warn!(
                "derived storage budget is 0 (validator not in the active set?); running without a storage limit until it is re-derived"
            );
        }

        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|e| StartError::Other(format!("opening gRPC listener: {e}")))?;

        let background = CancellationToken::new();
        let prune = {
            let inner = Arc::clone(inner);
            let cancel = background.clone();
            tokio::spawn(async move { inner.prune_loop(cancel).await })
        };

        let shutdown = CancellationToken::new();
        let force = CancellationToken::new();
        let connections = TaskTracker::new();
        let serve = {
            let opts = TransportOptions::from_config(&inner.cfg);
            let inner = Arc::clone(inner);
            let (shutdown, force, connections) =
                (shutdown.clone(), force.clone(), connections.clone());
            let done = self.done.clone();
            let serve_error = Arc::clone(&self.serve_error);
            tokio::spawn(async move {
                let res =
                    transport::serve(listener, tls, inner, opts, shutdown, force, connections)
                        .await;
                if let Err(e) = res {
                    tracing::error!(error = %e, "fibre gRPC server stopped serving");
                    *serve_error.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(e.to_string());
                }
                done.cancel();
            })
        };
        tracing::info!(addr = %self.addr, "serving gRPC");

        inner.health.start(
            HealthDeps {
                state: Arc::clone(&inner.state),
                signer: Arc::clone(&inner.signer),
                pub_key,
                store,
            },
            chain_id,
        );
        let health = {
            let inner = Arc::clone(inner);
            let cancel = background.clone();
            tokio::spawn(async move { inner.health.run(cancel).await })
        };

        self.lifecycle().running = Some(Running {
            shutdown,
            force,
            background,
            connections,
            serve,
            prune,
            health,
        });
        Ok(())
    }

    /// Go `Server.Stop`: publishes NOT_SERVING, stops the background
    /// loops, stops accepting and drains in-flight requests for up to
    /// `drain_timeout` (then drops the connections), and closes the
    /// store. Calling it again is a no-op.
    pub async fn stop(&self, drain_timeout: Duration) -> Result<(), String> {
        let running = {
            let mut lc = self.lifecycle();
            if lc.stopped {
                return Ok(());
            }
            lc.stopped = true;
            lc.listener.take();
            lc.running.take()
        };
        tracing::info!("stopping server");
        self.inner.health.stop();
        if let Some(r) = running {
            r.background.cancel();
            r.shutdown.cancel();
            r.connections.close();
            if tokio::time::timeout(drain_timeout, r.connections.wait())
                .await
                .is_err()
            {
                r.force.cancel();
                r.connections.wait().await;
            }
            let _ = r.serve.await;
            let _ = r.prune.await;
            let _ = r.health.await;
        }
        self.done.cancel();

        // Also after a failed start that opened the store.
        let mut err = None;
        if let Some(store) = self.inner.store.get() {
            let store = Arc::clone(store);
            let closed = tokio::task::spawn_blocking(move || store.close())
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            if let Err(e) = closed {
                tracing::error!(error = %e, "closing store");
                err = Some(e);
            }
        }
        err.map_or(Ok(()), Err)
    }
}

impl Inner {
    /// Go `recomputeBudget`.
    pub(crate) async fn recompute_budget(&self) -> Result<(), String> {
        if self.cfg.unlimited_budget {
            self.occupancy.set_budget(0);
            return Ok(());
        }
        let full_stake = self
            .state
            .full_stake_storage_budget()
            .await
            .map_err(|e| format!("querying full-stake storage budget: {e}"))?;
        if full_stake <= 0 {
            return Err(format!(
                "full-stake storage budget must be positive (got {full_stake}); pass --unlimited-budget to disable the limiter"
            ));
        }
        let set = self
            .state
            .head_validator_set()
            .await
            .map_err(|e| format!("fetching head validator set: {e}"))?;
        let key = self
            .signer
            .public_key()
            .await
            .map_err(|e| format!("getting validator public key: {e}"))?;
        let Some((_, ours)) = set.get_by_address(&crypto::ed25519::address(&key)) else {
            // Not in the active set: keep the budget already applied.
            return Ok(());
        };
        let rows = assigned_rows(
            ours.voting_power,
            set.total_voting_power(),
            self.cfg.original_rows,
            self.cfg.min_rows_per_validator,
            self.cfg.liveness_threshold,
        );
        let budget = derive_budget(
            full_stake,
            i64::try_from(rows).unwrap_or(i64::MAX),
            i64::try_from(self.cfg.original_rows).unwrap_or(i64::MAX),
        );
        if full_stake < i64::try_from(self.cfg.max_shard_size).unwrap_or(i64::MAX) {
            tracing::warn!(
                full_stake_budget = full_stake,
                max_shard_size = self.cfg.max_shard_size,
                "FullStakeStorageBudget is below one maximum shard; uploads near the maximum blob size will be rejected"
            );
        }
        if budget > 0
            && let Some(store) = self.store.get()
            && let Ok(avail) = store.disk_available()
            && avail < budget
        {
            tracing::warn!(
                available_bytes = avail,
                budget_bytes = budget,
                "available disk is below the storage budget; provision more or the disk will fill"
            );
        }
        self.occupancy.set_budget(budget);
        Ok(())
    }

    /// Go `startPruneLoop`: every prune interval, prune then recompute the
    /// budget (a failure keeps the previous budget).
    async fn prune_loop(&self, cancel: CancellationToken) {
        let period = self.cfg.tuning.prune_interval;
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {}
            }
            self.prune(&cancel).await;
            if let Err(e) = self.recompute_budget().await {
                self.metrics.budget_recompute_failed();
                tracing::warn!(error = %e, "budget recompute failed; keeping previous budget");
            }
        }
    }

    /// Go `prune`: one pass over everything due before now.
    pub(crate) async fn prune(&self, cancel: &CancellationToken) {
        let Some(store) = self.store.get().cloned() else {
            return;
        };
        let start = Instant::now();
        let occ = Arc::clone(&self.occupancy);
        let token = cancel.clone();
        let summary = tokio::task::spawn_blocking(move || {
            store::prune_expired(&store, &occ, SystemTime::now(), &token)
        })
        .await;
        match summary {
            Ok(s) => self.metrics.observe_prune(
                s.error.is_none() && s.delete_error.is_none(),
                start.elapsed(),
                s.pruned,
                s.freed,
            ),
            Err(e) => {
                tracing::error!(error = %e, "failed to prune store");
                self.metrics.observe_prune(false, start.elapsed(), 0, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestDeriveBudget.
    #[test]
    fn derive_budget_matches_go() {
        assert_eq!(derive_budget(i64::MAX, 4096, 4096), i64::MAX);
        assert_eq!(derive_budget(100, 1, 4), 25);
        assert_eq!(derive_budget(100, 2, 4), 50);
        assert_eq!(derive_budget(0, 10, 100), 0);
        assert_eq!(derive_budget(100, 0, 100), 0);
        assert_eq!(derive_budget(100, 10, 0), 0);
    }
}
