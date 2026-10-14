//! Trusted state for bootstrapping (`statesync/stateprovider.go`): app
//! hashes, commits and the consensus `State` at a snapshot height, verified
//! with the light client against celestia-core RPC servers.

use std::sync::{Arc, Mutex};

use ctypes::gofmt::hex_upper;
use ctypes::{Commit, ConsensusParams, Time};
use light::{Client, DbStore, HttpProvider, Provider, RpcClient, TrustOptions};

use crate::state::{State, TM_CORE_SEM_VER, empty_state};

/// `StateProvider`.
pub trait StateProvider: Send + Sync {
    /// The app hash after `height` was committed (from header `height+1`).
    fn app_hash(&self, height: u64) -> light::Result<Vec<u8>>;
    /// The commit for `height`.
    fn commit(&self, height: u64) -> light::Result<Commit>;
    /// The consensus state after `height`.
    fn state(&self, height: u64) -> light::Result<State>;
}

/// Something that can fetch consensus params for a height (the primary's
/// RPC; abstracted for tests).
pub trait ParamsSource: Send + Sync {
    /// `/consensus_params?height=h` → (block height, params).
    fn consensus_params(&self, height: i64) -> Result<(i64, ConsensusParams), String>;
}

impl ParamsSource for RpcClient {
    fn consensus_params(&self, height: i64) -> Result<(i64, ConsensusParams), String> {
        RpcClient::consensus_params(self, Some(height)).map_err(|e| e.to_string())
    }
}

struct Entry {
    provider: Arc<dyn Provider>,
    params: Arc<dyn ParamsSource>,
}

/// `lightClientStateProvider`.
pub struct LightClientStateProvider {
    lc: Mutex<Client>,
    version: ctypes::Consensus,
    initial_height: i64,
    providers: Vec<Entry>,
    now: fn() -> Time,
}

impl std::fmt::Debug for LightClientStateProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LightClientStateProvider")
            .field("initial_height", &self.initial_height)
            .finish_non_exhaustive()
    }
}

fn same(a: &Arc<dyn Provider>, b: &Arc<dyn Provider>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

impl LightClientStateProvider {
    /// `NewLightClientStateProvider` over RPC servers (at least two: the
    /// first is the primary, the rest are witnesses).
    pub fn new(
        chain_id: &str,
        version: ctypes::Consensus,
        initial_height: i64,
        servers: &[String],
        trust_options: &TrustOptions,
    ) -> light::Result<Self> {
        if servers.len() < 2 {
            return Err(light::Error::msg(format!(
                "at least 2 RPC servers are required, got {}",
                servers.len()
            )));
        }
        let entries: Vec<Entry> = servers
            .iter()
            .map(|s| {
                let p = HttpProvider::new(chain_id, s);
                let rpc = RpcClient::new(s, std::time::Duration::from_secs(10));
                Entry {
                    provider: Arc::new(p),
                    params: Arc::new(rpc),
                }
            })
            .collect();
        Self::with_providers(
            chain_id,
            version,
            initial_height,
            entries_to_pairs(entries),
            trust_options,
            light::Options {
                max_retry_attempts: 5,
                ..light::Options::default()
            },
        )
    }

    /// A state provider over arbitrary providers (each paired with the
    /// source used for consensus params when it is the primary).
    pub fn with_providers(
        chain_id: &str,
        version: ctypes::Consensus,
        initial_height: i64,
        providers: Vec<(Arc<dyn Provider>, Arc<dyn ParamsSource>)>,
        trust_options: &TrustOptions,
        opts: light::Options,
    ) -> light::Result<Self> {
        if providers.is_empty() {
            return Err(light::Error::NoWitnesses);
        }
        let primary = Arc::clone(&providers[0].0);
        let witnesses = providers[1..].iter().map(|(p, _)| Arc::clone(p)).collect();
        let lc = Client::new(
            chain_id,
            trust_options,
            primary,
            witnesses,
            Box::new(DbStore::in_memory()),
            opts,
        )?;
        Ok(Self {
            lc: Mutex::new(lc),
            version,
            initial_height,
            providers: providers
                .into_iter()
                .map(|(provider, params)| Entry { provider, params })
                .collect(),
            now: light::verifier::now,
        })
    }

    /// Overrides the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, now: fn() -> Time) -> Self {
        self.now = now;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Client> {
        self.lc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn entries_to_pairs(e: Vec<Entry>) -> Vec<(Arc<dyn Provider>, Arc<dyn ParamsSource>)> {
    e.into_iter().map(|e| (e.provider, e.params)).collect()
}

fn height_i64(h: u64) -> i64 {
    i64::try_from(h).unwrap_or(i64::MAX)
}

impl StateProvider for LightClientStateProvider {
    fn app_hash(&self, height: u64) -> light::Result<Vec<u8>> {
        let mut lc = self.lock();
        let h = height_i64(height);
        let header = lc.verify_light_block_at_height(h + 1, (self.now)())?;
        // also fetch H+2, needed when building the state (Go's "hack")
        lc.verify_light_block_at_height(h + 2, (self.now)())?;
        Ok(header
            .header()
            .map(|h| h.app_hash.clone())
            .unwrap_or_default())
    }

    fn commit(&self, height: u64) -> light::Result<Commit> {
        let mut lc = self.lock();
        let lb = lc.verify_light_block_at_height(height_i64(height), (self.now)())?;
        Ok(lb
            .signed_header
            .and_then(|sh| sh.commit)
            .unwrap_or_default())
    }

    fn state(&self, height: u64) -> light::Result<State> {
        let mut lc = self.lock();
        let h = height_i64(height);
        let mut state = empty_state(
            lc.chain_id(),
            self.version,
            if self.initial_height == 0 {
                1
            } else {
                self.initial_height
            },
        );
        let last = lc.verify_light_block_at_height(h, (self.now)())?;
        let current = lc.verify_light_block_at_height(h + 1, (self.now)())?;
        let next = lc.verify_light_block_at_height(h + 2, (self.now)())?;
        let (Some(lh), Some(ch), Some(nh)) = (last.header(), current.header(), next.header())
        else {
            return Err(light::Error::msg("missing header"));
        };
        state.version = ch.version;
        TM_CORE_SEM_VER.clone_into(&mut state.software);
        state.last_block_height = lh.height;
        state.last_block_time = lh.time;
        state.last_block_id = last
            .signed_header
            .as_ref()
            .and_then(|sh| sh.commit.as_ref())
            .map(|c| c.block_id.clone())
            .unwrap_or_default();
        state.app_hash.clone_from(&ch.app_hash);
        state.last_results_hash.clone_from(&ch.last_results_hash);
        state.last_validators = last.validator_set.clone().unwrap_or_default();
        state.validators = current.validator_set.clone().unwrap_or_default();
        state.next_validators = next.validator_set.clone().unwrap_or_default();
        state.last_height_validators_changed = nh.height;

        // consensus params via the primary's RPC, verified by the light client
        let primary = lc.primary();
        let Some(entry) = self.providers.iter().find(|e| same(&e.provider, &primary)) else {
            return Err(light::Error::msg(
                "could not find address for primary light client provider",
            ));
        };
        let current_height = ch.height;
        let params =
            verified_consensus_params(&mut lc, entry.params.as_ref(), current_height, (self.now)())
                .map_err(|e| {
                    e.wrap(format!(
                        "unable to fetch consensus parameters for height {current_height}: "
                    ))
                })?;
        state.consensus_params = params;
        state.last_height_consensus_params_changed = current_height;
        Ok(state)
    }
}

/// `lightrpc.Client.ConsensusParams`: fetch params for `height` and check
/// them against the light-client-verified header's `ConsensusHash`.
pub fn verified_consensus_params(
    lc: &mut Client,
    source: &dyn ParamsSource,
    height: i64,
    now: Time,
) -> light::Result<ConsensusParams> {
    let (block_height, params) = source.consensus_params(height).map_err(light::Error::Msg)?;
    params.validate_basic()?;
    if block_height <= 0 {
        return Err(light::Error::msg("negative or zero height"));
    }
    if block_height != height {
        return Err(light::Error::msg(format!(
            "params height {block_height} does not match requested height {height}"
        )));
    }
    let l = lc
        .verify_light_block_at_height(block_height, now)
        .map_err(|e| e.wrap(format!("failed to update light client to {block_height}: ")))?;
    let trusted = l
        .header()
        .map(|h| h.consensus_hash.clone())
        .unwrap_or_default();
    let got = params.hash();
    if got != trusted {
        return Err(light::Error::msg(format!(
            "params hash {} does not match trusted hash {}",
            hex_upper(&got),
            hex_upper(&trusted)
        )));
    }
    Ok(params)
}
