//! CometBFT light-client verification as the 07-tendermint client runs it
//! (celestia-core v0.42.3 `light.Verify`, `types.VerifyCommitLight*`,
//! `SignedHeaderFromProto`, `ValidatorSetFromProto`), on the protobuf types.
//!
//! Every failure here is an unregistered Go error (`fmt.Errorf` /
//! `errors.New`), which ABCI reports as `undefined`/1; callers only need the
//! outcome, so errors are plain strings.

use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;
use celestia_proto::celestia_core_proto::tendermint::types as tm;
use prost::Message;

/// `version.BlockProtocol`.
pub const BLOCK_PROTOCOL: u64 = 11;
/// `types.MaxChainIDLen`.
pub const MAX_CHAIN_ID_LEN: usize = 50;
/// `types.MaxVotesCount`.
pub const MAX_VOTES_COUNT: usize = 10_000;
/// `types.MaxSignatureSize`.
pub const MAX_SIGNATURE_SIZE: usize = 64;
/// `types.MaxTotalVotingPower` (`math.MaxInt64 / 8`).
pub const MAX_TOTAL_VOTING_POWER: i64 = i64::MAX / 8;
/// Go's zero `time.Time` as a protobuf timestamp (0001-01-01T00:00:00Z).
pub const GO_ZERO_TIME_SECONDS: i64 = -62_135_596_800;

/// Result alias (Go error strings).
pub type LResult<T> = Result<T, String>;

/// A timestamp as nanoseconds since the Unix epoch (Go `time.Time`).
pub type Nanos = i128;

/// Nanoseconds of a protobuf timestamp (absent = Go zero time).
#[must_use]
pub fn ts_nanos(t: Option<&pbjson_types::Timestamp>) -> Nanos {
    t.map_or(i128::from(GO_ZERO_TIME_SECONDS) * 1_000_000_000, |t| {
        i128::from(t.seconds) * 1_000_000_000 + i128::from(t.nanos)
    })
}

/// A consensus public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubKey {
    /// Ed25519.
    Ed25519([u8; 32]),
    /// secp256k1 (compressed).
    Secp256k1([u8; 33]),
}

impl PubKey {
    /// `crypto/encoding.PubKeyFromProto`.
    pub fn from_proto(
        pk: Option<&celestia_proto::celestia_core_proto::tendermint::crypto::PublicKey>,
    ) -> LResult<Self> {
        match pk.and_then(|p| p.sum.as_ref()) {
            Some(Sum::Ed25519(b)) => b.as_slice().try_into().map(Self::Ed25519).map_err(|_| {
                format!(
                    "invalid size for PubKeyEd25519. Got {}, expected 32",
                    b.len()
                )
            }),
            Some(Sum::Secp256k1(b)) => b.as_slice().try_into().map(Self::Secp256k1).map_err(|_| {
                format!(
                    "invalid size for PubKeySecp256k1. Got {}, expected 33",
                    b.len()
                )
            }),
            None => Err("fromproto: key type <nil> is not supported".into()),
        }
    }

    fn to_proto(&self) -> celestia_proto::celestia_core_proto::tendermint::crypto::PublicKey {
        celestia_proto::celestia_core_proto::tendermint::crypto::PublicKey {
            sum: Some(match self {
                Self::Ed25519(b) => Sum::Ed25519(b.to_vec()),
                Self::Secp256k1(b) => Sum::Secp256k1(b.to_vec()),
            }),
        }
    }

    /// `PubKey.Address()`.
    #[must_use]
    pub fn address(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(b) => crypto::ed25519::address(b).to_vec(),
            Self::Secp256k1(b) => crypto::secp256k1::address(b).to_vec(),
        }
    }

    /// `PubKey.VerifySignature`.
    #[must_use]
    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        match self {
            Self::Ed25519(pk) => sig
                .try_into()
                .is_ok_and(|s| crypto::ed25519::verify(pk, msg, &s)),
            Self::Secp256k1(pk) => sig
                .try_into()
                .is_ok_and(|s| crypto::secp256k1::verify(pk, msg, &s)),
        }
    }
}

/// A validator (`types.Validator`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// Address as given (validated against the key by `validate_basic`).
    pub address: Vec<u8>,
    /// Public key.
    pub pub_key: PubKey,
    /// Voting power.
    pub voting_power: i64,
}

impl Validator {
    /// `ValidatorFromProto`.
    pub fn from_proto(v: Option<&tm::Validator>) -> LResult<Self> {
        let v = v.ok_or("nil validator")?;
        Ok(Self {
            address: v.address.clone(),
            pub_key: PubKey::from_proto(v.pub_key.as_ref())?,
            voting_power: v.voting_power,
        })
    }

    /// `Validator.ValidateBasic`.
    pub fn validate_basic(&self) -> LResult<()> {
        if self.voting_power < 0 {
            return Err("validator has negative voting power".into());
        }
        if self.address != self.pub_key.address() {
            return Err("validator address is incorrectly derived from pubkey".into());
        }
        Ok(())
    }

    /// `Validator.Bytes()`: the `SimpleValidator` encoding.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        tm::SimpleValidator {
            pub_key: Some(self.pub_key.to_proto()),
            voting_power: self.voting_power,
        }
        .encode_to_vec()
    }
}

/// A validator set (`types.ValidatorSet`), validators in the given order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    /// Validators.
    pub validators: Vec<Validator>,
    /// Proposer.
    pub proposer: Validator,
    /// Total voting power.
    pub total_voting_power: i64,
}

impl ValidatorSet {
    /// `ValidatorSetFromProto` (including its `ValidateBasic`).
    pub fn from_proto(vp: Option<&tm::ValidatorSet>) -> LResult<Self> {
        let vp = vp.ok_or("nil validator set")?;
        let validators = vp
            .validators
            .iter()
            .map(|v| Validator::from_proto(Some(v)))
            .collect::<LResult<Vec<_>>>()?;
        let proposer = Validator::from_proto(vp.proposer.as_ref())
            .map_err(|e| format!("fromProto: validatorSet proposer error: {e}"))?;
        let mut total: i64 = 0;
        for v in &validators {
            total = total.saturating_add(v.voting_power);
            if total > MAX_TOTAL_VOTING_POWER {
                return Err(format!(
                    "total voting power {total} exceeds maximum {MAX_TOTAL_VOTING_POWER}"
                ));
            }
        }
        let set = Self {
            validators,
            proposer,
            total_voting_power: total,
        };
        set.validate_basic()?;
        Ok(set)
    }

    fn validate_basic(&self) -> LResult<()> {
        if self.validators.is_empty() {
            return Err("validator set is nil or empty".into());
        }
        for (i, v) in self.validators.iter().enumerate() {
            v.validate_basic()
                .map_err(|e| format!("invalid validator #{i}: {e}"))?;
        }
        self.proposer
            .validate_basic()
            .map_err(|e| format!("proposer failed validate basic, error: {e}"))?;
        if self
            .validators
            .iter()
            .any(|v| v.address == self.proposer.address)
        {
            Ok(())
        } else {
            Err("proposer not in validator set".into())
        }
    }

    /// `ValidatorSet.Hash()`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        let bzs: Vec<Vec<u8>> = self.validators.iter().map(Validator::bytes).collect();
        merkle::root_from_byte_slices(&bzs).to_vec()
    }

    fn get_by_address(&self, addr: &[u8]) -> Option<(usize, &Validator)> {
        self.validators
            .iter()
            .enumerate()
            .find(|(_, v)| v.address == addr)
    }
}

/// `ValidateHash`: empty or 32 bytes.
fn validate_hash(h: &[u8]) -> LResult<()> {
    if !h.is_empty() && h.len() != 32 {
        return Err(format!(
            "expected size to be 32 bytes, got {} bytes",
            h.len()
        ));
    }
    Ok(())
}

fn part_set_header(b: Option<&tm::BlockId>) -> tm::PartSetHeader {
    b.and_then(|b| b.part_set_header.clone())
        .unwrap_or_default()
}

/// `BlockIDFromProto` + `BlockID.ValidateBasic`.
fn validate_block_id(b: &tm::BlockId) -> LResult<()> {
    validate_hash(&b.hash)?;
    validate_hash(&part_set_header(Some(b)).hash)
}

fn block_id_is_zero(b: &tm::BlockId) -> bool {
    let psh = part_set_header(Some(b));
    b.hash.is_empty() && psh.total == 0 && psh.hash.is_empty()
}

/// `HeaderFromProto` (includes `Header.ValidateBasic`).
pub fn validate_header(h: &tm::Header) -> LResult<()> {
    validate_block_id(&h.last_block_id.clone().unwrap_or_default())?;
    if h.version.as_ref().map_or(0, |v| v.block) != BLOCK_PROTOCOL {
        return Err("block protocol is incorrect".into());
    }
    if h.chain_id.len() > MAX_CHAIN_ID_LEN {
        return Err("chainID is too long".into());
    }
    if h.height <= 0 {
        return Err("non-positive Height".into());
    }
    validate_block_id(&h.last_block_id.clone().unwrap_or_default())?;
    validate_hash(&h.last_commit_hash)?;
    validate_hash(&h.data_hash)?;
    validate_hash(&h.evidence_hash)?;
    if h.proposer_address.len() != 20 {
        return Err("invalid ProposerAddress length".into());
    }
    validate_hash(&h.validators_hash)?;
    validate_hash(&h.next_validators_hash)?;
    validate_hash(&h.consensus_hash)?;
    validate_hash(&h.last_results_hash)
}

/// `CommitSig.ValidateBasic` (`FromProto`).
fn validate_commit_sig(cs: &tm::CommitSig) -> LResult<()> {
    match cs.block_id_flag {
        1 => {
            if !cs.validator_address.is_empty() {
                return Err("validator address is present".into());
            }
            if ts_nanos(cs.timestamp.as_ref()) != ts_nanos(None) {
                return Err("time is present".into());
            }
            if !cs.signature.is_empty() {
                return Err("signature is present".into());
            }
        }
        2 | 3 => {
            if cs.validator_address.len() != 20 {
                return Err("expected ValidatorAddress size to be 20 bytes".into());
            }
            if cs.signature.is_empty() {
                return Err("signature is missing".into());
            }
            if cs.signature.len() > MAX_SIGNATURE_SIZE {
                return Err("signature is too big".into());
            }
        }
        f => return Err(format!("unknown BlockIDFlag: {f}")),
    }
    Ok(())
}

/// `CommitFromProto` (includes `Commit.ValidateBasic`).
pub fn validate_commit(c: &tm::Commit) -> LResult<()> {
    if c.signatures.len() > MAX_VOTES_COUNT {
        return Err("commit has too many signatures".into());
    }
    let bid = c.block_id.clone().unwrap_or_default();
    validate_block_id(&bid)?;
    for s in &c.signatures {
        validate_commit_sig(s)?;
    }
    if c.height < 0 {
        return Err("negative Height".into());
    }
    if c.round < 0 {
        return Err("negative Round".into());
    }
    if c.height >= 1 {
        if block_id_is_zero(&bid) {
            return Err("commit cannot be for nil block".into());
        }
        if c.signatures.is_empty() {
            return Err("no signatures in commit".into());
        }
    }
    Ok(())
}

/// Protobuf encoding of a non-empty string/int/bytes as its wrapper
/// message (`cdcEncode`); empty values encode as nothing.
fn cdc_bytes(b: &[u8]) -> Vec<u8> {
    if b.is_empty() {
        return Vec::new();
    }
    pbjson_types::BytesValue::from(prost::bytes::Bytes::copy_from_slice(b)).encode_to_vec()
}

/// `Header.Hash()` (`None` when `ValidatorsHash` is empty).
#[must_use]
pub fn header_hash(h: &tm::Header) -> Option<Vec<u8>> {
    if h.validators_hash.is_empty() {
        return None;
    }
    let version = h.version.unwrap_or_default().encode_to_vec();
    let chain_id = if h.chain_id.is_empty() {
        Vec::new()
    } else {
        pbjson_types::StringValue::from(h.chain_id.clone()).encode_to_vec()
    };
    let height = if h.height == 0 {
        Vec::new()
    } else {
        pbjson_types::Int64Value::from(h.height).encode_to_vec()
    };
    let time = h.time.unwrap_or_default().encode_to_vec();
    let mut bid = h.last_block_id.clone().unwrap_or_default();
    if bid.part_set_header.is_none() {
        bid.part_set_header = Some(tm::PartSetHeader::default());
    }
    let fields = [
        version,
        chain_id,
        height,
        time,
        bid.encode_to_vec(),
        cdc_bytes(&h.last_commit_hash),
        cdc_bytes(&h.data_hash),
        cdc_bytes(&h.validators_hash),
        cdc_bytes(&h.next_validators_hash),
        cdc_bytes(&h.consensus_hash),
        cdc_bytes(&h.app_hash),
        cdc_bytes(&h.last_results_hash),
        cdc_bytes(&h.evidence_hash),
        cdc_bytes(&h.proposer_address),
    ];
    Some(merkle::root_from_byte_slices(&fields).to_vec())
}

/// `SignedHeaderFromProto` followed by `SignedHeader.ValidateBasic(chain_id)`.
pub fn validate_signed_header(sh: Option<&tm::SignedHeader>, chain_id: &str) -> LResult<()> {
    let sh = sh.ok_or("nil SignedHeader")?;
    if let Some(h) = &sh.header {
        validate_header(h)?;
    }
    if let Some(c) = &sh.commit {
        validate_commit(c)?;
    }
    let h = sh.header.as_ref().ok_or("missing header")?;
    let c = sh.commit.as_ref().ok_or("missing commit")?;
    if h.chain_id != chain_id {
        return Err(format!(
            "header belongs to another chain {:?}, not {chain_id:?}",
            h.chain_id
        ));
    }
    if c.height != h.height {
        return Err("header and commit height mismatch".into());
    }
    let hhash = header_hash(h).unwrap_or_default();
    let chash = c.block_id.clone().unwrap_or_default().hash;
    if hhash != chash {
        return Err("commit signs block, header is another block".into());
    }
    Ok(())
}

/// `SignedHeaderFromProto` only (no chain-ID / hash consistency checks).
pub fn signed_header_from_proto(sh: Option<&tm::SignedHeader>) -> LResult<()> {
    let sh = sh.ok_or("nil SignedHeader")?;
    if let Some(h) = &sh.header {
        validate_header(h)?;
    }
    if let Some(c) = &sh.commit {
        validate_commit(c)?;
    }
    Ok(())
}

/// `Commit.VoteSignBytes(chainID, idx)`.
#[must_use]
pub fn vote_sign_bytes(chain_id: &str, c: &tm::Commit, idx: usize) -> Vec<u8> {
    let sig = &c.signatures[idx];
    let block_id = if sig.block_id_flag == 2 {
        c.block_id.clone().unwrap_or_default()
    } else {
        tm::BlockId::default()
    };
    let canonical_block_id = if block_id_is_zero(&block_id) {
        None
    } else {
        let psh = part_set_header(Some(&block_id));
        Some(tm::CanonicalBlockId {
            hash: block_id.hash.clone(),
            part_set_header: Some(tm::CanonicalPartSetHeader {
                total: psh.total,
                hash: psh.hash,
            }),
        })
    };
    let cv = tm::CanonicalVote {
        r#type: tm::SignedMsgType::Precommit as i32,
        height: c.height,
        round: i64::from(c.round),
        block_id: canonical_block_id,
        timestamp: Some(sig.timestamp.unwrap_or(pbjson_types::Timestamp {
            seconds: GO_ZERO_TIME_SECONDS,
            nanos: 0,
        })),
        chain_id: chain_id.to_owned(),
    };
    cv.encode_length_delimited_to_vec()
}

/// `verifyCommitSingle` / `verifyCommitBatch` (identical outcomes): checks
/// signatures until more than `needed` power is tallied.
fn verify_commit_sigs(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &tm::Commit,
    needed: i64,
    by_index: bool,
) -> LResult<()> {
    let mut tallied: i64 = 0;
    let mut seen = std::collections::BTreeSet::new();
    for (idx, sig) in commit.signatures.iter().enumerate() {
        // Light verification ignores everything but votes for the block.
        if sig.block_id_flag != 2 {
            continue;
        }
        let val = if by_index {
            let v = &vals.validators[idx];
            if v.address != sig.validator_address {
                return Err(format!("validator address mismatch at index {idx}"));
            }
            v
        } else {
            let Some((vi, v)) = vals.get_by_address(&sig.validator_address) else {
                continue;
            };
            if !seen.insert(vi) {
                return Err(format!("double vote from validator at index {idx}"));
            }
            v
        };
        let msg = vote_sign_bytes(chain_id, commit, idx);
        if !val.pub_key.verify(&msg, &sig.signature) {
            return Err(format!("wrong signature (#{idx})"));
        }
        tallied = tallied.saturating_add(val.voting_power);
        if tallied > needed {
            return Ok(());
        }
    }
    Err(format!(
        "invalid commit -- insufficient voting power: got {tallied}, needed more than {needed}"
    ))
}

/// `ValidatorSet.VerifyCommitLight`.
pub fn verify_commit_light(
    chain_id: &str,
    vals: &ValidatorSet,
    block_id: &tm::BlockId,
    height: i64,
    commit: &tm::Commit,
) -> LResult<()> {
    if vals.validators.len() != commit.signatures.len() {
        return Err("invalid commit -- wrong set size".into());
    }
    if height != commit.height {
        return Err("invalid commit -- wrong height".into());
    }
    let cbid = commit.block_id.clone().unwrap_or_default();
    if block_id.hash != cbid.hash || part_set_header(Some(block_id)) != part_set_header(Some(&cbid))
    {
        return Err("invalid commit -- wrong block ID".into());
    }
    let needed = vals.total_voting_power * 2 / 3;
    verify_commit_sigs(chain_id, vals, commit, needed, true)
}

/// `ValidatorSet.VerifyCommitLightTrusting`.
pub fn verify_commit_light_trusting(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &tm::Commit,
    numerator: u64,
    denominator: u64,
) -> LResult<()> {
    if denominator == 0 {
        return Err("trustLevel has zero Denominator".into());
    }
    // Go converts with `int64(uint64)` (wrapping).
    #[allow(clippy::cast_possible_wrap)]
    let (num, den) = (numerator as i64, denominator as i64);
    let product = vals
        .total_voting_power
        .checked_mul(num)
        .ok_or("int64 overflow while calculating voting power needed")?;
    let needed = product.wrapping_div(den);
    verify_commit_sigs(chain_id, vals, commit, needed, false)
}

/// The trusted side of a verification: only chain ID, height, time and
/// next validators hash are used (`verifyHeader`'s synthetic header).
#[derive(Debug, Clone)]
pub struct Trusted {
    /// Client chain ID.
    pub chain_id: String,
    /// Trusted height.
    pub height: i64,
    /// Trusted header time.
    pub time: Nanos,
    /// Next validators hash.
    pub next_validators_hash: Vec<u8>,
}

/// Verification parameters.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Trusting period (ns).
    pub trusting_period: i128,
    /// Current block time.
    pub now: Nanos,
    /// Max clock drift (ns).
    pub max_clock_drift: i128,
    /// Trust level numerator.
    pub trust_numerator: u64,
    /// Trust level denominator.
    pub trust_denominator: u64,
}

fn header_expired(trusted: &Trusted, p: &Params) -> bool {
    trusted.time + p.trusting_period <= p.now
}

fn verify_new_header_and_vals(
    sh: &tm::SignedHeader,
    vals: &ValidatorSet,
    trusted: &Trusted,
    p: &Params,
) -> LResult<()> {
    validate_signed_header(Some(sh), &trusted.chain_id)?;
    let h = sh.header.as_ref().ok_or("missing header")?;
    if h.height <= trusted.height {
        return Err("expected new header height to be greater than one of old header".into());
    }
    let t = ts_nanos(h.time.as_ref());
    if t <= trusted.time {
        return Err("expected new header time to be after old header time".into());
    }
    if t >= p.now + p.max_clock_drift {
        return Err("new header has a time from the future".into());
    }
    if h.validators_hash != vals.hash() {
        return Err("expected new header validators to match those that were supplied".into());
    }
    Ok(())
}

/// `light.Verify`.
#[allow(clippy::if_not_else)]
pub fn verify(
    trusted: &Trusted,
    trusted_vals: &ValidatorSet,
    untrusted: &tm::SignedHeader,
    untrusted_vals: &ValidatorSet,
    p: &Params,
) -> LResult<()> {
    let h = untrusted.header.as_ref().ok_or("missing header")?;
    let commit = untrusted.commit.as_ref().ok_or("missing commit")?;
    if h.height != trusted.height + 1 {
        // VerifyNonAdjacent.
        if header_expired(trusted, p) {
            return Err("old header has expired".into());
        }
        verify_new_header_and_vals(untrusted, untrusted_vals, trusted, p)?;
        verify_commit_light_trusting(
            &trusted.chain_id,
            trusted_vals,
            commit,
            p.trust_numerator,
            p.trust_denominator,
        )?;
    } else {
        // VerifyAdjacent.
        if header_expired(trusted, p) {
            return Err("old header has expired".into());
        }
        verify_new_header_and_vals(untrusted, untrusted_vals, trusted, p)?;
        if h.validators_hash != trusted.next_validators_hash {
            return Err(
                "expected old header next validators to match those from new header".into(),
            );
        }
    }
    verify_commit_light(
        &trusted.chain_id,
        untrusted_vals,
        &commit.block_id.clone().unwrap_or_default(),
        h.height,
        commit,
    )
}

/// `light.ValidateTrustLevel`.
pub fn validate_trust_level(numerator: u64, denominator: u64) -> LResult<()> {
    // `Numerator*3` wraps on uint64 overflow in Go.
    if numerator.wrapping_mul(3) < denominator || numerator > denominator || denominator == 0 {
        return Err(format!(
            "trustLevel must be within [1/3, 1], given {numerator}/{denominator}"
        ));
    }
    Ok(())
}
