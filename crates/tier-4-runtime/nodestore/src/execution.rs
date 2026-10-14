//! `state.BlockExecutor` (`state/execution.go`) and block validation
//! (`state/validation.go`), calling the application in process through
//! [`AppConn`].

use ctypes::proto::abci;
use ctypes::proto::abci::response_process_proposal::ProposalStatus;
use ctypes::proto::abci::response_verify_vote_extension::VerifyStatus;
use ctypes::{
    BLOCK_ID_FLAG_ABSENT, Block, BlockId, Data, ExtendedCommit, PartSet, Time, Tx, ValidatorSet,
    Vote, block::max_data_bytes,
};

use crate::block_store::BlockStore;
use crate::evidence::EvidencePool;
use crate::state::{
    State, add_duration, median_time, update_state, validate_validator_updates,
    validator_updates_from_abci,
};
use crate::state_store::StateStore;
use crate::{Error, Result, ctypes_err};

/// `maxBlockTimeTolerance`: a block may not be this far ahead of the local
/// clock (Go `time.Duration` nanoseconds).
pub const MAX_BLOCK_TIME_TOLERANCE: i64 = 60 * 1_000_000_000;

/// The ABCI consensus connection (plus Info), called in process.
pub trait AppConn: Send {
    /// `Info`.
    fn info(&mut self) -> Result<abci::ResponseInfo>;
    /// `InitChain`.
    fn init_chain(&mut self, req: abci::RequestInitChain) -> Result<abci::ResponseInitChain>;
    /// `PrepareProposal`.
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> Result<abci::ResponsePrepareProposal>;
    /// `ProcessProposal`.
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> Result<abci::ResponseProcessProposal>;
    /// `FinalizeBlock`.
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> Result<abci::ResponseFinalizeBlock>;
    /// `Commit`.
    fn commit(&mut self) -> Result<abci::ResponseCommit>;
    /// `ExtendVote` (default: no extension, like the SDK's default handler).
    fn extend_vote(&mut self, _req: abci::RequestExtendVote) -> Result<abci::ResponseExtendVote> {
        Ok(abci::ResponseExtendVote::default())
    }
    /// `VerifyVoteExtension` (default: accept).
    fn verify_vote_extension(
        &mut self,
        _req: abci::RequestVerifyVoteExtension,
    ) -> Result<abci::ResponseVerifyVoteExtension> {
        Ok(abci::ResponseVerifyVoteExtension {
            status: abci::response_verify_vote_extension::VerifyStatus::Accept.into(),
        })
    }
}

/// The mempool as block execution sees it.
pub trait Mempool: Send {
    /// `ReapMaxBytesMaxGas` (`-1` = unlimited).
    fn reap_max_bytes_max_gas(&mut self, max_bytes: i64, max_gas: i64) -> Vec<Vec<u8>>;
    /// `Update` after a commit.
    fn update(&mut self, height: i64, txs: &[Tx], results: &[abci::ExecTxResult]);
    /// Whether txs are waiting (drives `TxsAvailable`).
    fn has_txs(&self) -> bool {
        false
    }
}

/// A mempool that never has transactions.
#[derive(Debug, Default, Clone, Copy)]
pub struct NopMempool;

impl Mempool for NopMempool {
    fn reap_max_bytes_max_gas(&mut self, _: i64, _: i64) -> Vec<Vec<u8>> {
        Vec::new()
    }
    fn update(&mut self, _: i64, _: &[Tx], _: &[abci::ExecTxResult]) {}
}

/// `BlockExecutor`.
pub struct BlockExecutor {
    /// State store.
    pub state_store: StateStore,
    /// Block store.
    pub block_store: BlockStore,
    app: Box<dyn AppConn>,
    mempool: Box<dyn Mempool>,
    evpool: Box<dyn EvidencePool>,
    /// Number of states pruned so far.
    pub pruned_states: u64,
}

impl std::fmt::Debug for BlockExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockExecutor").finish_non_exhaustive()
    }
}

/// Strips celestia `BlobTx` wrappers (FinalizeBlock sees the inner tx).
fn finalize_txs(block: &Block) -> Vec<Vec<u8>> {
    block
        .data
        .txs
        .iter()
        .map(|tx| match ctypes::tx::unmarshal_blob_tx(&tx.0) {
            Some(b) => b.tx,
            None => tx.0.clone(),
        })
        .collect()
}

fn evidence_abci(block: &Block) -> Vec<abci::Misbehavior> {
    ctypes::evidence::evidence_list_to_abci(&block.evidence.evidence)
}

/// `BuildLastCommitInfo`.
pub fn build_last_commit_info(
    block: &Block,
    last_vals: &ValidatorSet,
    initial_height: i64,
) -> Result<abci::CommitInfo> {
    if block.header.height == initial_height {
        return Ok(abci::CommitInfo::default());
    }
    let lc = block
        .last_commit
        .as_ref()
        .ok_or_else(|| Error::new("nil LastCommit"))?;
    if lc.signatures.len() != last_vals.validators.len() {
        return Err(Error::new(format!(
            "commit size ({}) doesn't match validator set length ({}) at height {}",
            lc.signatures.len(),
            last_vals.validators.len(),
            block.header.height
        )));
    }
    let votes = last_vals
        .validators
        .iter()
        .zip(&lc.signatures)
        .map(|(v, cs)| abci::VoteInfo {
            validator: Some(abci::Validator {
                address: v.address.clone(),
                power: v.voting_power,
            }),
            block_id_flag: i32::from(cs.block_id_flag),
        })
        .collect();
    Ok(abci::CommitInfo {
        round: lc.round,
        votes,
    })
}

/// `BuildExtendedCommitInfo`.
pub fn build_extended_commit_info(
    ec: &ExtendedCommit,
    vals: &ValidatorSet,
    initial_height: i64,
    abci_params: &ctypes::params::AbciParams,
) -> Result<abci::ExtendedCommitInfo> {
    if ec.height < initial_height {
        return Ok(abci::ExtendedCommitInfo::default());
    }
    if ec.extended_signatures.len() != vals.validators.len() {
        return Err(Error::new(format!(
            "extended commit size ({}) does not match validator set length ({}) at height {}",
            ec.extended_signatures.len(),
            vals.validators.len(),
            ec.height
        )));
    }
    let ext_enabled = abci_params
        .vote_extensions_enabled(ec.height)
        .map_err(ctypes_err)?;
    let mut votes = Vec::with_capacity(vals.validators.len());
    for (i, v) in vals.validators.iter().enumerate() {
        let ecs = &ec.extended_signatures[i];
        if ecs.commit_sig.block_id_flag != BLOCK_ID_FLAG_ABSENT
            && ecs.commit_sig.validator_address != v.address
        {
            return Err(Error::new(format!(
                "validator address of extended commit signature in position {i} does not match the corresponding validator's at height {}",
                ec.height
            )));
        }
        ecs.ensure_extension(ext_enabled).map_err(|e| {
            Error::new(format!(
                "commit at height {} has problems with vote extension data; err {e}",
                ec.height
            ))
        })?;
        votes.push(abci::ExtendedVoteInfo {
            validator: Some(abci::Validator {
                address: v.address.clone(),
                power: v.voting_power,
            }),
            vote_extension: ecs.extension.clone(),
            extension_signature: ecs.extension_signature.clone(),
            block_id_flag: i32::from(ecs.commit_sig.block_id_flag),
        });
    }
    Ok(abci::ExtendedCommitInfo {
        round: ec.round,
        votes,
    })
}

/// `validateBlock`: the block against the state (`now` is the local clock).
pub fn validate_block(state: &State, block: &mut Block, now: Time) -> Result<()> {
    validate_block_opts(state, block, now, false)
}

fn validate_block_opts(
    state: &State,
    block: &mut Block,
    now: Time,
    skip_last_commit: bool,
) -> Result<()> {
    block.validate_basic().map_err(ctypes_err)?;
    let h = &block.header;
    if h.version != state.version {
        return Err(Error::new(format!(
            "wrong Block.Header.Version. Expected {:?}, got {:?}",
            state.version, h.version
        )));
    }
    if h.chain_id != state.chain_id {
        return Err(Error::new(format!(
            "wrong Block.Header.ChainID. Expected {}, got {}",
            state.chain_id, h.chain_id
        )));
    }
    if state.last_block_height == 0 && h.height != state.initial_height {
        return Err(Error::new(format!(
            "wrong Block.Header.Height. Expected {} for initial block, got {}",
            h.height, state.initial_height
        )));
    }
    if state.last_block_height > 0 && h.height != state.last_block_height + 1 {
        return Err(Error::new(format!(
            "wrong Block.Header.Height. Expected {}, got {}",
            state.last_block_height + 1,
            h.height
        )));
    }
    if !h.last_block_id.equals(&state.last_block_id) {
        return Err(Error::new(format!(
            "wrong Block.Header.LastBlockID.  Expected {}, got {}",
            state.last_block_id, h.last_block_id
        )));
    }
    if h.app_hash != state.app_hash {
        return Err(Error::new(format!(
            "wrong Block.Header.AppHash.  Expected {}, got {}",
            hex::encode_upper(&state.app_hash),
            hex::encode_upper(&h.app_hash)
        )));
    }
    let ch = state.consensus_params.hash();
    if h.consensus_hash != ch {
        return Err(Error::new(format!(
            "wrong Block.Header.ConsensusHash.  Expected {}, got {}",
            hex::encode_upper(&ch),
            hex::encode_upper(&h.consensus_hash)
        )));
    }
    if h.last_results_hash != state.last_results_hash {
        return Err(Error::new(format!(
            "wrong Block.Header.LastResultsHash.  Expected {}, got {}",
            hex::encode_upper(&state.last_results_hash),
            hex::encode_upper(&h.last_results_hash)
        )));
    }
    if h.validators_hash != state.validators.hash() {
        return Err(Error::new(format!(
            "wrong Block.Header.ValidatorsHash.  Expected {}, got {}",
            hex::encode_upper(state.validators.hash()),
            hex::encode_upper(&h.validators_hash)
        )));
    }
    if h.next_validators_hash != state.next_validators.hash() {
        return Err(Error::new(format!(
            "wrong Block.Header.NextValidatorsHash.  Expected {}, got {}",
            hex::encode_upper(state.next_validators.hash()),
            hex::encode_upper(&h.next_validators_hash)
        )));
    }
    let lc = block.last_commit.clone().unwrap_or_default();
    if block.header.height == state.initial_height {
        if !lc.signatures.is_empty() {
            return Err(Error::new("initial block can't have LastCommit signatures"));
        }
    } else if !skip_last_commit {
        ctypes::verify_commit(
            &state.chain_id,
            &state.last_validators,
            &state.last_block_id,
            block.header.height - 1,
            &lc,
        )
        .map_err(ctypes_err)?;
    }
    let h = &block.header;
    if h.proposer_address.len() != ctypes::header::ADDRESS_SIZE {
        return Err(Error::new(format!(
            "expected ProposerAddress size {}, got {}",
            ctypes::header::ADDRESS_SIZE,
            h.proposer_address.len()
        )));
    }
    if !state.validators.has_address(&h.proposer_address) {
        return Err(Error::new(format!(
            "block.Header.ProposerAddress {} is not a validator",
            hex::encode_upper(&h.proposer_address)
        )));
    }
    if h.time >= add_duration(now, MAX_BLOCK_TIME_TOLERANCE) {
        return Err(Error::new(format!(
            "block time {} is too far in the future (wall clock {})",
            h.time.go_string(),
            now.go_string()
        )));
    }
    match h.height.cmp(&state.initial_height) {
        std::cmp::Ordering::Greater => {
            if h.time <= state.last_block_time {
                return Err(Error::new(format!(
                    "block time {} not greater than last block time {}",
                    h.time.go_string(),
                    state.last_block_time.go_string()
                )));
            }
            let median = median_time(&lc, &state.last_validators).map_err(|e| {
                Error::new(format!(
                    "error validating block while calculating median time: {e}"
                ))
            })?;
            if h.time != median {
                return Err(Error::new(format!(
                    "invalid block time. Expected {}, got {}",
                    median.go_string(),
                    h.time.go_string()
                )));
            }
        }
        std::cmp::Ordering::Equal => {
            if h.time != state.last_block_time {
                return Err(Error::new(format!(
                    "block time {} is not equal to genesis time {}",
                    h.time.go_string(),
                    state.last_block_time.go_string()
                )));
            }
        }
        std::cmp::Ordering::Less => {
            return Err(Error::new(format!(
                "block height {} lower than initial height {}",
                h.height, state.initial_height
            )));
        }
    }
    let max = state.consensus_params.evidence.max_bytes;
    let got = block.evidence.byte_size();
    if got > max {
        return Err(Error::new(ctypes::evidence::evidence_overflow_message(
            max, got,
        )));
    }
    Ok(())
}

/// `ExecCommitBlock`: FinalizeBlock + Commit without touching the state
/// (handshake replay). Returns the app hash.
pub fn exec_commit_block(
    app: &mut dyn AppConn,
    block: &mut Block,
    state_store: &StateStore,
    initial_height: i64,
) -> Result<Vec<u8>> {
    let commit_info = if block.header.height == initial_height {
        abci::CommitInfo::default()
    } else {
        let last_vals = state_store.load_validators(block.header.height - 1)?;
        build_last_commit_info(block, &last_vals, initial_height)?
    };
    let resp = app.finalize_block(finalize_request(block, commit_info))?;
    if block.data.txs.len() != resp.tx_results.len() {
        return Err(Error::new(format!(
            "expected tx results length to match size of transactions in block. Expected {}, got {}",
            block.data.txs.len(),
            resp.tx_results.len()
        )));
    }
    app.commit()?;
    Ok(resp.app_hash)
}

/// The `RequestFinalizeBlock` for a block (blob txs unwrapped).
pub fn finalize_request(
    block: &mut Block,
    commit_info: abci::CommitInfo,
) -> abci::RequestFinalizeBlock {
    abci::RequestFinalizeBlock {
        txs: finalize_txs(block),
        decided_last_commit: Some(commit_info),
        misbehavior: evidence_abci(block),
        hash: block.hash().unwrap_or_default(),
        height: block.header.height,
        time: Some(block.header.time.to_proto()),
        next_validators_hash: block.header.next_validators_hash.clone(),
        proposer_address: block.header.proposer_address.clone(),
        header: Some(block.header.to_proto()),
    }
}

impl BlockExecutor {
    /// `NewBlockExecutor`.
    #[must_use]
    pub fn new(
        state_store: StateStore,
        block_store: BlockStore,
        app: Box<dyn AppConn>,
        mempool: Box<dyn Mempool>,
        evpool: Box<dyn EvidencePool>,
    ) -> Self {
        Self {
            state_store,
            block_store,
            app,
            mempool,
            evpool,
            pruned_states: 0,
        }
    }

    /// The application connection.
    pub fn app(&mut self) -> &mut dyn AppConn {
        self.app.as_mut()
    }

    /// The mempool.
    pub fn mempool(&mut self) -> &mut dyn Mempool {
        self.mempool.as_mut()
    }

    /// The evidence pool.
    pub fn evpool(&mut self) -> &mut dyn EvidencePool {
        self.evpool.as_mut()
    }

    /// `CreateProposalBlock`: reaps the mempool and evidence, asks the app
    /// (`PrepareProposal`) for the block data (txs, square size, data root).
    pub fn create_proposal_block(
        &mut self,
        height: i64,
        state: &State,
        last_ext_commit: &ExtendedCommit,
        proposer_addr: Vec<u8>,
    ) -> Result<(Block, PartSet)> {
        let mut max_bytes = state.consensus_params.block.max_bytes;
        let empty_max_bytes = max_bytes == -1;
        if empty_max_bytes {
            max_bytes = ctypes::part_set::MAX_BLOCK_SIZE_BYTES;
        }
        let max_gas = state.consensus_params.block.max_gas;
        let (evidence, ev_size) = self
            .evpool
            .pending_evidence(state.consensus_params.evidence.max_bytes);
        let max_data_bytes =
            max_data_bytes(max_bytes, ev_size, state.validators.size()).map_err(ctypes_err)?;
        let max_reap_bytes = if empty_max_bytes {
            -1
        } else {
            max_data_bytes + max_data_bytes / 4
        };
        let txs = self.mempool.reap_max_bytes_max_gas(max_reap_bytes, max_gas);
        let commit = last_ext_commit.to_commit();
        let block = state.make_block_without_partset(
            height,
            Data::new(txs.into_iter().map(Tx).collect(), 0, None),
            commit.clone(),
            evidence.clone(),
            proposer_addr.clone(),
        )?;
        let local_last_commit = if last_ext_commit.height < state.initial_height {
            abci::ExtendedCommitInfo::default()
        } else {
            let vals = self.state_store.load_validators(last_ext_commit.height)?;
            build_extended_commit_info(
                last_ext_commit,
                &vals,
                state.initial_height,
                &state.consensus_params.abci,
            )?
        };
        let req = abci::RequestPrepareProposal {
            max_tx_bytes: max_data_bytes,
            txs: block.data.txs.iter().map(|t| t.0.clone()).collect(),
            local_last_commit: Some(local_last_commit),
            misbehavior: evidence_abci(&block),
            height: block.header.height,
            time: Some(block.header.time.to_proto()),
            next_validators_hash: block.header.next_validators_hash.clone(),
            proposer_address: block.header.proposer_address.clone(),
        };
        let rpp = self.app.prepare_proposal(req)?;
        let txs: Vec<Tx> = rpp.txs.into_iter().map(Tx).collect();
        ctypes::tx::validate_txs(&txs, max_data_bytes).map_err(ctypes_err)?;
        let data = Data::new(txs, rpp.square_size, Some(rpp.data_root_hash));
        state.make_block(height, data, commit, evidence, proposer_addr)
    }

    /// `ProcessProposal`: whether the app accepts the block.
    pub fn process_proposal(&mut self, block: &mut Block, state: &State) -> Result<bool> {
        let commit_info = if block.header.height == state.initial_height {
            abci::CommitInfo::default()
        } else {
            let last_vals = self.state_store.load_validators(block.header.height - 1)?;
            build_last_commit_info(block, &last_vals, state.initial_height)?
        };
        let req = abci::RequestProcessProposal {
            txs: block.data.txs.iter().map(|t| t.0.clone()).collect(),
            proposed_last_commit: Some(commit_info),
            misbehavior: evidence_abci(block),
            hash: block.header.hash().unwrap_or_default(),
            height: block.header.height,
            time: Some(block.header.time.to_proto()),
            next_validators_hash: block.header.next_validators_hash.clone(),
            proposer_address: block.header.proposer_address.clone(),
            square_size: block.data.square_size,
            data_root_hash: block.data.hash(),
            header: Some(block.header.to_proto()),
        };
        let resp = self.app.process_proposal(req)?;
        match ProposalStatus::try_from(resp.status) {
            Ok(ProposalStatus::Accept) => Ok(true),
            Ok(ProposalStatus::Reject) => Ok(false),
            _ => Err(Error::new(format!(
                "ProcessProposal responded with status {}",
                resp.status
            ))),
        }
    }

    /// `ValidateBlock`: state checks plus evidence.
    pub fn validate_block(&mut self, state: &State, block: &mut Block, now: Time) -> Result<()> {
        validate_block(state, block, now)?;
        self.evpool.check_evidence(state, &block.evidence.evidence)
    }

    /// `ApplyBlock`: validates, then executes.
    pub fn apply_block(
        &mut self,
        state: &State,
        block_id: BlockId,
        block: &mut Block,
        now: Time,
    ) -> Result<State> {
        validate_block(state, block, now).map_err(|e| Error::new(format!("invalid block: {e}")))?;
        self.apply_verified_block(state, block_id, block)
    }

    /// `ApplyVerifiedBlock`: FinalizeBlock, save the response, update the
    /// state (validator updates take effect two heights later, param
    /// updates one height later), Commit, save the state, prune.
    pub fn apply_verified_block(
        &mut self,
        state: &State,
        block_id: BlockId,
        block: &mut Block,
    ) -> Result<State> {
        let commit_info = if block.header.height == state.initial_height {
            abci::CommitInfo::default()
        } else {
            let last_vals = self.state_store.load_validators(block.header.height - 1)?;
            build_last_commit_info(block, &last_vals, state.initial_height)?
        };
        let resp = self
            .app
            .finalize_block(finalize_request(block, commit_info))?;
        if block.data.txs.len() != resp.tx_results.len() {
            return Err(Error::new(format!(
                "expected tx results length to match size of transactions in block. Expected {}, got {}",
                block.data.txs.len(),
                resp.tx_results.len()
            )));
        }
        self.state_store
            .save_finalize_block_response(block.header.height, &resp)?;
        validate_validator_updates(&resp.validator_updates, &state.consensus_params.validator)
            .map_err(|e| Error::new(format!("error in validator updates: {e}")))?;
        let validator_updates = validator_updates_from_abci(&resp.validator_updates)?;
        let mut new_state = update_state(state, block_id, &block.header, &resp, &validator_updates)
            .map_err(|e| Error::new(format!("commit failed for application: {e}")))?;
        let retain_height = self.commit(block, &resp)?;
        self.evpool.update(&new_state, &block.evidence.evidence);
        new_state.app_hash.clone_from(&resp.app_hash);
        self.state_store.save(&new_state)?;
        if retain_height > 0 {
            if let Err(e) = self.prune_blocks(retain_height, &new_state) {
                tracing::error!(retain_height, error = %e, "failed to prune blocks");
            }
        }
        Ok(new_state)
    }

    /// `Commit`: app Commit then mempool update; returns the retain height.
    fn commit(&mut self, block: &Block, resp: &abci::ResponseFinalizeBlock) -> Result<i64> {
        let res = self
            .app
            .commit()
            .map_err(|e| Error::new(format!("commit failed for application: {e}")))?;
        self.mempool
            .update(block.header.height, &block.data.txs, &resp.tx_results);
        Ok(res.retain_height)
    }

    fn prune_blocks(&mut self, retain_height: i64, state: &State) -> Result<u64> {
        let base = self.block_store.base();
        if retain_height <= base {
            return Ok(0);
        }
        let (pruned, evidence_point) = self.block_store.prune_blocks(retain_height, state)?;
        let p = self
            .state_store
            .prune_states(base, retain_height, evidence_point)?;
        self.pruned_states += p;
        Ok(pruned)
    }

    /// `ExtendVote`.
    pub fn extend_vote(
        &mut self,
        vote: &Vote,
        block: &mut Block,
        state: &State,
    ) -> Result<Vec<u8>> {
        if !block.hashes_to(&vote.block_id.hash) {
            return Err(Error::new(
                "vote's hash does not match the block it is referring to",
            ));
        }
        let commit_info = if block.header.height == state.initial_height {
            abci::CommitInfo::default()
        } else {
            let last_vals = self.state_store.load_validators(block.header.height - 1)?;
            build_last_commit_info(block, &last_vals, state.initial_height)?
        };
        let req = abci::RequestExtendVote {
            hash: vote.block_id.hash.clone(),
            height: vote.height,
            time: Some(block.header.time.to_proto()),
            txs: block.data.txs.iter().map(|t| t.0.clone()).collect(),
            proposed_last_commit: Some(commit_info),
            misbehavior: evidence_abci(block),
            next_validators_hash: block.header.next_validators_hash.clone(),
            proposer_address: block.header.proposer_address.clone(),
        };
        Ok(self.app.extend_vote(req)?.vote_extension)
    }

    /// `VerifyVoteExtension`.
    pub fn verify_vote_extension(&mut self, vote: &Vote) -> Result<()> {
        let req = abci::RequestVerifyVoteExtension {
            hash: vote.block_id.hash.clone(),
            validator_address: vote.validator_address.clone(),
            height: vote.height,
            vote_extension: vote.extension.clone(),
        };
        let resp = self.app.verify_vote_extension(req)?;
        match VerifyStatus::try_from(resp.status) {
            Ok(VerifyStatus::Accept) => Ok(()),
            Ok(VerifyStatus::Reject) => Err(Error::new("invalid vote extension")),
            _ => Err(Error::new(format!(
                "VerifyVoteExtension responded with status {}",
                resp.status
            ))),
        }
    }
}

/// The block id of a block and its part set.
pub fn block_id_of(block: &mut Block, parts: &PartSet) -> BlockId {
    BlockId {
        hash: block.hash().unwrap_or_default(),
        part_set_header: parts.header(),
    }
}
