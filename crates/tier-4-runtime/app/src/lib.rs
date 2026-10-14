//! celestia-app v10 state machine.
//!
//! [`App`] mirrors `baseapp.BaseApp` + `app.App`: it owns the multistore,
//! drives InitChain → (FinalizeBlock → Commit)* and dispatches modules in
//! Go's exact order lists ([`modules`]). [`AbciApp`] adapts it to the ABCI
//! socket server.

pub mod genutil;
pub mod hyperlane;
pub mod modules;
pub mod proposal;
pub mod router;
pub mod stores;
pub mod tx;
pub mod txpipe;

use std::sync::Mutex;

use abci_server::Application;
use celestia_proto::celestia_core_proto::abci::{
    self, ResponseCommit, ResponseException, ResponseFinalizeBlock, ResponseInfo,
    ResponseInitChain, TimeoutInfo, request, response,
};
use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
use pbjson_types::Duration;
use sdk::{Context, ExecMode, Header, MultiStore, Result, SdkError};

/// `baseapp.InitialAppVersion` is 0; celestia genesis files carry the app
/// version explicitly in consensus params.
pub const INITIAL_APP_VERSION: u64 = 0;

/// The application.
#[derive(Debug)]
pub struct App {
    ms: MultiStore,
    chain_id: String,
    initial_height: i64,
    /// Header of the block being finalized; `Some` between InitChain (or
    /// FinalizeBlock) and Commit.
    finalize_header: Option<Header>,
    last_commit_hash: Vec<u8>,
    last_height: i64,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn dur(seconds: i64, nanos: i32) -> Option<Duration> {
    Some(Duration { seconds, nanos })
}

/// `app.TimeoutInfo()` with default config values.
#[must_use]
pub fn timeout_info() -> TimeoutInfo {
    TimeoutInfo {
        timeout_propose: dur(3, 0),
        timeout_propose_delta: dur(0, 500_000_000),
        timeout_prevote: dur(2, 0),
        timeout_prevote_delta: dur(0, 500_000_000),
        timeout_precommit: dur(3, 0),
        timeout_precommit_delta: dur(0, 500_000_000),
        timeout_commit: dur(0, 500_000_000),
        delayed_precommit_timeout: dur(2, 100_000_000),
    }
}

impl App {
    /// Creates an app with all v10 stores mounted.
    ///
    /// # Panics
    /// Never: store names are static and unique.
    #[must_use]
    pub fn new() -> Self {
        let mut ms = MultiStore::new();
        for (name, kind) in stores::all() {
            #[allow(clippy::expect_used)]
            ms.mount(name, kind).expect("static store list is valid");
        }
        modules::install_staking_hooks();
        Self {
            ms,
            chain_id: String::new(),
            initial_height: 1,
            finalize_header: None,
            last_commit_hash: sdk_empty_hash(),
            last_height: 0,
        }
    }

    /// Read access to the multistore (tests, diagnostics).
    #[must_use]
    pub fn multistore(&self) -> &MultiStore {
        &self.ms
    }

    /// `Info`.
    #[must_use]
    pub fn info(&self) -> ResponseInfo {
        ResponseInfo {
            data: "celestia-app".into(),
            version: String::new(),
            app_version: self.app_version(),
            last_block_height: self.last_height,
            last_block_app_hash: self.last_commit_hash.clone(),
            timeout_info: Some(timeout_info()),
        }
    }

    fn app_version(&self) -> u64 {
        if self.last_height == 0 {
            return INITIAL_APP_VERSION;
        }
        let mut ms_view = self
            .ms
            .root()
            .get_store(x_consensus::STORE_KEY)
            .ok()
            .and_then(|s| s.get(x_consensus::PARAMS_KEY).ok().flatten());
        ms_view
            .take()
            .and_then(|b| <ConsensusParams as prost::Message>::decode(b.as_slice()).ok())
            .and_then(|cp| cp.version)
            .map_or(INITIAL_APP_VERSION, |v| v.app)
    }

    /// `InitChain`.
    pub fn init_chain(&mut self, req: abci::RequestInitChain) -> Result<ResponseInitChain> {
        if req.initial_height > 1 {
            return Err(sdk::error::sdk::not_supported("initial_height > 1"));
        }
        self.chain_id.clone_from(&req.chain_id);
        self.initial_height = req.initial_height.max(1);
        let header = Header {
            chain_id: req.chain_id.clone(),
            height: 0,
            time: req.time.unwrap_or_default(),
            ..Header::default()
        };
        let mut ctx = Context::new(&mut self.ms, header.clone(), ExecMode::Finalize);

        if let Some(mut cp) = req.consensus_params.clone() {
            if cp.version.as_ref().is_none_or(|v| v.app == 0) {
                cp.version = Some(VersionParams {
                    app: INITIAL_APP_VERSION,
                });
            }
            x_consensus::set_params(&mut ctx, &cp)?;
            ctx.consensus_params = cp;
        }

        // InitChainer.
        let genesis: serde_json::Value = serde_json::from_slice(&req.app_state_bytes)
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?;
        let app_state = &genesis;
        x_upgrade::set_module_version_map(&mut ctx, modules::VERSION_MAP.iter().copied())?;
        let mut validators = Vec::new();
        for module in modules::INIT_GENESIS {
            let updates = modules::init_genesis(&mut ctx, module, &app_state[*module])?;
            if !updates.is_empty() {
                if !validators.is_empty() {
                    return Err(sdk::error::sdk::logic(
                        "validator InitGenesis updates already set by a previous module",
                    ));
                }
                validators = updates;
            }
        }
        drop(ctx);

        self.finalize_header = Some(Header {
            height: self.initial_height,
            ..header
        });
        Ok(ResponseInitChain {
            consensus_params: None,
            validators,
            app_hash: self.last_commit_hash.clone(),
            timeout_info: Some(timeout_info()),
        })
    }

    /// `FinalizeBlock` (no optimistic execution).
    pub fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> Result<ResponseFinalizeBlock> {
        let header = Header {
            chain_id: self.chain_id.clone(),
            height: req.height,
            time: req.time.unwrap_or_default(),
            proposer_address: req.proposer_address.clone(),
            hash: req.hash.clone(),
            app_hash: self.last_commit_hash.clone(),
            next_validators_hash: req.next_validators_hash.clone(),
        };
        let mut ctx = Context::new(&mut self.ms, header.clone(), ExecMode::Finalize);
        ctx.consensus_params = x_consensus::get_params(&mut ctx)?.unwrap_or_default();
        ctx.vote_infos = req
            .decided_last_commit
            .as_ref()
            .map(|c| c.votes.clone())
            .unwrap_or_default();
        ctx.misbehavior.clone_from(&req.misbehavior);

        for module in modules::PRE_BLOCKERS {
            modules::pre_block(&mut ctx, module)?;
        }
        for module in modules::BEGIN_BLOCKERS {
            modules::begin_block(&mut ctx, module)?;
        }
        let mut events = ctx.events.take();

        let tx_results = execute_txs_phased(&mut ctx, &req.txs);

        let mut validator_updates = Vec::new();
        for module in modules::END_BLOCKERS {
            let updates = modules::end_block(&mut ctx, module)?;
            if !updates.is_empty() {
                validator_updates = updates;
            }
        }
        modules::app_end_block(&mut ctx)?;
        events.extend(ctx.events.take());
        let cp = x_consensus::get_params(&mut ctx)?;
        drop(ctx);

        self.ms.flush()?;
        let app_hash = self.ms.working_hash()?.to_vec();
        self.finalize_header = Some(header);
        Ok(ResponseFinalizeBlock {
            events,
            tx_results,
            validator_updates,
            consensus_param_updates: cp,
            app_hash,
            timeout_info: Some(timeout_info()),
        })
    }

    /// `Commit`.
    pub fn commit(&mut self) -> Result<ResponseCommit> {
        let header = self
            .finalize_header
            .take()
            .ok_or_else(|| sdk::error::sdk::logic("commit without finalized block"))?;
        // InitChain writes are flushed with the first block; flushing again
        // here is a no-op for already-flushed state.
        self.ms.flush()?;
        let id = self.ms.commit()?;
        // setState(execModeCheck): mempool validation restarts from the new state.
        self.ms.reset_check();
        self.last_commit_hash = id.hash.to_vec();
        self.last_height = id.version;
        let _ = header;
        Ok(ResponseCommit { retain_height: 0 })
    }
}

/// `executeTxsPhased`: every tx's ante handler first, then every surviving
/// tx's messages. Ante writes persist even when the messages fail.
fn execute_txs_phased(ctx: &mut Context<'_>, txs: &[Vec<u8>]) -> Vec<abci::ExecTxResult> {
    let mut results: Vec<Option<abci::ExecTxResult>> = vec![None; txs.len()];
    let mut prepared: Vec<Option<tx::Prepared>> = Vec::with_capacity(txs.len());
    for (i, raw) in txs.iter().enumerate() {
        if txpipe::decode_tx(raw).is_err() {
            // comet expects a result for every tx, even undecodable ones.
            results[i] = Some(tx::error_result(
                &sdk::error::sdk::tx_decode(""),
                0,
                0,
                Vec::new(),
            ));
            prepared.push(None);
            continue;
        }
        let (p, res) = tx::run_ante(ctx, raw, txpipe::decode_tx, txpipe::ante, false);
        match res {
            Ok(()) => prepared.push(Some(p)),
            Err(e) => {
                results[i] = Some(tx::exec_result(&p, Err(e)));
                prepared.push(None);
            }
        }
    }
    for (i, p) in prepared.iter_mut().enumerate() {
        if let Some(p) = p {
            let res = tx::run_msgs(ctx, p, true);
            results[i] = Some(tx::exec_result(p, res));
        }
    }
    results.into_iter().map(Option::unwrap_or_default).collect()
}

fn sdk_empty_hash() -> Vec<u8> {
    // sha256 of the empty commit info (version 0).
    vec![
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ]
}

/// Thread-safe ABCI adapter.
#[derive(Debug, Default)]
pub struct AbciApp {
    inner: Mutex<App>,
}

impl AbciApp {
    /// Wraps an app.
    #[must_use]
    pub fn new(app: App) -> Self {
        Self {
            inner: Mutex::new(app),
        }
    }
}

fn exception(e: &SdkError) -> response::Value {
    response::Value::Exception(ResponseException {
        error: e.log.clone(),
    })
}

impl Application for AbciApp {
    fn handle(&self, request: request::Value) -> response::Value {
        let Ok(mut app) = self.inner.lock() else {
            return response::Value::Exception(ResponseException {
                error: "poisoned".into(),
            });
        };
        match request {
            request::Value::Echo(e) => {
                response::Value::Echo(abci::ResponseEcho { message: e.message })
            }
            request::Value::Info(_) => response::Value::Info(app.info()),
            request::Value::InitChain(r) => app
                .init_chain(r)
                .map_or_else(|e| exception(&e), response::Value::InitChain),
            request::Value::FinalizeBlock(r) => app
                .finalize_block(r)
                .map_or_else(|e| exception(&e), response::Value::FinalizeBlock),
            request::Value::Commit(_) => app
                .commit()
                .map_or_else(|e| exception(&e), response::Value::Commit),
            other => response::Value::Exception(ResponseException {
                error: format!("unsupported request {other:?}"),
            }),
        }
    }
}
