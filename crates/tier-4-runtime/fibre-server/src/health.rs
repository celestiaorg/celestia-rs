//! Readiness checks behind the gRPC health service (Go
//! `server_health.go`).
//!
//! The `grpc.health.v1.Health` service answers for three names: the
//! liveness service [`HEALTH_SERVICE_LIVENESS`], SERVING whenever the
//! server answers, and the readiness services `""` and
//! `celestia.fibre.v1.Fibre`, SERVING once start finished and every check
//! passed. The checks (`app`, `fibre_module`, `signer`, `validator`,
//! `registration`, `store`) run one after another every check interval,
//! each bounded by the probe timeout; a change of a check's result is
//! logged with its reason code. Stopping publishes NOT_SERVING for every
//! service before the server drains, and later results are ignored.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::config::HealthSettings;
use crate::metrics::ServerMetrics;
use crate::proto::ServingStatus;
use crate::state::{ChainState, Signer, StateError};
use crate::store::Store;

/// Go `HealthServiceLiveness`.
pub const HEALTH_SERVICE_LIVENESS: &str = "fibre-liveness";
/// The readiness services (Go `readinessServices`).
pub const READINESS_SERVICES: [&str; 2] = ["", "celestia.fibre.v1.Fibre"];

/// Reason codes of failed checks (Go's `reason*` constants).
pub mod reason {
    /// The app did not answer.
    pub const APP_UNREACHABLE: &str = "app_unreachable";
    /// The app is syncing.
    pub const APP_SYNCING: &str = "app_syncing";
    /// The latest block is too old.
    pub const CHAIN_STALLED: &str = "chain_stalled";
    /// The app reports another chain ID.
    pub const CHAIN_MISMATCH: &str = "chain_id_mismatch";
    /// The app has no Fibre module.
    pub const MODULE_UNAVAILABLE: &str = "fibre_module_unavailable";
    /// The signer did not answer.
    pub const SIGNER_UNREACHABLE: &str = "signer_unreachable";
    /// The signer's key changed since start.
    pub const SIGNER_KEY_CHANGED: &str = "signer_key_changed";
    /// The validator is not in the active set.
    pub const VALIDATOR_INACTIVE: &str = "validator_not_active";
    /// No Fibre provider registration.
    pub const NOT_REGISTERED: &str = "provider_not_registered";
    /// The registered host is not `host:port`.
    pub const HOST_INVALID: &str = "provider_host_invalid";
    /// The store failed a probe.
    pub const STORE_FAILED: &str = "store_failed";
}

/// The check names, in run order.
pub const CHECKS: [&str; 6] = [
    "app",
    "fibre_module",
    "signer",
    "validator",
    "registration",
    "store",
];

/// The outcome of one check (Go `checkResult`). An empty reason passed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckResult {
    /// Reason code; empty when the check passed.
    pub reason: &'static str,
    /// What to do about it.
    pub message: String,
    /// The underlying error (logged, never served).
    pub error: Option<String>,
}

impl CheckResult {
    fn passed() -> Self {
        Self::default()
    }

    fn failed(reason: &'static str, message: impl Into<String>, error: Option<String>) -> Self {
        Self {
            reason,
            message: message.into(),
            error,
        }
    }
}

/// What the checks probe (Go `healthDeps`).
#[derive(Clone)]
pub(crate) struct HealthDeps {
    pub(crate) state: Arc<dyn ChainState>,
    pub(crate) signer: Arc<dyn Signer>,
    /// The signer's key at start.
    pub(crate) pub_key: [u8; 32],
    pub(crate) store: Arc<Store>,
}

#[derive(Default)]
struct HealthState {
    /// Between start and stop.
    running: bool,
    /// The health service was shut down: every status is NOT_SERVING and
    /// updates are ignored (grpc-go `health.Server.Shutdown`).
    shutdown: bool,
    statuses: BTreeMap<String, ServingStatus>,
    chain_id: String,
    deps: Option<HealthDeps>,
    results: HashMap<&'static str, CheckResult>,
    ready: bool,
}

/// Runs the checks and publishes readiness (Go `healthManager`).
pub(crate) struct HealthManager {
    settings: HealthSettings,
    metrics: Arc<ServerMetrics>,
    state: Mutex<HealthState>,
}

const DEADLINE_EXCEEDED: &str = "context deadline exceeded";

/// Runs `fut` until `deadline`; the error of an expired call is Go's
/// context error text.
async fn until<T, E: std::fmt::Display>(
    deadline: tokio::time::Instant,
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T, Probe> {
    match tokio::time::timeout_at(deadline, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Probe::Failed(e.to_string())),
        Err(_) => Err(Probe::TimedOut),
    }
}

enum Probe {
    Failed(String),
    TimedOut,
}

impl Probe {
    fn text(&self) -> String {
        match self {
            Self::Failed(e) => e.clone(),
            Self::TimedOut => DEADLINE_EXCEEDED.into(),
        }
    }
}

impl HealthManager {
    /// Go `newHealthManager`: every service NOT_SERVING.
    pub(crate) fn new(settings: HealthSettings, metrics: Arc<ServerMetrics>) -> Self {
        let mut st = HealthState::default();
        st.statuses
            .insert(HEALTH_SERVICE_LIVENESS.into(), ServingStatus::NotServing);
        for s in READINESS_SERVICES {
            st.statuses.insert(s.into(), ServingStatus::NotServing);
        }
        Self {
            settings,
            metrics,
            state: Mutex::new(st),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HealthState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(st: &mut HealthState, serving: bool, services: &[&str]) {
        if st.shutdown {
            return;
        }
        let status = if serving {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };
        for s in services {
            st.statuses.insert((*s).to_owned(), status);
        }
    }

    /// Go `start`: initialization is complete; publishes liveness.
    pub(crate) fn start(&self, deps: HealthDeps, chain_id: String) {
        let mut st = self.lock();
        st.running = true;
        st.chain_id = chain_id;
        st.deps = Some(deps);
        Self::set(&mut st, true, &[HEALTH_SERVICE_LIVENESS]);
        self.metrics.set_live(!st.shutdown);
    }

    /// Go `stop`: NOT_SERVING for every service; later results are
    /// ignored.
    pub(crate) fn stop(&self) {
        let mut st = self.lock();
        st.running = false;
        st.shutdown = true;
        for v in st.statuses.values_mut() {
            *v = ServingStatus::NotServing;
        }
        self.metrics.set_live(false);
        self.metrics.set_ready(false);
    }

    /// Go `run`: a cycle now, then one every check interval, until
    /// `cancel` fires.
    pub(crate) async fn run(&self, cancel: CancellationToken) {
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = self.cycle() => {}
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(self.settings.check_interval) => {}
            }
        }
    }

    /// Go `cycle`: the checks one after another.
    pub(crate) async fn cycle(&self) {
        let Some((deps, chain_id)) = ({
            let st = self.lock();
            st.deps.clone().map(|d| (d, st.chain_id.clone()))
        }) else {
            return;
        };
        for name in CHECKS {
            let deadline = tokio::time::Instant::now() + self.settings.probe_timeout;
            let res = match name {
                "app" => self.check_app(&deps, &chain_id, deadline).await,
                "fibre_module" => Self::check_module(&deps, deadline).await,
                "signer" => Self::check_signer(&deps, deadline).await,
                "validator" => Self::check_validator(&deps, deadline).await,
                "registration" => Self::check_registration(&deps, deadline).await,
                _ => Self::check_store(&deps, deadline).await,
            };
            self.record(name, res);
        }
    }

    /// Go `record`: stores a result, logs changes and republishes
    /// readiness.
    pub(crate) fn record(&self, name: &'static str, res: CheckResult) {
        let mut st = self.lock();
        if !st.running {
            return;
        }
        let prev_reason = st.results.get(name).map_or("", |r| r.reason);
        if prev_reason != res.reason {
            if res.reason.is_empty() {
                tracing::info!(check = name, "health check passed");
            } else {
                tracing::warn!(
                    check = name,
                    reason = res.reason,
                    message = %res.message,
                    error = res.error.as_deref().unwrap_or("<nil>"),
                    "health check failed"
                );
            }
        }
        self.metrics.set_health_check(name, res.reason.is_empty());
        st.results.insert(name, res);

        let (ready, failing) = Self::evaluate_locked(&st);
        if ready == st.ready {
            return;
        }
        st.ready = ready;
        Self::set(&mut st, ready, &READINESS_SERVICES);
        self.metrics.set_ready(ready);
        if ready {
            tracing::info!("server ready");
        } else {
            tracing::warn!(failed_checks = ?failing, "server not ready");
        }
    }

    fn evaluate_locked(st: &HealthState) -> (bool, Vec<&'static str>) {
        let failing: Vec<&'static str> = CHECKS
            .iter()
            .copied()
            .filter(|c| st.results.get(c).is_none_or(|r| !r.reason.is_empty()))
            .collect();
        (st.running && failing.is_empty(), failing)
    }

    /// Go `evaluate`: ready, and the failing checks.
    pub(crate) fn evaluate(&self) -> (bool, Vec<&'static str>) {
        Self::evaluate_locked(&self.lock())
    }

    /// The last result of a check.
    pub(crate) fn result(&self, name: &str) -> Option<CheckResult> {
        self.lock().results.get(name).cloned()
    }

    /// `grpc.health.v1.Health/Check`.
    pub(crate) fn check(&self, service: &str) -> Result<ServingStatus, Status> {
        self.lock()
            .statuses
            .get(service)
            .copied()
            .ok_or_else(|| Status::not_found("unknown service"))
    }

    /// `grpc.health.v1.Health/List`.
    pub(crate) fn list(&self) -> BTreeMap<String, ServingStatus> {
        self.lock().statuses.clone()
    }

    async fn check_app(
        &self,
        deps: &HealthDeps,
        chain_id: &str,
        deadline: tokio::time::Instant,
    ) -> CheckResult {
        let st = match until(deadline, deps.state.node_status()).await {
            Ok(st) if !st.chain_id.is_empty() && st.height != 0 && st.block_time.is_some() => st,
            res => {
                return CheckResult::failed(
                    reason::APP_UNREACHABLE,
                    "The node did not report a valid status (no committed block yet, or its block store failed). Check the node log.",
                    res.err().map(|e| e.text()),
                );
            }
        };
        if st.chain_id != chain_id {
            return CheckResult::failed(
                reason::CHAIN_MISMATCH,
                format!(
                    "The node reports chain ID {}, expected {}.",
                    gostr::quote(&st.chain_id),
                    gostr::quote(chain_id)
                ),
                None,
            );
        }
        if st.catching_up {
            return CheckResult::failed(reason::APP_SYNCING, "The node is still syncing.", None);
        }
        let block_time = st.block_time.unwrap_or(UNIX_EPOCH);
        let age = SystemTime::now()
            .duration_since(block_time)
            .unwrap_or(Duration::ZERO);
        if age > self.settings.max_block_age {
            return CheckResult::failed(
                reason::CHAIN_STALLED,
                format!(
                    "The latest block is {} old (limit {}).",
                    crate::godur::format(Duration::from_secs(age.as_secs())),
                    crate::godur::format(self.settings.max_block_age)
                ),
                None,
            );
        }
        CheckResult::passed()
    }

    async fn check_module(deps: &HealthDeps, deadline: tokio::time::Instant) -> CheckResult {
        match tokio::time::timeout_at(deadline, deps.state.full_stake_storage_budget()).await {
            Ok(Ok(_)) => CheckResult::passed(),
            Ok(Err(StateError::ModuleUnavailable(e))) => CheckResult::failed(
                reason::MODULE_UNAVAILABLE,
                "The app has no Fibre module. Fibre needs app version 10 or later: wait for the chain to upgrade.",
                Some(e),
            ),
            res => CheckResult::failed(
                reason::APP_UNREACHABLE,
                "The app did not answer the Fibre module query.",
                Some(match res {
                    Ok(Err(e)) => e.to_string(),
                    _ => DEADLINE_EXCEEDED.into(),
                }),
            ),
        }
    }

    /// Go `checkSigner`: the signer answers within the probe timeout and
    /// still holds the start key (it does not prove that signing works).
    async fn check_signer(deps: &HealthDeps, deadline: tokio::time::Instant) -> CheckResult {
        match until(deadline, deps.signer.public_key()).await {
            Err(Probe::TimedOut) => CheckResult::failed(
                reason::SIGNER_UNREACHABLE,
                "The validator signer did not answer within the probe timeout. With a remote signer, check it is connected to priv_validator_laddr.",
                Some(DEADLINE_EXCEEDED.into()),
            ),
            Err(Probe::Failed(e)) => CheckResult::failed(
                reason::SIGNER_UNREACHABLE,
                "The validator signer did not return a public key. With a remote signer, check it is connected to priv_validator_laddr.",
                Some(e),
            ),
            Ok(pk) if pk != deps.pub_key => CheckResult::failed(
                reason::SIGNER_KEY_CHANGED,
                "The validator signer reports a different public key than at startup. Confirm the validator key and restart the node.",
                None,
            ),
            Ok(_) => CheckResult::passed(),
        }
    }

    async fn check_validator(deps: &HealthDeps, deadline: tokio::time::Instant) -> CheckResult {
        let set = match until(deadline, deps.state.head_validator_set()).await {
            Ok(set) => set,
            Err(e) => {
                return CheckResult::failed(
                    reason::APP_UNREACHABLE,
                    "Could not load the validator set from the node's state.",
                    Some(e.text()),
                );
            }
        };
        let addr = crypto::ed25519::address(&deps.pub_key);
        match set.get_by_address(&addr) {
            Some((_, v)) if v.voting_power > 0 => CheckResult::passed(),
            _ => CheckResult::failed(
                reason::VALIDATOR_INACTIVE,
                format!(
                    "Validator {} is not in the active set.",
                    cons_address(&deps.pub_key)
                ),
                None,
            ),
        }
    }

    async fn check_registration(deps: &HealthDeps, deadline: tokio::time::Instant) -> CheckResult {
        let addr = crypto::ed25519::address(&deps.pub_key);
        match until(deadline, deps.state.provider_registration(&addr)).await {
            Err(e) => CheckResult::failed(
                reason::APP_UNREACHABLE,
                "Could not query the Fibre provider registration from the app.",
                Some(e.text()),
            ),
            Ok(reg) if !reg.found => CheckResult::failed(
                reason::NOT_REGISTERED,
                format!(
                    "Register a Fibre provider host for validator {} with MsgSetFibreProviderInfo (celestia-rs tx valaddr set-host <host:port>).",
                    cons_address(&deps.pub_key)
                ),
                None,
            ),
            Ok(reg) if !valid_host(&reg.host) => CheckResult::failed(
                reason::HOST_INVALID,
                format!(
                    "The registered provider host {} is not a valid host:port. Register it again.",
                    gostr::quote(&reg.host)
                ),
                None,
            ),
            Ok(_) => CheckResult::passed(),
        }
    }

    async fn check_store(deps: &HealthDeps, deadline: tokio::time::Instant) -> CheckResult {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        let store = Arc::clone(&deps.store);
        let probe = async move {
            tokio::task::spawn_blocking(move || store.probe(&nanos.to_be_bytes()))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
        };
        match until(deadline, probe).await {
            Ok(()) => CheckResult::passed(),
            Err(e) => CheckResult::failed(
                reason::STORE_FAILED,
                "The store failed a small write and read. Check free disk space and permissions of the store directory.",
                Some(e.text()),
            ),
        }
    }
}

/// Go `consAddress`: the bech32 `celestiavalcons` address of a key.
pub(crate) fn cons_address(pub_key: &[u8; 32]) -> String {
    sdk::address::to_bech32(
        sdk::address::VALCONS_PREFIX,
        &crypto::ed25519::address(pub_key),
    )
}

/// v11 `valaddr` `ValidateHost`: non-empty, at most 100 bytes, Go
/// `net.SplitHostPort` form with a non-empty host and a port in
/// `[1, 65535]`.
pub(crate) fn valid_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 100 {
        return false;
    }
    let Some((h, port)) = split_host_port(host) else {
        return false;
    };
    if h.is_empty() {
        return false;
    }
    atoi(port).is_some_and(|p| (1..=65_535).contains(&p))
}

/// Go `net.SplitHostPort`.
fn split_host_port(hostport: &str) -> Option<(&str, &str)> {
    let i = hostport.rfind(':')?;
    let (host, j, k) = if hostport.starts_with('[') {
        let end = hostport.find(']')?;
        if end + 1 != i {
            return None;
        }
        (&hostport[1..end], 1, end + 1)
    } else {
        let host = &hostport[..i];
        if host.contains(':') {
            return None;
        }
        (host, 0, 0)
    };
    if hostport[j..].contains('[') || hostport[k..].contains(']') {
        return None;
    }
    Some((host, &hostport[i + 1..]))
}

/// Go `strconv.Atoi` for the values a port can take.
fn atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: i64 = digits.parse().ok()?;
    Some(if s.starts_with('-') { -v } else { v })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::occupancy::Occupancy;
    use crate::state::{
        BoxFuture, NodeStatus, PaymentPromise, ProviderRegistration, SignerError, Validator,
        ValidatorSet, VerifiedPromise,
    };
    use crate::store::StoreConfig;
    use std::time::Instant;

    struct FakeState {
        status: Result<NodeStatus, StateError>,
        set: Result<ValidatorSet, StateError>,
        reg: ProviderRegistration,
        module: Result<i64, StateError>,
    }

    struct Fake {
        st: Mutex<FakeState>,
        key: Mutex<[u8; 32]>,
        signer_err: Mutex<Option<String>>,
        signer_block: Mutex<bool>,
    }

    impl ChainState for Fake {
        fn chain_id(&self) -> String {
            "test-chain".into()
        }
        fn validator_set(&self, _h: u64) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
            let r = self.st.lock().unwrap().set.clone();
            Box::pin(async move { r })
        }
        fn head_validator_set(&self) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
            let r = self.st.lock().unwrap().set.clone();
            Box::pin(async move { r })
        }
        fn verify_promise<'a>(
            &'a self,
            _p: &'a PaymentPromise,
        ) -> BoxFuture<'a, Result<VerifiedPromise, StateError>> {
            Box::pin(async { Err(StateError::other("unused")) })
        }
        fn full_stake_storage_budget(&self) -> BoxFuture<'_, Result<i64, StateError>> {
            let r = self.st.lock().unwrap().module.clone();
            Box::pin(async move { r })
        }
        fn node_status(&self) -> BoxFuture<'_, Result<NodeStatus, StateError>> {
            let r = self.st.lock().unwrap().status.clone();
            Box::pin(async move { r })
        }
        fn provider_registration<'a>(
            &'a self,
            _a: &'a [u8; 20],
        ) -> BoxFuture<'a, Result<ProviderRegistration, StateError>> {
            let r = self.st.lock().unwrap().reg.clone();
            Box::pin(async move { Ok(r) })
        }
    }

    impl Signer for Fake {
        fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>> {
            let key = *self.key.lock().unwrap();
            let err = self.signer_err.lock().unwrap().clone();
            let block = *self.signer_block.lock().unwrap();
            Box::pin(async move {
                if block {
                    std::future::pending::<()>().await;
                }
                err.map_or(Ok(key), |e| Err(SignerError(e)))
            })
        }
        fn sign_raw_bytes<'a>(
            &'a self,
            _c: &'a str,
            _u: &'a str,
            _r: &'a [u8],
        ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>> {
            Box::pin(async { Err(SignerError("unused".into())) })
        }
    }

    const KEY: [u8; 32] = [7; 32];

    fn healthy() -> FakeState {
        FakeState {
            status: Ok(NodeStatus {
                chain_id: "test-chain".into(),
                height: 42,
                block_time: Some(SystemTime::now()),
                catching_up: false,
            }),
            set: Ok(ValidatorSet {
                validators: vec![Validator::new(KEY, 10)],
                height: 42,
            }),
            reg: ProviderRegistration {
                found: true,
                host: "127.0.0.1:7980".into(),
            },
            module: Ok(0),
        }
    }

    fn setup(probe: Duration) -> (Arc<Fake>, HealthManager, tempfile::TempDir) {
        let fake = Arc::new(Fake {
            st: Mutex::new(healthy()),
            key: Mutex::new(KEY),
            signer_err: Mutex::new(None),
            signer_block: Mutex::new(false),
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&StoreConfig::new(dir.path())).unwrap());
        let metrics = Arc::new(ServerMetrics::new(Arc::new(Occupancy::new(0))));
        let m = HealthManager::new(
            HealthSettings {
                check_interval: Duration::from_secs(10),
                probe_timeout: probe,
                max_block_age: Duration::from_secs(60),
            },
            metrics,
        );
        m.start(
            HealthDeps {
                state: fake.clone(),
                signer: fake.clone(),
                pub_key: KEY,
                store,
            },
            "test-chain".into(),
        );
        (fake, m, dir)
    }

    type Mutate = Box<dyn Fn(&Fake)>;

    // Go TestHealthManager.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn every_failure_reason_and_recovery() {
        let (fake, m, _dir) = setup(Duration::from_secs(30));
        let ready = |m: &HealthManager| m.check("").unwrap();
        assert_eq!(
            m.check(HEALTH_SERVICE_LIVENESS).unwrap(),
            ServingStatus::Serving
        );
        assert_eq!(ready(&m), ServingStatus::NotServing);
        m.cycle().await;
        assert_eq!(ready(&m), ServingStatus::Serving);
        assert_eq!(
            m.check("celestia.fibre.v1.Fibre").unwrap(),
            ServingStatus::Serving
        );
        assert_eq!(m.check("nope").unwrap_err().code(), tonic::Code::NotFound);
        assert_eq!(m.check("nope").unwrap_err().message(), "unknown service");

        let cases: Vec<(&str, &str, Mutate)> = vec![
            (
                "app",
                reason::APP_UNREACHABLE,
                Box::new(|f| f.st.lock().unwrap().status = Err(StateError::other("down"))),
            ),
            (
                "app",
                reason::CHAIN_MISMATCH,
                Box::new(|f| {
                    f.st.lock().unwrap().status.as_mut().unwrap().chain_id = "other".into();
                }),
            ),
            (
                "app",
                reason::APP_SYNCING,
                Box::new(|f| f.st.lock().unwrap().status.as_mut().unwrap().catching_up = true),
            ),
            (
                "app",
                reason::CHAIN_STALLED,
                Box::new(|f| {
                    f.st.lock().unwrap().status.as_mut().unwrap().block_time =
                        Some(SystemTime::now() - Duration::from_secs(3600));
                }),
            ),
            (
                "fibre_module",
                reason::MODULE_UNAVAILABLE,
                Box::new(|f| {
                    f.st.lock().unwrap().module =
                        Err(StateError::ModuleUnavailable("unknown service".into()));
                }),
            ),
            (
                "fibre_module",
                reason::APP_UNREACHABLE,
                Box::new(|f| f.st.lock().unwrap().module = Err(StateError::other("timeout"))),
            ),
            (
                "signer",
                reason::SIGNER_UNREACHABLE,
                Box::new(|f| *f.signer_err.lock().unwrap() = Some("down".into())),
            ),
            (
                "signer",
                reason::SIGNER_KEY_CHANGED,
                Box::new(|f| *f.key.lock().unwrap() = [8; 32]),
            ),
            (
                "validator",
                reason::APP_UNREACHABLE,
                Box::new(|f| f.st.lock().unwrap().set = Err(StateError::other("down"))),
            ),
            (
                "validator",
                reason::VALIDATOR_INACTIVE,
                Box::new(|f| {
                    f.st.lock().unwrap().set = Ok(ValidatorSet {
                        validators: vec![],
                        height: 42,
                    });
                }),
            ),
            (
                "registration",
                reason::NOT_REGISTERED,
                Box::new(|f| f.st.lock().unwrap().reg.found = false),
            ),
            (
                "registration",
                reason::HOST_INVALID,
                Box::new(|f| f.st.lock().unwrap().reg.host = "no-port".into()),
            ),
        ];
        for (check, why, mutate) in cases {
            mutate(&fake);
            m.cycle().await;
            let (is_ready, failing) = m.evaluate();
            assert!(!is_ready, "{why}");
            assert_eq!(failing, vec![check], "{why}");
            assert_eq!(m.result(check).unwrap().reason, why);
            assert_eq!(ready(&m), ServingStatus::NotServing, "{why}");
            *fake.st.lock().unwrap() = healthy();
            *fake.key.lock().unwrap() = KEY;
            *fake.signer_err.lock().unwrap() = None;
            m.cycle().await;
            assert_eq!(ready(&m), ServingStatus::Serving, "recovery after {why}");
        }
        assert_eq!(
            m.result("app").unwrap(),
            CheckResult::passed(),
            "the last result is kept"
        );

        // Stop publishes not-ready and ignores later results.
        m.stop();
        m.record("app", CheckResult::passed());
        assert_eq!(ready(&m), ServingStatus::NotServing);
        assert_eq!(
            m.check(HEALTH_SERVICE_LIVENESS).unwrap(),
            ServingStatus::NotServing
        );
    }

    #[tokio::test]
    async fn messages_match_go() {
        let (fake, m, _dir) = setup(Duration::from_secs(30));
        fake.st.lock().unwrap().status.as_mut().unwrap().block_time =
            Some(SystemTime::now() - Duration::from_secs(3600));
        m.cycle().await;
        let msg = m.result("app").unwrap().message;
        assert!(
            msg == "The latest block is 1h0m0s old (limit 1m0s)."
                || msg == "The latest block is 1h0m1s old (limit 1m0s).",
            "{msg}"
        );
        *fake.st.lock().unwrap() = healthy();
        fake.st.lock().unwrap().reg.found = false;
        m.cycle().await;
        let msg = m.result("registration").unwrap().message;
        assert!(
            msg.starts_with("Register a Fibre provider host for validator celestiavalcons1"),
            "{msg}"
        );
        fake.st.lock().unwrap().status.as_mut().unwrap().chain_id = "other".into();
        m.cycle().await;
        assert_eq!(
            m.result("app").unwrap().message,
            "The node reports chain ID \"other\", expected \"test-chain\"."
        );
    }

    // Go TestCheckSignerTimeout.
    #[tokio::test]
    async fn signer_timeout_and_cancelled_loop() {
        let (fake, m, _dir) = setup(Duration::from_millis(20));
        *fake.signer_block.lock().unwrap() = true;
        let start = Instant::now();
        m.cycle().await;
        assert_eq!(
            m.result("signer").unwrap().reason,
            reason::SIGNER_UNREACHABLE
        );
        assert!(start.elapsed() < Duration::from_secs(1));

        let cancel = CancellationToken::new();
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), m.run(cancel))
            .await
            .expect("the loop returns on cancellation");
    }

    #[test]
    fn host_validation() {
        for ok in ["127.0.0.1:7980", "example.com:1", "[::1]:65535", "a:+80"] {
            assert!(valid_host(ok), "{ok}");
        }
        for bad in [
            "",
            "no-port",
            ":80",
            "h:0",
            "h:65536",
            "h:x",
            "h:",
            "::1:80",
            "[::1]80",
            "a]:80",
            &"a".repeat(101),
        ] {
            assert!(!valid_host(bad), "{bad}");
        }
    }
}
