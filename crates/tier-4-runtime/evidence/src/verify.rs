//! Evidence verification (`evidence/verify.go`).

use ctypes::gofmt::{duration_string, hex_upper};
use ctypes::vote::signed_msg_type_string;
use ctypes::{
    BlockMeta, Commit, DuplicateVoteEvidence, Evidence, Fraction, LightClientAttackEvidence,
    SignedHeader, Time, ValidatorSet,
};
use nodestore::State;
use nodestore::block_store::is_evidence_expired;
use nodestore::state::add_duration;

/// Go's `evidence.BlockStore` interface.
pub trait BlockSource {
    /// `LoadBlockMeta`.
    fn load_block_meta(&self, height: i64) -> Option<BlockMeta>;
    /// `LoadBlockCommit` (the `LastCommit` of block `height + 1`).
    fn load_block_commit(&self, height: i64) -> Option<Commit>;
    /// `Height`.
    fn height(&self) -> i64;
}

impl BlockSource for nodestore::BlockStore {
    fn load_block_meta(&self, height: i64) -> Option<BlockMeta> {
        nodestore::BlockStore::load_block_meta(self, height)
            .ok()
            .flatten()
    }
    fn load_block_commit(&self, height: i64) -> Option<Commit> {
        nodestore::BlockStore::load_block_commit(self, height)
            .ok()
            .flatten()
    }
    fn height(&self) -> i64 {
        nodestore::BlockStore::height(self)
    }
}

/// The validator-set part of Go's `sm.Store`.
pub trait ValidatorSource {
    /// `LoadValidators`.
    fn load_validators(&self, height: i64) -> Result<ValidatorSet, String>;
}

impl ValidatorSource for nodestore::StateStore {
    fn load_validators(&self, height: i64) -> Result<ValidatorSet, String> {
        nodestore::StateStore::load_validators(self, height).map_err(|e| e.to_string())
    }
}

/// `verify`: the evidence against the node's state and stores — the time
/// of the evidence's block, expiry (max age in blocks and duration), then
/// the type-specific checks.
pub fn verify(
    state: &State,
    evidence: &Evidence,
    block_store: &dyn BlockSource,
    state_db: &dyn ValidatorSource,
) -> Result<(), String> {
    let height = state.last_block_height;
    let params = state.consensus_params.evidence;
    let ev_height = evidence.height();
    let Some(meta) = block_store.load_block_meta(ev_height) else {
        return Err(format!("don't have header #{ev_height}"));
    };
    let ev_time = meta.header.time;
    if evidence.time() != ev_time {
        return Err(format!(
            "evidence has a different time to the block it is associated with ({} != {})",
            evidence.time().go_string(),
            ev_time.go_string()
        ));
    }
    if is_evidence_expired(height, state.last_block_time, ev_height, ev_time, &params) {
        return Err(format!(
            "evidence from height {ev_height} (created at: {}) is too old; min height is {} and evidence can not be older than {}",
            ev_time.go_string(),
            height - params.max_age_num_blocks,
            add_duration(state.last_block_time, params.max_age_duration).go_string()
        ));
    }
    match evidence {
        Evidence::DuplicateVote(ev) => {
            let vals = state_db.load_validators(ev_height)?;
            verify_duplicate_vote(ev, &state.chain_id, &vals)
        }
        Evidence::LightClientAttack(ev) => {
            let common_header = get_signed_header(block_store, ev_height)?;
            let common_vals = state_db.load_validators(ev_height)?;
            let conflicting_height = conflicting_header(ev).map_or(0, |h| h.height);
            let mut trusted_header = common_header.clone();
            if ev_height != conflicting_height {
                if let Ok(sh) = get_signed_header(block_store, conflicting_height) {
                    trusted_header = sh;
                } else {
                    {
                        // A forward lunatic attack: trust our latest header.
                        let latest = block_store.height();
                        trusted_header = get_signed_header(block_store, latest)?;
                        let trusted_time = header_time(&trusted_header);
                        let conflicting_time =
                            conflicting_header(ev).map_or(Time::ZERO, |h| h.time);
                        if trusted_time < conflicting_time {
                            return Err(format!(
                                "latest block time ({}) is before conflicting block time ({})",
                                trusted_time.go_string(),
                                conflicting_time.go_string()
                            ));
                        }
                    }
                }
            }
            verify_light_client_attack(
                ev,
                &common_header,
                &trusted_header,
                &common_vals,
                state.last_block_time,
                params.max_age_duration,
            )
        }
    }
}

fn header_time(sh: &SignedHeader) -> Time {
    sh.header.as_ref().map_or(Time::ZERO, |h| h.time)
}

fn conflicting_header(ev: &LightClientAttackEvidence) -> Option<&ctypes::Header> {
    ev.conflicting_block
        .as_ref()
        .and_then(ctypes::LightBlock::header)
}

fn conflicting_commit(ev: &LightClientAttackEvidence) -> Commit {
    ev.conflicting_block
        .as_ref()
        .and_then(|cb| cb.signed_header.as_ref())
        .and_then(|sh| sh.commit.clone())
        .unwrap_or_default()
}

/// `VerifyLightClientAttack`: the conflicting block is signed by 1/3+ of
/// the common validators (lunatic, skipping verification) or correctly
/// derived (equivocation / amnesia), 2/3+ of its own validator set signed
/// it (all signatures checked), the total power matches, and it differs
/// from our trusted header (or, past our head, violates time
/// monotonicity); then the ABCI byzantine validators are checked.
///
/// `now` and `trust_period` are unused, as in Go.
pub fn verify_light_client_attack(
    e: &LightClientAttackEvidence,
    common_header: &SignedHeader,
    trusted_header: &SignedHeader,
    common_vals: &ValidatorSet,
    _now: Time,
    _trust_period: i64,
) -> Result<(), String> {
    let common_height = common_header.header.as_ref().map_or(0, |h| h.height);
    let trusted = trusted_header.header.clone().unwrap_or_default();
    let conflicting = conflicting_header(e).cloned().unwrap_or_default();
    let commit = conflicting_commit(e);
    if common_height != conflicting.height {
        ctypes::verify_commit_light_trusting_all_signatures(
            &trusted.chain_id,
            common_vals,
            &commit,
            Fraction::ONE_THIRD,
        )
        .map_err(|err| format!("skipping verification of conflicting block failed: {err}"))?;
    } else if e.conflicting_header_is_invalid(&trusted) {
        return Err("common height is the same as conflicting block height so expected the conflicting block to be correctly derived yet it wasn't".into());
    }
    let conflicting_vals = e
        .conflicting_block
        .as_ref()
        .and_then(|cb| cb.validator_set.clone())
        .unwrap_or_default();
    ctypes::verify_commit_light_all_signatures(
        &trusted.chain_id,
        &conflicting_vals,
        &commit.block_id,
        conflicting.height,
        &commit,
    )
    .map_err(|err| format!("invalid commit from conflicting block: {err}"))?;
    let (ev_total, vals_total) = (e.total_voting_power, common_vals.total_voting_power());
    if ev_total != vals_total {
        return Err(format!(
            "total voting power from the evidence and our validator set does not match ({ev_total} != {vals_total})"
        ));
    }
    if conflicting.height > trusted.height && conflicting.time > trusted.time {
        return Err(format!(
            "conflicting block doesn't violate monotonically increasing time ({} is after {})",
            conflicting.time.go_string(),
            trusted.time.go_string()
        ));
    }
    let trusted_hash = trusted.hash().unwrap_or_default();
    if trusted_hash == conflicting.hash().unwrap_or_default() {
        return Err(format!(
            "trusted header hash matches the evidence's conflicting header hash: {}",
            hex_upper(&trusted_hash)
        ));
    }
    validate_abci_evidence(e, common_vals, trusted_header)
}

/// `VerifyDuplicateVote`: same validator, height, round and type, different
/// block IDs, powers matching the validator set of that height, and both
/// signatures valid.
pub fn verify_duplicate_vote(
    e: &DuplicateVoteEvidence,
    chain_id: &str,
    val_set: &ValidatorSet,
) -> Result<(), String> {
    let a = e.vote_a.clone().unwrap_or_default();
    let b = e.vote_b.clone().unwrap_or_default();
    let Some((_, val)) = val_set.get_by_address(&a.validator_address) else {
        return Err(format!(
            "address {} was not a validator at height {}",
            hex_upper(&a.validator_address),
            e.height()
        ));
    };
    let pub_key = val.pub_key.clone();
    if a.height != b.height || a.round != b.round || a.vote_type != b.vote_type {
        return Err(format!(
            "h/r/s does not match: {}/{}/{} vs {}/{}/{}",
            a.height,
            a.round,
            signed_msg_type_string(a.vote_type),
            b.height,
            b.round,
            signed_msg_type_string(b.vote_type)
        ));
    }
    if a.validator_address != b.validator_address {
        return Err(format!(
            "validator addresses do not match: {} vs {}",
            hex_upper(&a.validator_address),
            hex_upper(&b.validator_address)
        ));
    }
    if a.block_id.equals(&b.block_id) {
        return Err(format!(
            "block IDs are the same ({}) - not a real duplicate vote",
            a.block_id
        ));
    }
    let addr = &a.validator_address;
    if &pub_key.address() != addr {
        return Err(format!(
            "address ({}) doesn't match pubkey ({} - {})",
            hex_upper(addr),
            pub_key,
            hex_upper(&pub_key.address())
        ));
    }
    if val.voting_power != e.validator_power {
        return Err(format!(
            "validator power from evidence and our validator set does not match ({} != {})",
            e.validator_power, val.voting_power
        ));
    }
    if val_set.total_voting_power() != e.total_voting_power {
        return Err(format!(
            "total voting power from the evidence and our validator set does not match ({} != {})",
            e.total_voting_power,
            val_set.total_voting_power()
        ));
    }
    let sig_ok = |v: &ctypes::Vote| {
        ctypes::canonical::vote_sign_bytes(chain_id, &v.to_proto())
            .is_ok_and(|sb| pub_key.verify_signature(&sb, &v.signature))
    };
    if !sig_ok(&a) {
        return Err("verifying VoteA: invalid signature".into());
    }
    if !sig_ok(&b) {
        return Err("verifying VoteB: invalid signature".into());
    }
    Ok(())
}

/// `validateABCIEvidence`: total power and the byzantine validators (by
/// address, power and key) against those derived from the attack type.
///
/// Evidence reaching a node is always decoded from protobuf, where Go
/// makes `ByzantineValidators` a non-nil slice; so an attack from which no
/// byzantine validators can be derived (amnesia) fails like in Go.
pub fn validate_abci_evidence(
    ev: &LightClientAttackEvidence,
    common_vals: &ValidatorSet,
    trusted_header: &SignedHeader,
) -> Result<(), String> {
    let (ev_total, vals_total) = (ev.total_voting_power, common_vals.total_voting_power());
    if ev_total != vals_total {
        return Err(format!(
            "total voting power from the evidence and our validator set does not match ({ev_total} != {vals_total})"
        ));
    }
    let validators = ev.get_byzantine_validators(common_vals, trusted_header);
    if validators.is_empty() {
        return Err(format!(
            "expected nil validators from an amnesia light client attack but got {}",
            ev.byzantine_validators.len()
        ));
    }
    if validators.len() != ev.byzantine_validators.len() {
        return Err(format!(
            "expected {} byzantine validators from evidence but got {}",
            validators.len(),
            ev.byzantine_validators.len()
        ));
    }
    for (idx, val) in validators.iter().enumerate() {
        let got = &ev.byzantine_validators[idx];
        if got.address != val.address {
            return Err(format!(
                "evidence contained an unexpected byzantine validator address; expected: {}, got: {}",
                hex_upper(&val.address),
                hex_upper(&got.address)
            ));
        }
        if got.voting_power != val.voting_power {
            return Err(format!(
                "evidence contained unexpected byzantine validator power; expected {}, got {}",
                val.voting_power, got.voting_power
            ));
        }
        if got.pub_key.address() != val.address {
            return Err(format!(
                "evidence byzantine validator pubkey does not match address; pubkey address: {}, expected: {}",
                hex_upper(&got.pub_key.address()),
                hex_upper(&val.address)
            ));
        }
    }
    Ok(())
}

/// `getSignedHeader`.
pub fn get_signed_header(
    block_store: &dyn BlockSource,
    height: i64,
) -> Result<SignedHeader, String> {
    let Some(meta) = block_store.load_block_meta(height) else {
        return Err(format!("don't have header at height #{height}"));
    };
    let Some(commit) = block_store.load_block_commit(height) else {
        return Err(format!("don't have commit at height #{height}"));
    };
    Ok(SignedHeader {
        header: Some(meta.header),
        commit: Some(commit),
    })
}

/// Go's `%v` of a `time.Duration` (for messages that print one).
#[must_use]
pub fn go_duration(ns: i64) -> String {
    duration_string(ns)
}
