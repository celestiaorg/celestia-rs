//! The embedded Fibre data availability server (`[fibre]` in config.toml,
//! `start --fibre`): the `fibre-server` crate run inside `celestia-rs
//! start`, so a validator needs no separate Go `fibre` process.
//!
//! - **Configuration**: an optional `[fibre]` table in `config.toml` with
//!   the keys of Go's `server_config.toml` (absent: off). Flags and
//!   `CELESTIA_RS_FIBRE*` variables override it. Unknown keys are errors.
//! - **State**: read in process. Module queries run on a detached read-only
//!   view of the latest committed state: the app lock is held only to
//!   build the view, never while a promise is validated.
//! - **Signing**: a raw-bytes signer separate from consensus: the file key's
//!   seed, or a handle on the remote signer connection
//!   ([`privval::RemoteRawBytesSigner`]). It never touches the vote and
//!   proposal sign state.
//! - **Isolation**: the server runs on its own thread and runtime under a
//!   supervisor ([`Supervisor`]). A failure to start (busy port, chain
//!   below app version 10, store error) is logged and retried with backoff;
//!   a listener that dies is restarted. Consensus never waits for it.
//! - **Shutdown**: on SIGINT / SIGTERM readiness turns NOT_SERVING, in-flight
//!   requests drain for up to [`DRAIN_TIMEOUT`] and the store is closed.

use std::net::ToSocketAddrs as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, anyhow, bail};
use celestia_proto::celestia::fibre::v1 as fibre_pb;
use celestia_proto::celestia::valaddr::v1 as valaddr_pb;
use fibre_pb::PaymentPromise;
use fibre_server::{
    BoxFuture, ChainState, NodeStatus, ProviderRegistration, ServerConfig, ServerMetrics, Signer,
    SignerError, StateError, Validator, ValidatorSet, VerifiedPromise,
};
use nodestore::{BlockStore, StateStore};
use tokio_util::sync::CancellationToken;

use crate::config::{ConfigFiles, File};

/// Default store directory under the node home (`store_path`).
pub const DEFAULT_STORE_DIR: &str = "data/fibre";
/// How long a stop waits for in-flight requests before dropping them.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// First retry delay after a failed start; doubles up to [`MAX_BACKOFF`].
const MIN_BACKOFF: Duration = Duration::from_secs(1);
/// Longest retry delay.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The server re-issues its TLS certificate (valid 365 days) by restarting
/// after this long.
const CERT_RENEWAL: Duration = Duration::from_secs(360 * 24 * 3600);
/// Bytes one in-flight upload stream may buffer (the maximum gRPC message,
/// about 132 MiB), for the worst-case memory figure in the start log.
const PER_STREAM_BYTES: u64 = 132 << 20;

/// The type of a `[fibre]` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// TOML boolean.
    Bool,
    /// TOML integer.
    Int,
    /// TOML string.
    Str,
}

/// Every `[fibre]` key (dotted, under `fibre.`) and its type.
pub const KEYS: &[(&str, KeyKind)] = &[
    ("enable", KeyKind::Bool),
    ("server_listen_address", KeyKind::Str),
    ("min_upload_size", KeyKind::Int),
    ("max_connections", KeyKind::Int),
    ("max_concurrent_streams", KeyKind::Int),
    ("upload_verify_workers", KeyKind::Int),
    ("unlimited_budget", KeyKind::Bool),
    ("storage_backend", KeyKind::Str),
    ("store_path", KeyKind::Str),
    ("health.check_interval", KeyKind::Str),
    ("health.probe_timeout", KeyKind::Str),
    ("health.max_block_age", KeyKind::Str),
];

/// Keys of Go's standalone `server_config.toml` the embedded server has no
/// use for.
const GO_ONLY_KEYS: &[&str] = &[
    "app_grpc_address",
    "signer_grpc_address",
    "signer_grpc_ca_file",
    "signer_grpc_cert_file",
    "signer_grpc_key_file",
    "signer_grpc_allow_insecure",
];

/// The type of `fibre.<key>` (`key` without the `fibre.` prefix).
#[must_use]
pub fn key_kind(key: &str) -> Option<KeyKind> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, kind)| *kind)
}

/// `start` flags (or their `CELESTIA_RS_FIBRE*` variables) that override
/// `[fibre]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FibreFlags {
    /// `--fibre[=true|false]`.
    pub enable: Option<bool>,
    /// `--fibre-listen-address` (enables the server unless `enable` is
    /// `Some(false)`).
    pub listen_address: Option<String>,
}

/// The effective Fibre server configuration: `None` when disabled. Code
/// defaults (Go's) < `[fibre]` in `config.toml` < environment < flags. The
/// whole table is checked even when the server is disabled, so a typo
/// fails `start` instead of silently dropping a setting.
pub fn options(files: &ConfigFiles, flags: &FibreFlags) -> anyhow::Result<Option<ServerConfig>> {
    let table = match files.value(File::Config, "fibre") {
        None => toml::Table::new(),
        Some(toml::Value::Table(t)) => t.clone(),
        Some(_) => bail!("fibre in config.toml must be a table ([fibre])"),
    };
    let mut flat = Vec::new();
    for (k, v) in &table {
        match (k.as_str(), v) {
            ("health", toml::Value::Table(h)) => {
                for (hk, hv) in h {
                    flat.push((format!("health.{hk}"), hv));
                }
            }
            _ => flat.push((k.clone(), v)),
        }
    }
    let mut cfg = ServerConfig::default();
    let mut enable = false;
    let mut store_path = None;
    for (key, value) in flat {
        if GO_ONLY_KEYS.contains(&key.as_str()) {
            bail!(
                "fibre.{key} in config.toml is a setting of the standalone Go fibre server; \
                 the embedded server reads state and signs in process. Remove it."
            );
        }
        if key == "object_storage" || key.starts_with("object_storage.") {
            bail!(
                "fibre.object_storage in config.toml: object storage is not supported by \
                 celestia-rs; the embedded server stores shards locally (store_path)"
            );
        }
        let Some(kind) = key_kind(&key) else {
            let known: Vec<&str> = KEYS.iter().map(|(k, _)| *k).collect();
            bail!(
                "unknown key fibre.{key} in config.toml (known: {})",
                known.join(", ")
            );
        };
        let bad = |want: &str| anyhow!("fibre.{key} in config.toml must be {want}, got {value}");
        match kind {
            KeyKind::Bool => {
                let b = value.as_bool().ok_or_else(|| bad("true or false"))?;
                match key.as_str() {
                    "enable" => enable = b,
                    _ => cfg.unlimited_budget = b,
                }
            }
            KeyKind::Int => {
                let i = value.as_integer().ok_or_else(|| bad("an integer"))?;
                match key.as_str() {
                    "min_upload_size" => cfg.min_upload_size = i,
                    "max_connections" => cfg.max_connections = i,
                    "max_concurrent_streams" => cfg.max_concurrent_streams = i,
                    _ => cfg.upload_verify_workers = i,
                }
            }
            KeyKind::Str => {
                let s = value.as_str().ok_or_else(|| bad("a string"))?.to_owned();
                match key.as_str() {
                    "server_listen_address" => cfg.server_listen_address = s,
                    "storage_backend" => {
                        cfg.store.storage_backend = s
                            .parse()
                            .map_err(|e| anyhow!("fibre.storage_backend: {e}"))?;
                    }
                    "store_path" => store_path = Some(s),
                    "health.check_interval" => cfg.health.check_interval = s,
                    "health.probe_timeout" => cfg.health.probe_timeout = s,
                    _ => cfg.health.max_block_age = s,
                }
            }
        }
    }
    if let Some(addr) = &flags.listen_address {
        cfg.server_listen_address.clone_from(addr);
        enable = true;
    }
    if let Some(e) = flags.enable {
        enable = e;
    }
    cfg.server_listen_address = listen_address(&cfg.server_listen_address)?;
    cfg.store.path = match store_path.as_deref() {
        None | Some("") => files.home().join(DEFAULT_STORE_DIR),
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => files.home().join(p),
    };
    cfg.validate()
        .map_err(|e| anyhow!("invalid [fibre] configuration: {e}"))?;
    Ok(enable.then_some(cfg))
}

/// Checks a `host:port` listen address (Go's `:7980` means every
/// interface).
fn listen_address(addr: &str) -> anyhow::Result<String> {
    let addr = match addr.strip_prefix(':') {
        Some(port) => format!("0.0.0.0:{port}"),
        None => addr.to_owned(),
    };
    let resolved = addr.to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
    if !resolved {
        bail!("fibre server_listen_address {addr:?} is not a host:port address");
    }
    Ok(addr)
}

/// Worst-case memory of buffered uploads: `max_connections ×
/// max_concurrent_streams × ~132 MiB`.
#[must_use]
pub fn worst_case_upload_bytes(cfg: &ServerConfig) -> u64 {
    u64::try_from(cfg.max_connections)
        .unwrap_or(0)
        .saturating_mul(u64::try_from(cfg.max_concurrent_streams).unwrap_or(0))
        .saturating_mul(PER_STREAM_BYTES)
}

// ------------------------------------------------------------------ state

/// [`ChainState`] over the node's own stores and app.
pub struct NodeChainState {
    chain_id: String,
    app: Arc<Mutex<app::App>>,
    block_store: BlockStore,
    state_store: StateStore,
    env: Arc<rpc::Environment>,
}

impl NodeChainState {
    /// State for `chain_id` from the node's app, stores and RPC
    /// environment (node status).
    #[must_use]
    pub fn new(
        chain_id: &str,
        app: Arc<Mutex<app::App>>,
        block_store: BlockStore,
        state_store: StateStore,
        env: Arc<rpc::Environment>,
    ) -> Self {
        Self {
            chain_id: chain_id.to_owned(),
            app,
            block_store,
            state_store,
            env,
        }
    }

    /// Runs module query `path` like Go's app gRPC client would see it:
    /// the view is built under the app lock, the handler runs without it.
    async fn query<Req, Resp>(&self, path: &'static str, req: &Req) -> Result<Resp, StateError>
    where
        Req: prost::Message,
        Resp: prost::Message + Default,
    {
        let data = req.encode_to_vec();
        let app = Arc::clone(&self.app);
        let res = tokio::task::spawn_blocking(move || {
            let prepared = app
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .detached_grpc_query(path);
            prepared.and_then(|q| q.run(&data))
        })
        .await
        .map_err(|e| StateError::other(format!("query task failed: {e}")))?;
        match res {
            Ok((bytes, _)) => Resp::decode(bytes.as_slice()).map_err(|e| {
                StateError::other(format!(
                    "rpc error: code = Internal desc = grpc: failed to unmarshal the received message: {e}"
                ))
            }),
            Err(e) => Err(grpc_error(&e)),
        }
    }

    async fn validators_at(&self, height: Option<u64>) -> Result<ValidatorSet, StateError> {
        let bs = self.block_store.clone();
        let ss = self.state_store.clone();
        tokio::task::spawn_blocking(move || {
            let requested = height.unwrap_or(0);
            let h = match height {
                Some(h) => i64::try_from(h).unwrap_or(i64::MAX),
                None => bs.height(),
            };
            let set = ss.load_validators(h).map_err(|e| {
                StateError::other(format!(
                    "getting validator set at height {requested}: rpc error: code = Unknown desc = {e}"
                ))
            })?;
            let validators = set
                .validators
                .iter()
                .map(|v| match &v.pub_key {
                    ctypes::PubKey::Ed25519(pk) => Ok(Validator {
                        address: v.address.as_slice().try_into().map_err(|_| {
                            StateError::other(format!("validator address of {} bytes", v.address.len()))
                        })?,
                        pub_key: *pk,
                        voting_power: v.voting_power,
                    }),
                    ctypes::PubKey::Secp256k1(_) => Err(StateError::other(format!(
                        "validator {} has a secp256k1 key; Fibre needs Ed25519 consensus keys",
                        hex::encode_upper(&v.address)
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ValidatorSet {
                validators,
                height: u64::try_from(h).unwrap_or(0),
            })
        })
        .await
        .map_err(|e| StateError::other(format!("validator set task failed: {e}")))?
    }
}

/// The error text of a failed query as a Go gRPC client prints it;
/// `Unimplemented` (no Fibre module before app version 10) is
/// [`StateError::ModuleUnavailable`].
fn grpc_error(e: &app::query::GrpcQueryError) -> StateError {
    let (code, msg) = e.grpc_status();
    let text = format!("rpc error: code = {} desc = {msg}", code.name());
    if code.name() == "Unimplemented" {
        StateError::ModuleUnavailable(text)
    } else {
        StateError::Other(text)
    }
}

fn system_time(seconds: i64, nanos: i32) -> SystemTime {
    let d = Duration::new(
        seconds.unsigned_abs(),
        u32::try_from(nanos).unwrap_or(0).min(999_999_999),
    );
    if seconds >= 0 {
        UNIX_EPOCH + d
    } else {
        UNIX_EPOCH - d
    }
}

impl ChainState for NodeChainState {
    fn chain_id(&self) -> String {
        self.chain_id.clone()
    }

    fn validator_set(&self, height: u64) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
        Box::pin(async move {
            if height == 0 {
                return Err(StateError::other(
                    "height must be greater than 0, use Head() to get the latest validator set",
                ));
            }
            self.validators_at(Some(height)).await
        })
    }

    fn head_validator_set(&self) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
        Box::pin(self.validators_at(None))
    }

    fn verify_promise<'a>(
        &'a self,
        promise: &'a PaymentPromise,
    ) -> BoxFuture<'a, Result<VerifiedPromise, StateError>> {
        Box::pin(async move {
            let resp: fibre_pb::QueryValidatePaymentPromiseResponse = self
                .query(
                    "/celestia.fibre.v1.Query/ValidatePaymentPromise",
                    &fibre_pb::QueryValidatePaymentPromiseRequest {
                        promise: Some(promise.clone()),
                    },
                )
                .await?;
            if !resp.is_valid {
                return Err(StateError::other("payment promise is invalid"));
            }
            let Some(exp) = resp.expiration_time else {
                return Err(StateError::other(
                    "expiration time not provided in validation response",
                ));
            };
            let retention = resp.shard_retention.unwrap_or_default();
            Ok(VerifiedPromise {
                expires_at: system_time(exp.seconds, exp.nanos),
                shard_retention: Duration::new(
                    u64::try_from(retention.seconds).unwrap_or(0),
                    u32::try_from(retention.nanos).unwrap_or(0).min(999_999_999),
                ),
            })
        })
    }

    fn full_stake_storage_budget(&self) -> BoxFuture<'_, Result<i64, StateError>> {
        Box::pin(async move {
            let resp: fibre_pb::QueryParamsResponse = self
                .query(
                    "/celestia.fibre.v1.Query/Params",
                    &fibre_pb::QueryParamsRequest {},
                )
                .await?;
            let budget = resp.params.unwrap_or_default().full_stake_storage_budget;
            Ok(i64::try_from(budget).unwrap_or(i64::MAX))
        })
    }

    fn node_status(&self) -> BoxFuture<'_, Result<NodeStatus, StateError>> {
        let env = Arc::clone(&self.env);
        Box::pin(async move {
            let st = tokio::task::spawn_blocking(move || env.status_parts())
                .await
                .map_err(|e| StateError::other(format!("status task failed: {e}")))?
                .map_err(|e| StateError::other(format!("rpc error: code = Unknown desc = {e}")))?;
            Ok(NodeStatus {
                chain_id: st.node_info.network.trim().to_owned(),
                height: u64::try_from(st.latest_height.max(0)).unwrap_or(0),
                block_time: Some(system_time(st.latest_time.seconds, st.latest_time.nanos)),
                catching_up: st.catching_up,
            })
        })
    }

    fn provider_registration<'a>(
        &'a self,
        consensus_address: &'a [u8; 20],
    ) -> BoxFuture<'a, Result<ProviderRegistration, StateError>> {
        Box::pin(async move {
            let resp: valaddr_pb::QueryFibreProviderInfoResponse = self
                .query(
                    "/celestia.valaddr.v1.Query/FibreProviderInfo",
                    &valaddr_pb::QueryFibreProviderInfoRequest {
                        validator_consensus_address: sdk::address::to_bech32(
                            sdk::address::VALCONS_PREFIX,
                            consensus_address,
                        ),
                    },
                )
                .await?;
            Ok(match resp.info {
                Some(info) if resp.found => ProviderRegistration {
                    found: true,
                    host: info.host,
                },
                _ => ProviderRegistration::default(),
            })
        })
    }
}

// ----------------------------------------------------------------- signer

/// The validator's consensus key for Fibre: the file key, or a handle on
/// the remote signer connection.
pub enum NodeSigner {
    /// `priv_validator_key.json` (signs in process).
    File(fibre_server::tlsid::Ed25519RawBytesSigner),
    /// The remote signer at `priv_validator_laddr`.
    Remote(privval::RemoteRawBytesSigner),
}

impl NodeSigner {
    /// The signer of a `priv_validator_key.json` key.
    pub fn from_key_file(path: &Path) -> anyhow::Result<Self> {
        let bz = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let key = privval::FilePvKey::from_json(&bz).map_err(|e| anyhow!("{e}"))?;
        let seed: [u8; 32] = key.priv_key[..32]
            .try_into()
            .map_err(|_| anyhow!("short validator key"))?;
        let signer = fibre_server::tlsid::Ed25519RawBytesSigner::from_seed(&seed);
        if signer.public_key() != key.pub_key {
            bail!(
                "{}: the public key does not match the private key",
                path.display()
            );
        }
        Ok(Self::File(signer))
    }
}

impl NodeSigner {
    /// Why the signer cannot sign raw bytes until the node restarts (the
    /// remote signer's raw-bytes path was disabled); `None` otherwise.
    #[must_use]
    pub fn disabled(&self) -> Option<String> {
        match self {
            Self::File(_) => None,
            Self::Remote(r) => r.disabled(),
        }
    }
}

impl Signer for NodeSigner {
    /// With the raw-bytes path disabled this fails, so the `signer` health
    /// check reports it (`signer_unreachable`, with the reason).
    fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>> {
        let res = match self {
            Self::File(s) => Ok(s.public_key()),
            Self::Remote(r) if r.disabled().is_some() => Err(SignerError(format!(
                "remote signer raw-bytes signing is disabled: {}",
                r.disabled().unwrap_or_default()
            ))),
            Self::Remote(r) => match r.pub_key() {
                ctypes::PubKey::Ed25519(pk) => Ok(pk),
                ctypes::PubKey::Secp256k1(_) => Err(SignerError(
                    "the remote signer holds a secp256k1 key; Fibre needs an Ed25519 consensus key"
                        .into(),
                )),
            },
        };
        Box::pin(async move { res })
    }

    fn sign_raw_bytes<'a>(
        &'a self,
        chain_id: &'a str,
        unique_id: &'a str,
        raw: &'a [u8],
    ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>> {
        match self {
            Self::File(s) => Signer::sign_raw_bytes(s, chain_id, unique_id, raw),
            Self::Remote(r) => {
                let (r, chain_id, unique_id, raw) = (
                    r.clone(),
                    chain_id.to_owned(),
                    unique_id.to_owned(),
                    raw.to_vec(),
                );
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || {
                        r.sign_raw_bytes(&chain_id, &unique_id, &raw)
                            .map_err(|e| SignerError(e.0))
                    })
                    .await
                    .map_err(|e| SignerError(format!("signer task failed: {e}")))?
                })
            }
        }
    }
}

// ------------------------------------------------------------- supervisor

/// Reports a failure that retrying cannot fix (the remote signer's
/// raw-bytes path is disabled); the supervisor then stops retrying.
pub type FatalCheck = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The current server's metrics, for the node's Prometheus endpoint.
pub type MetricsSlot = Arc<Mutex<Option<Arc<ServerMetrics>>>>;

/// Runs the Fibre server on its own thread and runtime: starts it, retries
/// failed starts with backoff, restarts a server whose listener died and
/// stops it gracefully. Dropping the supervisor stops the server and
/// waits for the drain.
pub struct Supervisor {
    stop: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    metrics: MetricsSlot,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor").finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Starts supervising a server with `cfg` (already validated).
    pub fn start(
        cfg: ServerConfig,
        state: Arc<dyn ChainState>,
        signer: Arc<dyn Signer>,
        validator: String,
        fatal: FatalCheck,
    ) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fibre")
            .enable_all()
            .build()
            .context("start the fibre runtime")?;
        let stop = CancellationToken::new();
        let metrics: MetricsSlot = Arc::default();
        let thread = {
            let (stop, metrics) = (stop.clone(), Arc::clone(&metrics));
            std::thread::Builder::new()
                .name("fibre-supervisor".into())
                .spawn(move || {
                    runtime.block_on(supervise(
                        cfg, state, signer, validator, stop, metrics, fatal,
                    ));
                    runtime.shutdown_timeout(Duration::from_secs(1));
                })
                .context("start the fibre supervisor thread")?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
            metrics,
        })
    }

    /// A token that stops the server when cancelled (the shutdown hook).
    #[must_use]
    pub fn stop_token(&self) -> CancellationToken {
        self.stop.clone()
    }

    /// The current server's metrics (empty while no server runs).
    #[must_use]
    pub fn metrics(&self) -> MetricsSlot {
        Arc::clone(&self.metrics)
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Sleeps `d` unless `stop` fires first (then returns true).
async fn wait(stop: &CancellationToken, d: Duration) -> bool {
    tokio::select! {
        () = stop.cancelled() => true,
        () = tokio::time::sleep(d) => false,
    }
}

async fn supervise(
    cfg: ServerConfig,
    state: Arc<dyn ChainState>,
    signer: Arc<dyn Signer>,
    validator: String,
    stop: CancellationToken,
    metrics: MetricsSlot,
    fatal: FatalCheck,
) {
    // A failure retrying cannot fix: log once, then wait for shutdown.
    let give_up = |stop: CancellationToken, fatal: FatalCheck| async move {
        let reason = fatal()?;
        tracing::error!(
            reason,
            "the fibre server stays off until the node restarts; consensus is not affected"
        );
        stop.cancelled().await;
        Some(())
    };
    let mut backoff = MIN_BACKOFF;
    let retry = |backoff: &mut Duration| {
        let d = *backoff;
        *backoff = (*backoff * 2).min(MAX_BACKOFF);
        d
    };
    while !stop.is_cancelled() {
        let server = match fibre_server::Server::new(
            cfg.clone(),
            Arc::clone(&state),
            Arc::clone(&signer),
        ) {
            Ok(s) => s,
            Err(e) => {
                let d = retry(&mut backoff);
                tracing::error!(error = %e, retry_in = ?d, "fibre server failed to start; consensus is not affected");
                if wait(&stop, d).await {
                    break;
                }
                continue;
            }
        };
        let started = tokio::select! {
            r = server.start() => Some(r),
            () = stop.cancelled() => None,
        };
        match started {
            None => {
                let _ = server.stop(Duration::ZERO).await;
                break;
            }
            Some(Err(e)) => {
                // Closes a store the failed start opened.
                let _ = server.stop(Duration::ZERO).await;
                drop(server);
                if give_up(stop.clone(), Arc::clone(&fatal)).await.is_some() {
                    break;
                }
                let d = retry(&mut backoff);
                tracing::error!(error = %e, retry_in = ?d, "fibre server failed to start; consensus is not affected");
                if wait(&stop, d).await {
                    break;
                }
                continue;
            }
            Some(Ok(())) => {}
        }
        backoff = MIN_BACKOFF;
        *metrics.lock().unwrap_or_else(PoisonError::into_inner) = Some(server.metrics());
        tracing::info!(
            listen = %server.listen_address(),
            store = %cfg.store.path.display(),
            validator = %validator,
            chain_id = %server.chain_id(),
            worst_case_upload_memory_mib = worst_case_upload_bytes(&cfg) >> 20,
            "fibre server started"
        );
        let done = server.done();
        tokio::select! {
            () = stop.cancelled() => {
                tracing::info!("stopping fibre server (draining in-flight requests)");
                if let Err(e) = server.stop(DRAIN_TIMEOUT).await {
                    tracing::error!(error = %e, "fibre server stopped with an error");
                } else {
                    tracing::info!("fibre server stopped");
                }
                break;
            }
            () = done.cancelled() => {
                let reason = server.serve_error().unwrap_or_else(|| "unknown".into());
                let _ = server.stop(Duration::ZERO).await;
                tracing::error!(error = %reason, "fibre server stopped serving");
                if give_up(stop.clone(), Arc::clone(&fatal)).await.is_some() {
                    break;
                }
                let d = retry(&mut backoff);
                tracing::error!(retry_in = ?d, "restarting the fibre server");
                if wait(&stop, d).await {
                    break;
                }
            }
            () = tokio::time::sleep(CERT_RENEWAL) => {
                tracing::info!("restarting the fibre server to renew its TLS certificate");
                if let Err(e) = server.stop(DRAIN_TIMEOUT).await {
                    tracing::error!(error = %e, "fibre server stopped with an error");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain the server never gets past: start fails at the TLS
    /// certificate's signature first.
    struct NoChain;

    impl ChainState for NoChain {
        fn chain_id(&self) -> String {
            "c".into()
        }
        fn validator_set(&self, _: u64) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
        fn head_validator_set(&self) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
        fn verify_promise<'a>(
            &'a self,
            _: &'a PaymentPromise,
        ) -> BoxFuture<'a, Result<VerifiedPromise, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
        fn full_stake_storage_budget(&self) -> BoxFuture<'_, Result<i64, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
        fn node_status(&self) -> BoxFuture<'_, Result<NodeStatus, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
        fn provider_registration<'a>(
            &'a self,
            _: &'a [u8; 20],
        ) -> BoxFuture<'a, Result<ProviderRegistration, StateError>> {
            Box::pin(async { Err(StateError::other("no chain")) })
        }
    }

    /// A signer whose raw-bytes path is broken; counts signing attempts.
    #[derive(Default)]
    struct BrokenSigner {
        attempts: std::sync::atomic::AtomicUsize,
    }

    impl Signer for BrokenSigner {
        fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>> {
            Box::pin(async { Ok([1; 32]) })
        }
        fn sign_raw_bytes<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: &'a [u8],
        ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { Err(SignerError("remote signer connection: hung up".into())) })
        }
    }

    /// Once the signer reports a permanent failure, the supervisor stops
    /// retrying (each retry would cost another signer request).
    #[test]
    fn a_permanent_signer_failure_stops_the_retries() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ServerConfig {
            server_listen_address: "127.0.0.1:0".into(),
            store: fibre_server::store::StoreConfig::new(dir.path().join("fibre")),
            ..ServerConfig::default()
        };
        let signer = Arc::new(BrokenSigner::default());
        let fatal: FatalCheck = {
            let s = Arc::clone(&signer);
            Arc::new(move || {
                (s.attempts.load(std::sync::atomic::Ordering::SeqCst) > 0)
                    .then(|| "the signer cannot sign raw bytes".to_owned())
            })
        };
        let sup = Supervisor::start(
            cfg,
            Arc::new(NoChain),
            Arc::clone(&signer) as Arc<dyn Signer>,
            "v".into(),
            fatal,
        )
        .unwrap();
        // Without the check, the 1 s and 2 s backoffs would retry twice.
        std::thread::sleep(Duration::from_millis(3500));
        assert_eq!(signer.attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(sup);
    }

    fn files(config: &str) -> (tempfile::TempDir, ConfigFiles) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config/config.toml"),
            crate::config::CONFIG_TEMPLATE.replace("{{MONIKER}}", "m") + config,
        )
        .unwrap();
        let f = ConfigFiles::load(dir.path()).unwrap();
        (dir, f)
    }

    fn err(config: &str, flags: &FibreFlags) -> String {
        options(&files(config).1, flags).unwrap_err().to_string()
    }

    #[test]
    fn absent_section_is_off_with_go_defaults() {
        let (dir, f) = files("");
        assert_eq!(options(&f, &FibreFlags::default()).unwrap(), None);
        let cfg = options(
            &f,
            &FibreFlags {
                enable: Some(true),
                ..FibreFlags::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(cfg.server_listen_address, "0.0.0.0:7980");
        assert_eq!(cfg.store.path, dir.path().join("data/fibre"));
        assert_eq!(cfg.max_connections, 16);
        assert_eq!(cfg.max_concurrent_streams, 13);
        assert_eq!(cfg.min_upload_size, 262_144);
        assert!(!cfg.unlimited_budget);
        assert_eq!(cfg.health.check_interval, "10s");
        assert_eq!(worst_case_upload_bytes(&cfg), 16 * 13 * (132 << 20));
    }

    #[test]
    fn file_values_and_flag_precedence() {
        let toml = "\n[fibre]\nenable = true\nserver_listen_address = \"127.0.0.1:7999\"\n\
            min_upload_size = 33554432\nmax_connections = 20\nmax_concurrent_streams = 2\n\
            upload_verify_workers = 3\nunlimited_budget = true\nstorage_backend = \"local\"\n\
            store_path = \"/mnt/fibre\"\n[fibre.health]\ncheck_interval = \"1s\"\n\
            probe_timeout = \"500ms\"\nmax_block_age = \"30s\"\n";
        let (_dir, f) = files(toml);
        let cfg = options(&f, &FibreFlags::default()).unwrap().unwrap();
        assert_eq!(cfg.server_listen_address, "127.0.0.1:7999");
        assert_eq!(cfg.min_upload_size, 32 << 20);
        assert_eq!(cfg.max_connections, 20);
        assert_eq!(cfg.max_concurrent_streams, 2);
        assert_eq!(cfg.upload_verify_workers, 3);
        assert!(cfg.unlimited_budget);
        assert_eq!(cfg.store.path, PathBuf::from("/mnt/fibre"));
        assert_eq!(cfg.health.check_interval, "1s");
        assert_eq!(cfg.health.probe_timeout, "500ms");
        assert_eq!(cfg.health.max_block_age, "30s");

        // Flags win over the file.
        let cfg = options(
            &f,
            &FibreFlags {
                enable: None,
                listen_address: Some("127.0.0.1:8001".into()),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(cfg.server_listen_address, "127.0.0.1:8001");
        assert_eq!(
            options(
                &f,
                &FibreFlags {
                    enable: Some(false),
                    listen_address: Some("127.0.0.1:8001".into()),
                },
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn listen_address_flag_implies_enable() {
        let (dir, f) = files("\n[fibre]\nenable = false\nstore_path = \"shards\"\n");
        let cfg = options(
            &f,
            &FibreFlags {
                enable: None,
                listen_address: Some(":7981".into()),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(cfg.server_listen_address, "0.0.0.0:7981");
        assert_eq!(cfg.store.path, dir.path().join("shards"));
    }

    #[test]
    fn query_errors_read_like_go_grpc_client_errors() {
        use app::query::GrpcQueryError;
        assert_eq!(
            grpc_error(&GrpcQueryError::UnknownService(
                "celestia.fibre.v1.Query".into()
            )),
            StateError::ModuleUnavailable(
                "rpc error: code = Unimplemented desc = unknown service celestia.fibre.v1.Query"
                    .into()
            )
        );
        let invalid = GrpcQueryError::Handler(grpc_query::QueryError::Status(
            grpc_query::Code::InvalidArgument,
            "bad promise".into(),
        ));
        assert_eq!(
            grpc_error(&invalid),
            StateError::Other("rpc error: code = InvalidArgument desc = bad promise".into())
        );
    }

    #[test]
    fn times_convert_like_go() {
        assert_eq!(system_time(0, 0), UNIX_EPOCH);
        assert_eq!(system_time(10, 5), UNIX_EPOCH + Duration::new(10, 5));
        assert_eq!(system_time(-1, 0), UNIX_EPOCH - Duration::from_secs(1));
    }

    #[test]
    fn file_signer_signs_raw_bytes_with_the_consensus_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("priv_validator_key.json");
        let key =
            privval::FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(&[9; 32]))
                .unwrap();
        std::fs::write(&path, key.to_json()).unwrap();
        let signer = NodeSigner::from_key_file(&path).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let pk = rt.block_on(signer.public_key()).unwrap();
        assert_eq!(pk, key.pub_key);
        let sig = rt
            .block_on(Signer::sign_raw_bytes(&signer, "c", "fibre", b"promise"))
            .unwrap();
        // The same signature the consensus signer makes.
        let pv = privval::FilePv::new(key, None, None);
        assert_eq!(
            sig,
            privval::PrivValidator::sign_raw_bytes(&pv, "c", "fibre", b"promise").unwrap()
        );
        assert!(NodeSigner::from_key_file(&dir.path().join("missing.json")).is_err());
    }

    #[test]
    fn invalid_settings_fail_early() {
        let on = FibreFlags::default();
        assert!(
            err("\n[fibre]\nenabel = true\n", &on).starts_with("unknown key fibre.enabel"),
            "{}",
            err("\n[fibre]\nenabel = true\n", &on)
        );
        assert!(
            err("\n[fibre.health]\ninterval = \"1s\"\n", &on)
                .starts_with("unknown key fibre.health.interval")
        );
        assert!(
            err("\n[fibre]\napp_grpc_address = \"127.0.0.1:9090\"\n", &on)
                .contains("standalone Go fibre server")
        );
        assert!(
            err("\n[fibre.object_storage]\nbucket = \"b\"\n", &on)
                .contains("object storage is not supported")
        );
        assert_eq!(
            err("\n[fibre]\nenable = \"yes\"\n", &on),
            "fibre.enable in config.toml must be true or false, got \"yes\""
        );
        assert_eq!(
            err("\n[fibre]\nmax_connections = \"16\"\n", &on),
            "fibre.max_connections in config.toml must be an integer, got \"16\""
        );
        assert_eq!(
            err("\n[fibre]\nmax_connections = 0\n", &on),
            "invalid [fibre] configuration: max_connections must be at least 1, got 0"
        );
        assert_eq!(
            err("\n[fibre]\nstorage_backend = \"object\"\n", &on),
            "invalid [fibre] configuration: store config: fibre storage_backend \"object\" is not supported by celestia-rs; use \"local\""
        );
        assert_eq!(
            err("\n[fibre]\nserver_listen_address = \"nonsense\"\n", &on),
            "fibre server_listen_address \"nonsense\" is not a host:port address"
        );
        assert_eq!(
            err("\n[fibre.health]\nprobe_timeout = \"0s\"\n", &on),
            "invalid [fibre] configuration: health config: health.probe_timeout must be a positive duration, got \"0s\""
        );
        // Checked even when the server stays off.
        assert!(
            err("\n[fibre]\nenable = false\nmin_upload_size = 0\n", &on)
                .contains("min_upload_size must be between 1")
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/config.toml"), "fibre = 1\n").unwrap();
        assert_eq!(
            options(&ConfigFiles::load(dir.path()).unwrap(), &on)
                .unwrap_err()
                .to_string(),
            "fibre in config.toml must be a table ([fibre])"
        );
    }
}
