//! `types.VoteSet` (`types/vote_set.go`) and `libs/bits.BitArray`.

use std::collections::BTreeMap;

use ctypes::proto::libs::bits as pbbits;
use ctypes::{
    BLOCK_ID_FLAG_COMMIT, BlockId, Commit, ExtendedCommit, ExtendedCommitSig, ValidatorSet, Vote,
    params::AbciParams,
};

/// `bits.BitArray` (Go's wire form: `bits` + little-endian `u64` words).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BitArray(pub Vec<bool>);

impl BitArray {
    /// `NewBitArray(n)`.
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self(vec![false; n])
    }
    /// `Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.0.len()
    }
    /// `GetIndex`.
    #[must_use]
    pub fn get(&self, i: usize) -> bool {
        self.0.get(i).copied().unwrap_or(false)
    }
    /// `SetIndex`.
    pub fn set(&mut self, i: usize, v: bool) -> bool {
        match self.0.get_mut(i) {
            Some(b) => {
                *b = v;
                true
            }
            None => false,
        }
    }
    /// `GetTrueIndices`.
    #[must_use]
    pub fn true_indices(&self) -> Vec<usize> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.then_some(i))
            .collect()
    }
    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pbbits::BitArray {
        let mut elems = vec![0u64; self.0.len().div_ceil(64)];
        for (i, b) in self.0.iter().enumerate() {
            if *b {
                elems[i / 64] |= 1 << (i % 64);
            }
        }
        pbbits::BitArray {
            bits: i64::try_from(self.0.len()).unwrap_or(0),
            elems,
        }
    }
    /// `FromProto` (invalid sizes give an empty array).
    #[must_use]
    pub fn from_proto(p: &pbbits::BitArray) -> Self {
        let Ok(n) = usize::try_from(p.bits) else {
            return Self::default();
        };
        if p.elems.len() != n.div_ceil(64) {
            return Self::default();
        }
        Self(
            (0..n)
                .map(|i| p.elems[i / 64] & (1 << (i % 64)) != 0)
                .collect(),
        )
    }
}

/// Why a vote was not added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteError {
    /// `ErrVoteConflictingVotes`: the validator signed two different
    /// blocks (`vote_a` is the earlier one).
    Conflicting {
        /// Existing vote.
        vote_a: Box<Vote>,
        /// New vote.
        vote_b: Box<Vote>,
    },
    /// `ErrVoteNonDeterministicSignature`.
    NonDeterministicSignature(String),
    /// `ErrVoteUnexpectedStep`.
    UnexpectedStep(String),
    /// `ErrVoteInvalidValidatorIndex`.
    InvalidValidatorIndex(String),
    /// `ErrVoteInvalidValidatorAddress`.
    InvalidValidatorAddress(String),
    /// Signature / extension verification failed.
    InvalidSignature(String),
    /// `ErrGotVoteFromUnwantedRound`.
    UnwantedRound,
    /// Anything else.
    Other(String),
}

impl std::fmt::Display for VoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflicting { vote_a, vote_b } => {
                write!(f, "conflicting votes from validator: {vote_a} vs {vote_b}")
            }
            Self::NonDeterministicSignature(s)
            | Self::UnexpectedStep(s)
            | Self::InvalidValidatorIndex(s)
            | Self::InvalidValidatorAddress(s)
            | Self::InvalidSignature(s)
            | Self::Other(s) => f.write_str(s),
            Self::UnwantedRound => f.write_str(
                "peer has sent a vote that does not match our round for more than one round",
            ),
        }
    }
}

impl std::error::Error for VoteError {}

#[derive(Debug, Clone)]
struct BlockVotes {
    peer_maj23: bool,
    bit_array: BitArray,
    votes: Vec<Option<Vote>>,
    sum: i64,
}

impl BlockVotes {
    fn new(peer_maj23: bool, n: usize) -> Self {
        Self {
            peer_maj23,
            bit_array: BitArray::new(n),
            votes: vec![None; n],
            sum: 0,
        }
    }

    fn add_verified_vote(&mut self, vote: &Vote, power: i64) {
        let idx = vote.validator_index as usize;
        if self.votes[idx].is_none() {
            self.bit_array.set(idx, true);
            self.votes[idx] = Some(vote.clone());
            self.sum += power;
        }
    }
}

/// Votes of one type for one height/round, with conflict tracking.
#[derive(Debug, Clone)]
pub struct VoteSet {
    chain_id: String,
    height: i64,
    round: i32,
    vote_type: i32,
    val_set: ValidatorSet,
    extensions_enabled: bool,
    votes_bit_array: BitArray,
    votes: Vec<Option<Vote>>,
    sum: i64,
    maj23: Option<BlockId>,
    votes_by_block: BTreeMap<Vec<u8>, BlockVotes>,
    peer_maj23s: BTreeMap<String, BlockId>,
}

impl VoteSet {
    /// `NewVoteSet`.
    #[must_use]
    pub fn new(
        chain_id: &str,
        height: i64,
        round: i32,
        vote_type: i32,
        val_set: ValidatorSet,
    ) -> Self {
        let n = val_set.size();
        Self {
            chain_id: chain_id.to_owned(),
            height,
            round,
            vote_type,
            val_set,
            extensions_enabled: false,
            votes_bit_array: BitArray::new(n),
            votes: vec![None; n],
            sum: 0,
            maj23: None,
            votes_by_block: BTreeMap::new(),
            peer_maj23s: BTreeMap::new(),
        }
    }

    /// `NewExtendedVoteSet`: verifies vote extensions too.
    #[must_use]
    pub fn new_extended(
        chain_id: &str,
        height: i64,
        round: i32,
        vote_type: i32,
        val_set: ValidatorSet,
    ) -> Self {
        let mut vs = Self::new(chain_id, height, round, vote_type, val_set);
        vs.extensions_enabled = true;
        vs
    }

    /// Height.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.height
    }
    /// Round.
    #[must_use]
    pub fn round(&self) -> i32 {
        self.round
    }
    /// Vote type.
    #[must_use]
    pub fn vote_type(&self) -> i32 {
        self.vote_type
    }
    /// Number of validators.
    #[must_use]
    pub fn size(&self) -> usize {
        self.val_set.size()
    }
    /// The validator set.
    #[must_use]
    pub fn validators(&self) -> &ValidatorSet {
        &self.val_set
    }

    /// `AddVote`: `Ok(true)` when new, `Ok(false)` for a duplicate.
    /// A conflicting vote returns [`VoteError::Conflicting`] (it may still
    /// have been added when a peer claimed a +2/3 majority for it).
    pub fn add_vote(&mut self, vote: &Vote) -> Result<bool, (bool, VoteError)> {
        let val_index = vote.validator_index;
        if val_index < 0 {
            return Err((
                false,
                VoteError::InvalidValidatorIndex("index < 0: invalid validator index".into()),
            ));
        }
        if vote.validator_address.is_empty() {
            return Err((
                false,
                VoteError::InvalidValidatorAddress(
                    "empty address: invalid validator address".into(),
                ),
            ));
        }
        if vote.height != self.height
            || vote.round != self.round
            || vote.vote_type != self.vote_type
        {
            return Err((
                false,
                VoteError::UnexpectedStep(format!(
                    "expected {}/{}/{}, but got {}/{}/{}: unexpected step",
                    self.height,
                    self.round,
                    self.vote_type,
                    vote.height,
                    vote.round,
                    vote.vote_type
                )),
            ));
        }
        let Some((lookup_addr, val)) = self.val_set.get_by_index(val_index) else {
            return Err((
                false,
                VoteError::InvalidValidatorIndex(format!(
                    "cannot find validator {val_index} in valSet of size {}: invalid validator index",
                    self.val_set.size()
                )),
            ));
        };
        if vote.validator_address != lookup_addr {
            return Err((
                false,
                VoteError::InvalidValidatorAddress(format!(
                    "vote.ValidatorAddress ({}) does not match address ({}) for vote.ValidatorIndex ({val_index})",
                    hex::encode_upper(&vote.validator_address),
                    hex::encode_upper(&lookup_addr)
                )),
            ));
        }
        let block_key = vote.block_id.key();
        if let Some(existing) = self.get_vote(val_index, &block_key) {
            if existing.signature == vote.signature {
                return Ok(false);
            }
            return Err((
                false,
                VoteError::NonDeterministicSignature(format!(
                    "existing vote: {existing}; new vote: {vote}: non-deterministic signature"
                )),
            ));
        }
        if self.extensions_enabled {
            vote.verify_vote_and_extension(&self.chain_id, &val.pub_key)
                .map_err(|e| {
                    (
                        false,
                        VoteError::InvalidSignature(format!(
                            "failed to verify extended vote with ChainID {} and PubKey {}: {e}",
                            self.chain_id, val.pub_key
                        )),
                    )
                })?;
        } else {
            vote.verify(&self.chain_id, &val.pub_key).map_err(|e| {
                (
                    false,
                    VoteError::InvalidSignature(format!(
                        "failed to verify vote with ChainID {} and PubKey {}: {e}",
                        self.chain_id, val.pub_key
                    )),
                )
            })?;
            if !vote.extension_signature.is_empty() || !vote.extension.is_empty() {
                return Err((
                    false,
                    VoteError::Other(format!(
                        "unexpected vote extension data present in vote; ext_len {}, sig_len {}",
                        vote.extension.len(),
                        vote.extension_signature.len()
                    )),
                ));
            }
        }
        let (added, conflicting) = self.add_verified_vote(vote, block_key, val.voting_power);
        if let Some(c) = conflicting {
            return Err((
                added,
                VoteError::Conflicting {
                    vote_a: Box::new(c),
                    vote_b: Box::new(vote.clone()),
                },
            ));
        }
        Ok(added)
    }

    fn get_vote(&self, val_index: i32, block_key: &[u8]) -> Option<&Vote> {
        let idx = val_index as usize;
        if let Some(Some(existing)) = self.votes.get(idx) {
            if existing.block_id.key() == block_key {
                return Some(existing);
            }
        }
        self.votes_by_block
            .get(block_key)
            .and_then(|bv| bv.votes.get(idx))
            .and_then(Option::as_ref)
    }

    fn add_verified_vote(
        &mut self,
        vote: &Vote,
        block_key: Vec<u8>,
        power: i64,
    ) -> (bool, Option<Vote>) {
        let idx = vote.validator_index as usize;
        let mut conflicting = None;
        if let Some(existing) = &self.votes[idx] {
            conflicting = Some(existing.clone());
            if self.maj23.as_ref().is_some_and(|m| m.key() == block_key) {
                self.votes[idx] = Some(vote.clone());
                self.votes_bit_array.set(idx, true);
            }
        } else {
            self.votes[idx] = Some(vote.clone());
            self.votes_bit_array.set(idx, true);
            self.sum += power;
        }
        let n = self.val_set.size();
        if let Some(bv) = self.votes_by_block.get(&block_key) {
            if conflicting.is_some() && !bv.peer_maj23 {
                return (false, conflicting);
            }
        } else {
            if conflicting.is_some() {
                return (false, conflicting);
            }
            self.votes_by_block
                .insert(block_key.clone(), BlockVotes::new(false, n));
        }
        let quorum = self.val_set.total_voting_power() * 2 / 3 + 1;
        let Some(bv) = self.votes_by_block.get_mut(&block_key) else {
            return (false, conflicting);
        };
        let orig_sum = bv.sum;
        bv.add_verified_vote(vote, power);
        if orig_sum < quorum && quorum <= bv.sum && self.maj23.is_none() {
            self.maj23 = Some(vote.block_id.clone());
            for (i, v) in bv.votes.iter().enumerate() {
                if let Some(v) = v {
                    self.votes[i] = Some(v.clone());
                }
            }
        }
        (true, conflicting)
    }

    /// `SetPeerMaj23`: a peer claims +2/3 for `block_id`.
    pub fn set_peer_maj23(&mut self, peer: &str, block_id: BlockId) -> Result<(), String> {
        let key = block_id.key();
        if let Some(existing) = self.peer_maj23s.get(peer) {
            if existing.equals(&block_id) {
                return Ok(());
            }
            return Err(format!(
                "setPeerMaj23: Received conflicting blockID from peer {peer}. Got {block_id}, expected {existing}"
            ));
        }
        self.peer_maj23s.insert(peer.to_owned(), block_id);
        let n = self.val_set.size();
        self.votes_by_block
            .entry(key)
            .and_modify(|bv| bv.peer_maj23 = true)
            .or_insert_with(|| BlockVotes::new(true, n));
        Ok(())
    }

    /// `BitArray`.
    #[must_use]
    pub fn bit_array(&self) -> BitArray {
        self.votes_bit_array.clone()
    }

    /// `BitArrayByBlockID`.
    #[must_use]
    pub fn bit_array_by_block_id(&self, block_id: &BlockId) -> Option<BitArray> {
        self.votes_by_block
            .get(&block_id.key())
            .map(|bv| bv.bit_array.clone())
    }

    /// `GetByIndex`.
    #[must_use]
    pub fn get_by_index(&self, idx: i32) -> Option<&Vote> {
        usize::try_from(idx)
            .ok()
            .and_then(|i| self.votes.get(i))
            .and_then(Option::as_ref)
    }

    /// `GetByAddress`.
    #[must_use]
    pub fn get_by_address(&self, address: &[u8]) -> Option<&Vote> {
        let (idx, _) = self.val_set.get_by_address(address)?;
        self.get_by_index(idx)
    }

    /// `List`.
    #[must_use]
    pub fn list(&self) -> Vec<Vote> {
        self.votes.iter().flatten().cloned().collect()
    }

    /// All votes known, including conflicting ones tracked per block.
    #[must_use]
    pub fn all_votes(&self) -> Vec<Vote> {
        let mut out = self.list();
        for bv in self.votes_by_block.values() {
            for v in bv.votes.iter().flatten() {
                if !out.iter().any(|o| o.signature == v.signature) {
                    out.push(v.clone());
                }
            }
        }
        out
    }

    /// `HasTwoThirdsMajority`.
    #[must_use]
    pub fn has_two_thirds_majority(&self) -> bool {
        self.maj23.is_some()
    }

    /// `IsCommit`.
    #[must_use]
    pub fn is_commit(&self) -> bool {
        self.vote_type == ctypes::vote::PRECOMMIT_TYPE && self.maj23.is_some()
    }

    /// `HasTwoThirdsAny`.
    #[must_use]
    pub fn has_two_thirds_any(&self) -> bool {
        self.sum > self.val_set.total_voting_power() * 2 / 3
    }

    /// `HasAll`.
    #[must_use]
    pub fn has_all(&self) -> bool {
        self.sum == self.val_set.total_voting_power()
    }

    /// `TwoThirdsMajority`.
    #[must_use]
    pub fn two_thirds_majority(&self) -> Option<BlockId> {
        self.maj23.clone()
    }

    /// Summed power of seen votes.
    #[must_use]
    pub fn sum(&self) -> i64 {
        self.sum
    }

    /// `MakeExtendedCommit`. Errors where Go panics.
    pub fn make_extended_commit(&self, ap: &AbciParams) -> Result<ExtendedCommit, String> {
        if self.vote_type != ctypes::vote::PRECOMMIT_TYPE {
            return Err("Cannot MakeExtendCommit() unless VoteSet.Type is PrecommitType".into());
        }
        let Some(maj23) = &self.maj23 else {
            return Err("Cannot MakeExtendCommit() unless a blockhash has +2/3".into());
        };
        let mut sigs = Vec::with_capacity(self.votes.len());
        for v in &self.votes {
            let sig = match v {
                None => ExtendedCommitSig::absent(),
                Some(v) => {
                    let s = v.extended_commit_sig().map_err(|e| e.to_string())?;
                    if s.commit_sig.block_id_flag == BLOCK_ID_FLAG_COMMIT
                        && !v.block_id.equals(maj23)
                    {
                        ExtendedCommitSig::absent()
                    } else {
                        s
                    }
                }
            };
            sigs.push(sig);
        }
        let ec = ExtendedCommit {
            height: self.height,
            round: self.round,
            block_id: maj23.clone(),
            extended_signatures: sigs,
        };
        let enabled = ap
            .vote_extensions_enabled(ec.height)
            .map_err(|e| e.to_string())?;
        ec.ensure_extensions(enabled).map_err(|e| {
            format!(
                "problem with vote extension data when making extended commit of height {}; {e}",
                ec.height
            )
        })?;
        Ok(ec)
    }

    /// `Commit.ToVoteSet`.
    pub fn from_commit(
        chain_id: &str,
        commit: &Commit,
        vals: ValidatorSet,
    ) -> Result<Self, String> {
        let mut vs = Self::new(
            chain_id,
            commit.height,
            commit.round,
            ctypes::vote::PRECOMMIT_TYPE,
            vals,
        );
        for (idx, cs) in commit.signatures.iter().enumerate() {
            if cs.block_id_flag == ctypes::BLOCK_ID_FLAG_ABSENT {
                continue;
            }
            let i = i32::try_from(idx).map_err(|e| e.to_string())?;
            let vote = commit.get_vote(i).map_err(|e| e.to_string())?;
            vote.validate_basic()
                .map_err(|e| format!("failed to validate vote reconstructed from commit: {e}"))?;
            match vs.add_vote(&vote) {
                Ok(true) => {}
                Ok(false) => return Err("failed to reconstruct vote set from commit".into()),
                Err((_, e)) => {
                    return Err(format!("failed to reconstruct vote set from commit: {e}"));
                }
            }
        }
        Ok(vs)
    }

    /// `ExtendedCommit.ToExtendedVoteSet`.
    pub fn from_extended_commit(
        chain_id: &str,
        ec: &ExtendedCommit,
        vals: ValidatorSet,
    ) -> Result<Self, String> {
        let mut vs = Self::new_extended(
            chain_id,
            ec.height,
            ec.round,
            ctypes::vote::PRECOMMIT_TYPE,
            vals,
        );
        for (idx, s) in ec.extended_signatures.iter().enumerate() {
            if s.commit_sig.block_id_flag == ctypes::BLOCK_ID_FLAG_ABSENT {
                continue;
            }
            let i = i32::try_from(idx).map_err(|e| e.to_string())?;
            let vote = ec.get_extended_vote(i).map_err(|e| e.to_string())?;
            vote.validate_basic().map_err(|e| {
                format!("failed to validate vote reconstructed from LastCommit: {e}")
            })?;
            match vs.add_vote(&vote) {
                Ok(true) => {}
                Ok(false) => {
                    return Err("failed to reconstruct vote set from extended commit".into());
                }
                Err((_, e)) => {
                    return Err(format!(
                        "failed to reconstruct vote set from extended commit: {e}"
                    ));
                }
            }
        }
        Ok(vs)
    }

    /// `StringShort`-like summary.
    #[must_use]
    pub fn string_short(&self) -> String {
        format!(
            "VoteSet{{H:{} R:{} T:{} +2/3:{} {}/{}}}",
            self.height,
            self.round,
            self.vote_type,
            self.maj23
                .as_ref()
                .map_or_else(|| "<nil>".into(), ToString::to_string),
            self.sum,
            self.val_set.total_voting_power()
        )
    }
}
