//! cosmos-sdk v0.46 block execution for celestia-app v1 to v3, as
//! celestia-app v3.13.0 runs it behind the multiplexer
//! (`multiplexer/abci/remote_v1.go`): InitChain without a commit, then per
//! block BeginBlock (full header), DeliverTx for each transaction, EndBlock
//! and Commit.
//!
//! The driver owns only baseapp behavior. Module behavior plugs in through
//! [`Modules`]: InitGenesis, begin and end blockers, and the ante handler
//! with message execution. Each driver call runs inside an
//! [`sdk::version::SdkV046Scope`] at the block's app version: the only
//! place shared modules take their v0.46 paths (`Context::is_sdk_v046`),
//! and the thread's previous state is restored when the call returns.
//! [`crate::App`] routes InitChain, FinalizeBlock and Commit here for a
//! cosmos-sdk v0.46 state ([`app`]); only releases that run as themselves
//! execute ([`executes`]: app version 1). App versions 2 and 3 still run
//! as v9 elsewhere and stop with an error on a v0.46 state; app versions
//! 4 and later never come here.
//!
//! Reference: cosmos-sdk `v1.31.2-sdk-v0.46.16` `baseapp/abci.go` and
//! `baseapp/baseapp.go`, celestia-app v3.13.0 `app/app.go`.

mod app;
pub(crate) use app::{AppDriver, AppModules, v1_modules};
pub mod authz;
pub mod blockers;
pub mod consensus_params;
pub mod genesis;
pub mod ibc;
pub mod ibc_channel;
pub mod ibc_transfer;
pub mod msgs;
pub mod params;
pub mod tx;

use celestia_proto::celestia_core_proto::abci::{
    self, ExecTxResult, ResponseFinalizeBlock, ResponseInitChain, TimeoutInfo, ValidatorUpdate,
};
use celestia_proto::celestia_core_proto::tendermint::types::Header as TmHeader;
use celestia_proto::celestia_core_proto::tendermint::version::Consensus;
use celestia_proto::cosmos::base::abci::v1beta1::TxMsgData;
use kvstore::GasMeter;
use multistore::CommitId;
use prost::Message as _;
use sdk::version::{Release, SdkLine};
use sdk::{Context, ExecMode, Header, MultiStore, Result, SdkError};

std::thread_local! {
    /// The first not-ported behavior a transaction reached
    /// ([`not_ported`]), reset before each transaction.
    static NOT_PORTED: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Records that the executing call reached a cosmos-sdk v0.46 behavior
/// that is not ported, and returns `e` (its error) unchanged. The driver
/// aborts the block (or InitChain) instead of committing a result: Go
/// would have run the behavior, so any result here (and the fee and
/// sequence the ante handler already took) would be wrong state. The
/// record survives callers that swallow the error (e.g. IBC
/// acknowledgements). It is the only signal: error text alone never
/// aborts, since it can echo transaction input.
pub(crate) fn not_ported(e: SdkError) -> SdkError {
    NOT_PORTED.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = Some(e.log.clone());
        }
    });
    e
}

/// [`not_ported`] for an error a shared crate reports (e.g. the ante
/// handler's unported keepers, [`tx::ante_deps`]).
pub(crate) fn record_not_ported(e: &SdkError) {
    let _ = not_ported(e.clone());
}

fn take_not_ported() -> Option<String> {
    NOT_PORTED.with(|c| c.borrow_mut().take())
}

/// Runs `f` on a store branch of the block stack: kept when `f`
/// succeeds, discarded with everything it branched when it fails, so an
/// aborted call leaves no writes behind.
fn on_branch<T>(ms: &mut MultiStore, f: impl FnOnce(&mut MultiStore) -> Result<T>) -> Result<T> {
    ms.select(sdk::store::Stack::Finalize);
    let base = ms.depth();
    ms.branch();
    let r = f(ms);
    ms.select(sdk::store::Stack::Finalize);
    if r.is_ok() {
        while ms.depth() > base {
            ms.write_branch();
        }
    } else {
        while ms.depth() > base {
            ms.discard_branch();
        }
    }
    r
}

/// The message type URLs of a raw block transaction (unwrapping a
/// `BlobTx`), for diagnostics; empty when it does not decode.
#[must_use]
pub fn tx_type_urls(raw: &[u8]) -> Vec<String> {
    use celestia_proto::celestia_core_proto::tendermint::types::BlobTx;
    use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};
    let inner = match BlobTx::decode(raw) {
        Ok(b) if b.type_id == "BLOB" => b.tx,
        _ => raw.to_vec(),
    };
    TxRaw::decode(inner.as_slice())
        .and_then(|t| TxBody::decode(t.body_bytes.as_slice()))
        .map(|b| b.messages.into_iter().map(|m| m.type_url).collect())
        .unwrap_or_default()
}

/// Tendermint v0.34 block protocol version (`version.BlockProtocol`), as
/// the multiplexer sets it in the BeginBlock header.
const BLOCK_PROTOCOL: u64 = 11;

/// Whether app version `v` executes on this driver: its release is on
/// cosmos-sdk v0.46 and runs as itself (app version 1).
#[must_use]
pub fn executes(v: u64) -> bool {
    Release::executing(v).sdk_line == SdkLine::V046
}

/// Whether app version `v` is a cosmos-sdk v0.46 release on mainnet (1 to
/// 3), executable here or not.
#[must_use]
pub fn is_v046(v: u64) -> bool {
    Release::for_app_version(v).is_some_and(|r| r.sdk_line == SdkLine::V046)
}

/// The cosmos-sdk v0.46 release whose commit info `info` is: its IAVL
/// stores and memory store entries, exactly. `None` for any other store
/// set (a v0.50 state).
#[must_use]
pub fn release_of(info: &multistore::CommitInfo) -> Option<&'static Release> {
    let mut names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    (1..=3)
        .filter_map(Release::for_app_version)
        .filter(|r| r.sdk_line == SdkLine::V046)
        .find(|r| {
            let mut want: Vec<&str> = r
                .stores
                .iter()
                .chain(r.commit_memory_stores)
                .copied()
                .collect();
            want.sort_unstable();
            want == names
                && info.store_infos.iter().all(|s| {
                    s.commit_id.is_none() == r.commit_memory_stores.contains(&s.name.as_str())
                })
        })
}

/// The committed app version of a v0.46 state (celestia-app `App.Info`):
/// `VersionParams` in subspace `baseapp`, or 1 while absent. 0 when the
/// stored value cannot be read.
#[must_use]
pub fn app_version(ms: &MultiStore) -> u64 {
    let key = [
        consensus_params::SUBSPACE.name.as_bytes(),
        b"/",
        consensus_params::VERSION.as_bytes(),
    ]
    .concat();
    let stored = ms
        .root()
        .get_store(x_params::STORE_KEY)
        .ok()
        .map(|s| s.get(&key));
    match stored {
        Some(Ok(Some(raw))) => consensus_params::decode_app_version(&raw).unwrap_or(0),
        Some(Ok(None)) | None => 1,
        Some(Err(_)) => 0,
    }
}

/// What a module set does in each ABCI phase of a v0.46 block. Each later
/// milestone fills one method: InitGenesis of the v1 modules,
/// begin/end blockers (incl. `qgb`), and the v1 ante chain with message
/// routing.
pub trait Modules {
    /// InitChainer: module InitGenesis in v1 order from the genesis app
    /// state (also the upgrade module's version map). Returns the genesis
    /// validator set.
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        app_state: &[u8],
    ) -> Result<Vec<ValidatorUpdate>>;
    /// BeginBlocker, with the full v0.34 header (staking stores it in
    /// `HistoricalInfo`).
    fn begin_block(&mut self, ctx: &mut Context<'_>, header: &TmHeader) -> Result<()>;
    /// DeliverTx: decode, ante handler and messages (`runTx`), on the
    /// block's gas meter (`ctx.BlockGasMeter()`). The driver shapes the
    /// result ([`deliver_tx_result`]).
    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        block_gas: &mut GasMeter,
        tx: &[u8],
    ) -> TxOutcome;
    /// EndBlocker: module end blockers and celestia-app's app-version
    /// upgrade logic. Returns validator updates.
    fn end_block(&mut self, ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>>;
}

impl<M: Modules + ?Sized> Modules for Box<M> {
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        app_state: &[u8],
    ) -> Result<Vec<ValidatorUpdate>> {
        (**self).init_genesis(ctx, app_state)
    }

    fn begin_block(&mut self, ctx: &mut Context<'_>, header: &TmHeader) -> Result<()> {
        (**self).begin_block(ctx, header)
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        block_gas: &mut GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        (**self).deliver_tx(ctx, block_gas, tx)
    }

    fn end_block(&mut self, ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        (**self).end_block(ctx)
    }
}

/// The empty module set: every phase is a no-op and every transaction
/// fails (no ante handler, no router).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoModules;

impl Modules for NoModules {
    fn init_genesis(&mut self, _: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }

    fn begin_block(&mut self, _: &mut Context<'_>, _: &TmHeader) -> Result<()> {
        Ok(())
    }

    fn deliver_tx(&mut self, _: &mut Context<'_>, _: &mut GasMeter, _: &[u8]) -> TxOutcome {
        TxOutcome {
            gas_wanted: 0,
            gas_used: 0,
            events: Vec::new(),
            result: Err(sdk::error::sdk::not_supported(
                "no v0.46 transaction handler",
            )),
        }
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

/// The app version 1 module set: InitGenesis, the begin and end
/// blockers, and transactions through the v0.46 `runTx` ([`tx`]) with
/// the v1 ante handler and the ported message servers ([`msgs`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct V1Modules;

impl Modules for V1Modules {
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        app_state: &[u8],
    ) -> Result<Vec<ValidatorUpdate>> {
        genesis::init_chainer(ctx, app_state)
    }

    fn begin_block(&mut self, ctx: &mut Context<'_>, header: &TmHeader) -> Result<()> {
        blockers::begin_block(ctx, header)
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        block_gas: &mut GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        tx::deliver_tx(ctx, block_gas, tx)
    }

    fn end_block(&mut self, ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        blockers::end_block(ctx)
    }
}

/// `BaseApp.InitChain`'s check of the genesis validators: when the request
/// lists validators, InitGenesis must return the same set (compared after
/// sorting each by public key).
pub(crate) fn check_genesis_validators(
    requested: &[ValidatorUpdate],
    returned: &[ValidatorUpdate],
) -> Result<()> {
    if requested.is_empty() {
        return Ok(());
    }
    if requested.len() != returned.len() {
        return Err(sdk::error::sdk::logic(format!(
            "len(RequestInitChain.Validators) != len(GenesisValidators) ({} != {})",
            requested.len(),
            returned.len()
        )));
    }
    let key = |v: &ValidatorUpdate| v.pub_key.as_ref().map(prost::Message::encode_to_vec);
    let mut a: Vec<&ValidatorUpdate> = requested.iter().collect();
    let mut b: Vec<&ValidatorUpdate> = returned.iter().collect();
    a.sort_by_key(|v| key(v));
    b.sort_by_key(|v| key(v));
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        if x != y {
            return Err(sdk::error::sdk::logic(format!(
                "genesisValidators[{i}] != req.Validators[{i}]"
            )));
        }
    }
    Ok(())
}

/// What `runTx` reports for one transaction.
#[derive(Debug)]
pub struct TxOutcome {
    /// Gas limit of the transaction.
    pub gas_wanted: u64,
    /// Gas consumed.
    pub gas_used: u64,
    /// Events: ante and message events on success, ante events on failure.
    pub events: Vec<abci::Event>,
    /// The message responses, or the error.
    pub result: std::result::Result<TxSuccess, SdkError>,
}

/// A successful transaction.
#[derive(Debug, Default)]
pub struct TxSuccess {
    /// One response per message (`MsgResponses`).
    pub msg_responses: Vec<celestia_proto::Any>,
    /// The ABCI message log (not part of any hash).
    pub log: String,
}

#[allow(clippy::cast_possible_wrap)]
const fn go_int64(v: u64) -> i64 {
    // Go `int64(uint64)`.
    v as i64
}

/// `BaseApp.DeliverTx` response: on success the data is `TxMsgData` of
/// the message responses (`makeABCIData`); on failure the error's code
/// and codespace with no data (`ResponseDeliverTxWithEvents`). Code,
/// data, gas wanted and gas used are what the v0.34 `LastResultsHash`
/// commits to.
#[must_use]
pub fn deliver_tx_result(o: TxOutcome) -> ExecTxResult {
    let gas_wanted = go_int64(o.gas_wanted);
    let gas_used = go_int64(o.gas_used);
    match o.result {
        Ok(ok) => ExecTxResult {
            data: TxMsgData {
                msg_responses: ok.msg_responses,
                ..TxMsgData::default()
            }
            .encode_to_vec(),
            log: ok.log,
            gas_wanted,
            gas_used,
            events: o.events,
            ..ExecTxResult::default()
        },
        Err(e) => crate::tx::error_result(&e, gas_wanted, gas_used, o.events),
    }
}

/// celestia-app v3.13.0 EndBlock timeouts of `app_version` (with the
/// default `timeout_commit` flag), as the multiplexer passes them on
/// (`timeoutInfoV1ToV2`: propose and commit only).
pub(crate) fn timeouts(app_version: u64) -> TimeoutInfo {
    let d = |seconds, nanos| Some(pbjson_types::Duration { seconds, nanos });
    let (propose, commit) = if app_version <= 2 {
        (d(10, 0), d(11, 0))
    } else {
        (d(3, 500_000_000), d(4, 200_000_000))
    };
    TimeoutInfo {
        timeout_propose: propose,
        timeout_commit: commit,
        ..TimeoutInfo::default()
    }
}

/// The v0.34 BeginBlock header the multiplexer builds from FinalizeBlock
/// (`remote_v1.go`): the block's header fields with protocol version
/// `{11, app_version}` and celestia-rs's chain ID.
fn begin_block_header(
    chain_id: &str,
    app_version: u64,
    req: &abci::RequestFinalizeBlock,
    h: &TmHeader,
) -> TmHeader {
    TmHeader {
        version: Some(Consensus {
            block: BLOCK_PROTOCOL,
            app: app_version,
        }),
        chain_id: chain_id.to_owned(),
        height: req.height,
        time: req.time,
        last_block_id: h.last_block_id.clone(),
        last_commit_hash: h.last_commit_hash.clone(),
        data_hash: h.data_hash.clone(),
        validators_hash: h.validators_hash.clone(),
        next_validators_hash: req.next_validators_hash.clone(),
        consensus_hash: h.consensus_hash.clone(),
        app_hash: h.app_hash.clone(),
        last_results_hash: h.last_results_hash.clone(),
        evidence_hash: h.evidence_hash.clone(),
        proposer_address: req.proposer_address.clone(),
    }
}

/// The v0.46 baseapp over a [`MultiStore`] mounted with a v0.46 release's
/// stores ([`crate::stores::mount_release`]).
pub struct Driver<M> {
    modules: M,
    chain_id: String,
    /// `BaseApp.appVersion` (0 until InitChain or the first block reads it).
    app_version: u64,
    /// Header of the block finalized and not yet committed.
    finalized: Option<Header>,
}

impl<M> std::fmt::Debug for Driver<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("chain_id", &self.chain_id)
            .field("app_version", &self.app_version)
            .field("finalized", &self.finalized.as_ref().map(|h| h.height))
            .finish_non_exhaustive()
    }
}

impl<M: Modules> Driver<M> {
    /// A driver over `modules`.
    pub fn new(modules: M) -> Self {
        Self {
            modules,
            chain_id: String::new(),
            app_version: 0,
            finalized: None,
        }
    }

    /// The chain ID a driver opened on committed state requires of every
    /// block header (empty: the first header's).
    pub fn expect_chain_id(&mut self, chain_id: &str) {
        chain_id.clone_into(&mut self.chain_id);
    }

    /// `InitChain` (celestia-app `App.InitChain` and `BaseApp.InitChain`):
    /// stores the consensus params, runs the InitChainer and returns the
    /// last commit hash, `sha256("")` on a new chain. Nothing is
    /// committed; block 1 starts from these writes.
    pub fn init_chain(
        &mut self,
        ms: &mut MultiStore,
        req: &abci::RequestInitChain,
    ) -> Result<ResponseInitChain> {
        let _ = take_not_ported();
        let r = on_branch(ms, |ms| self.init_chain_on(ms, req));
        let _ = take_not_ported();
        r
    }

    fn init_chain_on(
        &mut self,
        ms: &mut MultiStore,
        req: &abci::RequestInitChain,
    ) -> Result<ResponseInitChain> {
        if req.initial_height > 1 {
            return Err(sdk::error::sdk::not_supported("initial_height > 1"));
        }
        // `setDefaultAppVersion` panics without consensus or version params.
        let mut cp = req
            .consensus_params
            .clone()
            .ok_or_else(|| sdk::error::sdk::logic("no consensus params set"))?;
        let version = cp
            .version
            .as_mut()
            .ok_or_else(|| sdk::error::sdk::logic("no version set in consensus params"))?;
        if version.app == 0 {
            version.app = 1;
        }
        let app_version = version.app;
        let _v046 = sdk::version::SdkV046Scope::enter(app_version);
        self.chain_id.clone_from(&req.chain_id);
        let header = Header {
            chain_id: req.chain_id.clone(),
            height: 0,
            time: req.time.unwrap_or_default(),
            ..Header::default()
        };
        let mut ctx = Context::new(ms, header, ExecMode::Finalize);
        consensus_params::store(&mut ctx, &cp)?;
        self.app_version = app_version;
        ctx.consensus_params = cp;
        let validators = self.modules.init_genesis(&mut ctx, &req.app_state_bytes)?;
        if let Some(what) = take_not_ported() {
            return Err(sdk::error::sdk::not_supported(format!(
                "app version {app_version}: InitChain not executed: InitGenesis reached a behavior that is not ported: {what}"
            )));
        }
        check_genesis_validators(&req.validators, &validators)?;
        if app_version != 1 {
            consensus_params::set_initial_app_version(&mut ctx, app_version)?;
        }
        drop(ctx);
        let last = ms.root().last_commit_id().map_err(store_err)?;
        let app_hash = if last.is_zero() {
            hash::sha256(&[]).to_vec()
        } else {
            last.hash.to_vec()
        };
        Ok(ResponseInitChain {
            consensus_params: None,
            validators,
            app_hash,
            timeout_info: None,
        })
    }

    /// `FinalizeBlock` as the multiplexer splits it for ABCI v0.34:
    /// BeginBlock with the full header, DeliverTx for each transaction,
    /// EndBlock. Events are BeginBlock's, each transaction's, then
    /// EndBlock's; the consensus params are the full stored set.
    ///
    /// A block that fails (also one that reaches a behavior that is not
    /// ported, [`not_ported`]) leaves no writes behind.
    pub fn finalize_block(
        &mut self,
        ms: &mut MultiStore,
        req: &abci::RequestFinalizeBlock,
    ) -> Result<ResponseFinalizeBlock> {
        let _ = take_not_ported();
        let r = on_branch(ms, |ms| self.finalize_block_on(ms, req));
        let _ = take_not_ported();
        r
    }

    /// The error of a block phase that reached a not-ported behavior.
    fn phase_not_ported(&self, height: i64, phase: &str) -> Result<()> {
        match take_not_ported() {
            Some(what) => Err(sdk::error::sdk::not_supported(format!(
                "app version {}: block {height} not executed: {phase} reached a behavior that is not ported: {what}",
                self.app_version
            ))),
            None => Ok(()),
        }
    }

    fn finalize_block_on(
        &mut self,
        ms: &mut MultiStore,
        req: &abci::RequestFinalizeBlock,
    ) -> Result<ResponseFinalizeBlock> {
        let full = req.header.as_ref().ok_or_else(|| {
            sdk::error::sdk::logic("v0.46 BeginBlock needs the full block header")
        })?;
        let expected = ms.version().saturating_add(1);
        if req.height != expected || full.height != req.height || self.finalized.is_some() {
            return Err(sdk::error::sdk::logic(format!(
                "invalid height: {}; expected: {expected}",
                req.height
            )));
        }
        // A driver opened on committed state had no InitChain: the first
        // block header gives its chain ID. Every later header must agree.
        if self.chain_id.is_empty() {
            self.chain_id.clone_from(&full.chain_id);
        }
        if self.chain_id.is_empty() || full.chain_id != self.chain_id {
            return Err(sdk::error::sdk::logic(format!(
                "invalid chain ID in block header: {}; expected: {}",
                full.chain_id, self.chain_id
            )));
        }
        if self.app_version == 0 {
            self.app_version = app_version(ms);
        }
        let _v046 = sdk::version::SdkV046Scope::enter(self.app_version);
        let tm_header = begin_block_header(&self.chain_id, self.app_version, req, full);
        let header = Header {
            chain_id: self.chain_id.clone(),
            height: req.height,
            time: req.time.unwrap_or_default(),
            proposer_address: req.proposer_address.clone(),
            hash: req.hash.clone(),
            app_hash: full.app_hash.clone(),
            next_validators_hash: req.next_validators_hash.clone(),
        };
        let mut ctx = Context::new(ms, header.clone(), ExecMode::Finalize);
        ctx.consensus_params = consensus_params::load(&mut ctx, self.app_version)?;
        ctx.vote_infos = req
            .decided_last_commit
            .as_ref()
            .map(|c| c.votes.clone())
            .unwrap_or_default();
        ctx.misbehavior.clone_from(&req.misbehavior);
        let mut block_gas = tx::block_gas_meter(&ctx);

        self.modules.begin_block(&mut ctx, &tm_header)?;
        self.phase_not_ported(req.height, "BeginBlock")?;
        let mut events = ctx.events.take();
        let mut tx_results = Vec::with_capacity(req.txs.len());
        for (i, tx) in req.txs.iter().enumerate() {
            let _ = take_not_ported();
            let outcome = self.modules.deliver_tx(&mut ctx, &mut block_gas, tx);
            if let Some(what) = take_not_ported() {
                return Err(sdk::error::sdk::not_supported(format!(
                    "app version {}: block {} not executed: tx {i} [{}] reached a behavior that is not ported: {what}",
                    self.app_version,
                    req.height,
                    tx_type_urls(tx).join(", ")
                )));
            }
            let result = deliver_tx_result(outcome);
            events.extend(result.events.iter().cloned());
            tx_results.push(result);
        }
        let validator_updates = self.modules.end_block(&mut ctx)?;
        self.phase_not_ported(req.height, "EndBlock")?;
        events.extend(ctx.events.take());
        let cp = consensus_params::load(&mut ctx, self.app_version)?;
        drop(ctx);

        ms.flush()?;
        let app_hash = ms.working_hash()?.to_vec();
        self.finalized = Some(header);
        Ok(ResponseFinalizeBlock {
            events,
            tx_results,
            validator_updates,
            consensus_param_updates: Some(cp),
            app_hash,
            timeout_info: Some(timeouts(self.app_version)),
        })
    }

    /// `Commit`: commits the finalized block's writes as the next version.
    pub fn commit(&mut self, ms: &mut MultiStore) -> Result<CommitId> {
        let _v046 = sdk::version::SdkV046Scope::enter(self.app_version);
        if self.finalized.take().is_none() {
            return Err(sdk::error::sdk::logic("commit without finalized block"));
        }
        ms.flush()?;
        let id = ms.commit()?;
        ms.reset_check();
        Ok(id)
    }
}

fn store_err(e: impl std::fmt::Display) -> SdkError {
    sdk::error::sdk::logic(format!("store: {e}"))
}

#[cfg(test)]
mod tests;
