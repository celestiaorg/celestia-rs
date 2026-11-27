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

/// `types.MaxBlockPartsCount`: `MaxBlockSizeBytes / BlockPartSizeBytes + 1`
/// (128 MiB / 64 KiB + 1), computed once at package init.
pub const MAX_BLOCK_PARTS_COUNT: u32 = 2049;

thread_local! {
    static MAX_BLOCK_PARTS: std::cell::Cell<u32> =
        const { std::cell::Cell::new(MAX_BLOCK_PARTS_COUNT) };
}

/// Overrides `types.MaxBlockPartsCount` for the light client's part-set
/// checks on this thread: celestia-app's `test/interop` setup (the
/// ibctesting fixtures `ibc-edge` / `ibc-upgrades` were recorded with it)
/// sets it to 10 000. Nodes keep [`MAX_BLOCK_PARTS_COUNT`].
pub fn set_max_block_parts_count(n: u32) {
    MAX_BLOCK_PARTS.with(|c| c.set(n));
}

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
    /// Proposer priority (only printed, in `Validator.String()`).
    pub proposer_priority: i64,
}

impl Validator {
    /// `ValidatorFromProto`.
    pub fn from_proto(v: Option<&tm::Validator>) -> LResult<Self> {
        let v = v.ok_or("nil validator")?;
        Ok(Self {
            address: v.address.clone(),
            pub_key: PubKey::from_proto(v.pub_key.as_ref())?,
            voting_power: v.voting_power,
            proposer_priority: v.proposer_priority,
        })
    }

    /// `Validator.String()`: `Validator{ADDR PUBKEY VP:n A:p}`.
    fn go_string(&self) -> String {
        let pub_key = match &self.pub_key {
            PubKey::Ed25519(b) => format!("PubKeyEd25519{{{}}}", hex::encode_upper(b)),
            PubKey::Secp256k1(b) => format!("PubKeySecp256k1{{{}}}", hex::encode_upper(b)),
        };
        format!(
            "Validator{{{} {pub_key} VP:{} A:{}}}",
            hex::encode_upper(&self.address),
            self.voting_power,
            self.proposer_priority
        )
    }

    /// `Validator.ValidateBasic`.
    pub fn validate_basic(&self) -> LResult<()> {
        if self.voting_power < 0 {
            return Err("validator has negative voting power".into());
        }
        let addr = self.pub_key.address();
        if self.address != addr {
            return Err(format!(
                "validator address is incorrectly derived from pubkey. Exp: {}, got {}",
                hex::encode_upper(&addr),
                hex::encode_upper(&self.address)
            ));
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
    /// `ValidatorSet.AllKeysHaveSameType`.
    fn all_keys_have_same_type(&self) -> bool {
        self.validators
            .windows(2)
            .all(|w| std::mem::discriminant(&w[0].pub_key) == std::mem::discriminant(&w[1].pub_key))
    }

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

/// `BlockIDFromProto` + `BlockID.ValidateBasic`: `PartSetHeaderFromProto`
/// (`PartSetHeader.ValidateBasic`: the part count, then the hash) first.
/// celestia-core v0.34 has no part count limit.
fn validate_block_id(b: &tm::BlockId) -> LResult<()> {
    let psh = part_set_header(Some(b));
    let max = MAX_BLOCK_PARTS.with(std::cell::Cell::get);
    // celestia-core v0.34 (app versions 1 to 3) checks only the hash.
    if psh.total > max && !sdk::version::is_sdk_v046(0) {
        return Err(format!(
            "total {} exceeds MaxBlockPartsCount {max}",
            psh.total
        ));
    }
    validate_hash(&psh.hash).map_err(|e| format!("wrong Hash: {e}"))?;
    validate_hash(&b.hash).map_err(|_| "wrong Hash".to_owned())
}

fn block_id_is_zero(b: &tm::BlockId) -> bool {
    let psh = part_set_header(Some(b));
    b.hash.is_empty() && psh.total == 0 && psh.hash.is_empty()
}

/// `HeaderFromProto` (includes `Header.ValidateBasic`).
pub fn validate_header(h: &tm::Header) -> LResult<()> {
    validate_block_id(&h.last_block_id.clone().unwrap_or_default())?;
    let block = h.version.as_ref().map_or(0, |v| v.block);
    if block != BLOCK_PROTOCOL {
        return Err(format!(
            "block protocol is incorrect: got: {block}, want: {BLOCK_PROTOCOL} "
        ));
    }
    let chain_len = gostr::go_len(&h.chain_id);
    if chain_len > MAX_CHAIN_ID_LEN {
        return Err(format!(
            "chainID is too long; got: {chain_len}, max: {MAX_CHAIN_ID_LEN}"
        ));
    }
    if h.height < 0 {
        return Err("negative Height".into());
    }
    if h.height == 0 {
        return Err("zero Height".into());
    }
    validate_block_id(&h.last_block_id.clone().unwrap_or_default())
        .map_err(|e| format!("wrong LastBlockID: {e}"))?;
    validate_hash(&h.last_commit_hash).map_err(|e| format!("wrong LastCommitHash: {e}"))?;
    validate_hash(&h.data_hash).map_err(|e| format!("wrong DataHash: {e}"))?;
    validate_hash(&h.evidence_hash).map_err(|e| format!("wrong EvidenceHash: {e}"))?;
    if h.proposer_address.len() != 20 {
        return Err(format!(
            "invalid ProposerAddress length; got: {}, expected: 20",
            h.proposer_address.len()
        ));
    }
    validate_hash(&h.validators_hash).map_err(|e| format!("wrong ValidatorsHash: {e}"))?;
    validate_hash(&h.next_validators_hash).map_err(|e| format!("wrong NextValidatorsHash: {e}"))?;
    validate_hash(&h.consensus_hash).map_err(|e| format!("wrong ConsensusHash: {e}"))?;
    validate_hash(&h.last_results_hash).map_err(|e| format!("wrong LastResultsHash: {e}"))
}

/// `CommitSig.FromProto`'s `BlockIDFlag(csp.BlockIdFlag)`: Go's
/// `BlockIDFlag` is a `byte`, so the protobuf `int32` is truncated to its
/// low 8 bits (`258` and `-254` are `Commit`); the flag is not part of the
/// signed vote.
fn flag(cs: &tm::CommitSig) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let f = cs.block_id_flag as u8;
    f
}

/// `CommitSig.ValidateBasic` (`FromProto`).
fn validate_commit_sig(cs: &tm::CommitSig) -> LResult<()> {
    match flag(cs) {
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
                return Err(format!(
                    "expected ValidatorAddress size to be 20 bytes, got {} bytes",
                    cs.validator_address.len()
                ));
            }
            if cs.signature.is_empty() {
                return Err("signature is missing".into());
            }
            if cs.signature.len() > MAX_SIGNATURE_SIZE {
                return Err(format!("signature is too big (max: {MAX_SIGNATURE_SIZE})"));
            }
        }
        f => return Err(format!("unknown BlockIDFlag: {f}")),
    }
    Ok(())
}

/// `CommitFromProto` (includes `Commit.ValidateBasic`).
pub fn validate_commit(c: &tm::Commit) -> LResult<()> {
    if c.signatures.len() > MAX_VOTES_COUNT {
        return Err(format!(
            "commit has too many signatures: {}, max: {MAX_VOTES_COUNT}",
            c.signatures.len()
        ));
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
            "header belongs to another chain {}, not {}",
            gostr::quote(&h.chain_id),
            gostr::quote(chain_id)
        ));
    }
    if c.height != h.height {
        return Err(format!(
            "header and commit height mismatch: {} vs {}",
            h.height, c.height
        ));
    }
    let hhash = header_hash(h).unwrap_or_default();
    let chash = c.block_id.clone().unwrap_or_default().hash;
    if hhash != chash {
        return Err(format!(
            "commit signs block {}, header is block {}",
            hex::encode_upper(&chash),
            hex::encode_upper(&hhash)
        ));
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
    let block_id = if flag(sig) == 2 {
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

/// `tmbytes.Fingerprint`: the first 6 bytes, zero padded.
fn fingerprint(b: &[u8]) -> [u8; 6] {
    let mut f = [0u8; 6];
    let n = b.len().min(6);
    f[..n].copy_from_slice(&b[..n]);
    f
}

/// `CommitSig.String()`: `CommitSig{SIG6 by ADDR6 on FLAG @ TIME}`, the
/// time in `CanonicalTime` form (RFC 3339, milliseconds, UTC).
fn commit_sig_string(cs: &tm::CommitSig) -> String {
    let t = cs.timestamp.unwrap_or(pbjson_types::Timestamp {
        seconds: GO_ZERO_TIME_SECONDS,
        nanos: 0,
    });
    let (y, mo, d, h, mi, sec, ns) = sdk::time::to_civil(&t);
    let year = if y < 0 {
        format!("-{:04}", y.unsigned_abs())
    } else {
        format!("{y:04}")
    };
    format!(
        "CommitSig{{{} by {} on {} @ {year}-{mo:02}-{d:02}T{h:02}:{mi:02}:{sec:02}.{:03}Z}}",
        hex::encode_upper(fingerprint(&cs.signature)),
        hex::encode_upper(fingerprint(&cs.validator_address)),
        flag(cs),
        ns / 1_000_000
    )
}

/// Which celestia-core commit verification the executing release runs.
#[derive(Clone, Copy)]
struct CommitRules {
    /// `verifyCommitBatch` for an ed25519 proposer and at least
    /// `batchVerifyThreshold` (2) signatures (celestia-core v0.38 on:
    /// celestia-app v4.1.0 on). celestia-app v3.13.0's celestia-core v1
    /// verifies one signature at a time.
    batch: bool,
    /// The lookup by index checks the validator address (celestia-core
    /// v0.39.30 on: celestia-app v8.0.8 on).
    address_check: bool,
}

impl CommitRules {
    /// The rules of the release that defines the executing app version:
    /// app versions 2 and 3 execute as 9 elsewhere, but their light client
    /// is celestia-app v3.13.0's (celestia-core v1).
    fn of(vals: &ValidatorSet, commit: &tm::Commit) -> Self {
        let release = sdk::version::Release::for_app_version(sdk::version::app_version())
            .unwrap_or_else(|| sdk::version::release(0));
        Self {
            // `shouldBatchVerify`.
            batch: release.light_commit_batch_verification()
                && commit.signatures.len() >= 2
                && matches!(vals.proposer.pub_key, PubKey::Ed25519(_))
                && vals.all_keys_have_same_type(),
            address_check: release.light_commit_address_check(),
        }
    }
}

/// `verifyCommitSingle` / `verifyCommitBatch`: checks signatures until
/// more than `needed` power is tallied.
fn verify_commit_sigs(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &tm::Commit,
    needed: i64,
    by_index: bool,
) -> LResult<()> {
    let rules = CommitRules::of(vals, commit);
    if rules.batch {
        verify_commit_batch(chain_id, vals, commit, needed, by_index, rules)
    } else {
        verify_commit_single(chain_id, vals, commit, needed, by_index, rules)
    }
}

/// The validator signing `commit.signatures[idx]`: by index (with the
/// release's address check) or by address (skipping unknown ones and
/// failing on a second signature of a validator).
fn commit_validator<'a>(
    vals: &'a ValidatorSet,
    commit: &tm::Commit,
    idx: usize,
    by_index: bool,
    rules: CommitRules,
    seen: &mut std::collections::BTreeMap<usize, usize>,
) -> LResult<Option<&'a Validator>> {
    let sig = &commit.signatures[idx];
    if by_index {
        let v = &vals.validators[idx];
        if rules.address_check && v.address != sig.validator_address {
            return Err(format!(
                "validator address mismatch at index {idx}: expected {}, got {}",
                hex::encode_upper(&v.address),
                hex::encode_upper(&sig.validator_address)
            ));
        }
        return Ok(Some(v));
    }
    let Some((vi, v)) = vals.get_by_address(&sig.validator_address) else {
        return Ok(None);
    };
    if let Some(first) = seen.insert(vi, idx) {
        return Err(format!(
            "double vote from {} ({first} and {idx})",
            v.go_string()
        ));
    }
    Ok(Some(v))
}

/// `verifyCommitBatch`: structural errors end the loop at once, the tally
/// is checked before any signature, and the first bad signature in the
/// batch is reported as `%X` of its `CommitSig.String()`.
fn verify_commit_batch(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &tm::Commit,
    needed: i64,
    by_index: bool,
    rules: CommitRules,
) -> LResult<()> {
    let mut tallied: i64 = 0;
    let mut seen = std::collections::BTreeMap::new();
    let mut batch: Vec<(usize, &Validator, Vec<u8>)> = Vec::new();
    for (idx, sig) in commit.signatures.iter().enumerate() {
        // Light verification ignores everything but votes for the block.
        if flag(sig) != 2 {
            continue;
        }
        let Some(val) = commit_validator(vals, commit, idx, by_index, rules, &mut seen)? else {
            continue;
        };
        // ed25519 `BatchVerifier.Add`.
        if sig.signature.len() != 64 {
            return Err("invalid signature".into());
        }
        batch.push((idx, val, vote_sign_bytes(chain_id, commit, idx)));
        tallied = tallied.saturating_add(val.voting_power);
        if tallied > needed {
            break;
        }
    }
    if tallied <= needed {
        return Err(format!(
            "invalid commit -- insufficient voting power: got {tallied}, needed more than {needed}"
        ));
    }
    if let Some(bad) = crypto::parallel::first_failure(&batch, |(idx, val, msg)| {
        val.pub_key.verify(msg, &commit.signatures[*idx].signature)
    }) {
        let idx = batch[bad].0;
        return Err(format!(
            "wrong signature (#{idx}): {}",
            hex::encode_upper(commit_sig_string(&commit.signatures[idx]))
        ));
    }
    Ok(())
}

/// `verifyCommitSingle`: checks signatures in order.
///
/// The cheap checks (validator lookup, double votes) and the tally run in
/// order first and collect the signatures the sequential loop would check
/// before it stops; those are then verified in parallel. The first bad
/// signature among them is reported before a later structural error, as
/// in the sequential loop, and signatures past the stop are never checked.
fn verify_commit_single(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &tm::Commit,
    needed: i64,
    by_index: bool,
    rules: CommitRules,
) -> LResult<()> {
    let mut tallied: i64 = 0;
    let mut seen = std::collections::BTreeMap::new();
    let mut to_check: Vec<(usize, &Validator, Vec<u8>)> = Vec::new();
    // What the sequential loop returns if every collected signature is good.
    let mut outcome: Option<LResult<()>> = None;
    for (idx, sig) in commit.signatures.iter().enumerate() {
        // Light verification ignores everything but votes for the block.
        if flag(sig) != 2 {
            continue;
        }
        let val = match commit_validator(vals, commit, idx, by_index, rules, &mut seen) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                outcome = Some(Err(e));
                break;
            }
        };
        to_check.push((idx, val, vote_sign_bytes(chain_id, commit, idx)));
        tallied = tallied.saturating_add(val.voting_power);
        if tallied > needed {
            outcome = Some(Ok(()));
            break;
        }
    }
    if let Some(bad) = crypto::parallel::first_failure(&to_check, |(idx, val, msg)| {
        val.pub_key.verify(msg, &commit.signatures[*idx].signature)
    }) {
        let idx = to_check[bad].0;
        return Err(format!(
            "wrong signature (#{idx}): {}",
            hex::encode_upper(&commit.signatures[idx].signature)
        ));
    }
    outcome.unwrap_or_else(|| {
        Err(format!(
            "invalid commit -- insufficient voting power: got {tallied}, needed more than {needed}"
        ))
    })
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
        return Err(format!(
            "Invalid commit -- wrong set size: {} vs {}",
            vals.validators.len(),
            commit.signatures.len()
        ));
    }
    if height != commit.height {
        return Err(format!(
            "Invalid commit -- wrong height: {height} vs {}",
            commit.height
        ));
    }
    let cbid = commit.block_id.clone().unwrap_or_default();
    if block_id.hash != cbid.hash || part_set_header(Some(block_id)) != part_set_header(Some(&cbid))
    {
        return Err(format!(
            "invalid commit -- wrong block ID: want {}, got {}",
            block_id_string(block_id),
            block_id_string(&cbid)
        ));
    }
    let needed = vals.total_voting_power * 2 / 3;
    verify_commit_sigs(chain_id, vals, commit, needed, true)
}

/// `BlockID.String()`: `hash:total:fingerprint(part set hash)`.
fn block_id_string(b: &tm::BlockId) -> String {
    let psh = part_set_header(Some(b));
    format!(
        "{}:{}:{}",
        hex::encode_upper(&b.hash),
        psh.total,
        hex::encode_upper(&psh.hash[..psh.hash.len().min(6)])
    )
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
        .ok_or("int64 overflow while calculating voting power needed. please provide smaller trustLevel numerator")?;
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

/// `time.Time.String()` of a UTC time in nanoseconds.
fn go_time(n: Nanos) -> String {
    sdk::time::go_time_string(&sdk::time::from_nanos(n))
}

/// `ErrOldHeaderExpired`.
fn old_header_expired(trusted: &Trusted, p: &Params) -> String {
    format!(
        "old header has expired at {} (now: {})",
        go_time(trusted.time + p.trusting_period),
        go_time(p.now)
    )
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
    validate_signed_header(Some(sh), &trusted.chain_id)
        .map_err(|e| format!("untrustedHeader.ValidateBasic failed: {e}"))?;
    let h = sh.header.as_ref().ok_or("missing header")?;
    if h.height <= trusted.height {
        return Err(format!(
            "expected new header height {} to be greater than one of old header {}",
            h.height, trusted.height
        ));
    }
    let t = ts_nanos(h.time.as_ref());
    if t <= trusted.time {
        return Err(format!(
            "expected new header time {} to be after old header time {}",
            go_time(t),
            go_time(trusted.time)
        ));
    }
    if t >= p.now + p.max_clock_drift {
        return Err(format!(
            "new header has a time from the future {} (now: {}; max clock drift: {})",
            go_time(t),
            go_time(p.now),
            sdk::time::go_duration_string(p.max_clock_drift)
        ));
    }
    let vals_hash = vals.hash();
    if h.validators_hash != vals_hash {
        return Err(format!(
            "expected new header validators ({}) to match those that were supplied ({}) at height {}",
            hex::encode_upper(&h.validators_hash),
            hex::encode_upper(&vals_hash),
            h.height
        ));
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
            return Err(old_header_expired(trusted, p));
        }
        verify_new_header_and_vals(untrusted, untrusted_vals, trusted, p)
            .map_err(|e| format!("invalid header: {e}"))?;
        verify_commit_light_trusting(
            &trusted.chain_id,
            trusted_vals,
            commit,
            p.trust_numerator,
            p.trust_denominator,
        )
        .map_err(|e| {
            // `ErrNotEnoughVotingPowerSigned` -> `ErrNewValSetCantBeTrusted`.
            if e.starts_with("invalid commit -- insufficient voting power") {
                format!("cant trust new val set: {e}")
            } else {
                e
            }
        })?;
    } else {
        // VerifyAdjacent.
        if header_expired(trusted, p) {
            return Err(old_header_expired(trusted, p));
        }
        verify_new_header_and_vals(untrusted, untrusted_vals, trusted, p)
            .map_err(|e| format!("invalid header: {e}"))?;
        if h.validators_hash != trusted.next_validators_hash {
            return Err(format!(
                "expected old header next validators ({}) to match those from new header ({})",
                hex::encode_upper(&trusted.next_validators_hash),
                hex::encode_upper(&h.validators_hash)
            ));
        }
    }
    verify_commit_light(
        &trusted.chain_id,
        untrusted_vals,
        &commit.block_id.clone().unwrap_or_default(),
        h.height,
        commit,
    )
    .map_err(|e| format!("invalid header: {e}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(flag: i32) -> tm::CommitSig {
        tm::CommitSig {
            block_id_flag: flag,
            validator_address: vec![1; 20],
            timestamp: None,
            signature: vec![2; 64],
        }
    }

    /// `n` ed25519 validators of power 10 (seeded keys) and a commit they
    /// signed in set order, with the signatures at `bad` corrupted.
    fn signed(n: u8, bad: &[usize]) -> (ValidatorSet, tm::Commit) {
        let keys: Vec<_> = (0..n)
            .map(|i| crypto::ed25519::private_key_from_seed(&[i + 1; 32]))
            .collect();
        let validators: Vec<Validator> = keys
            .iter()
            .map(|k| {
                let pk = crypto::ed25519::public_key_from_private_key(k).unwrap();
                Validator {
                    address: crypto::ed25519::address(&pk).to_vec(),
                    pub_key: PubKey::Ed25519(pk),
                    voting_power: 10,
                    proposer_priority: 0,
                }
            })
            .collect();
        let mut commit = tm::Commit {
            height: 5,
            round: 0,
            block_id: Some(tm::BlockId {
                hash: vec![3; 32],
                part_set_header: Some(tm::PartSetHeader {
                    total: 1,
                    hash: vec![4; 32],
                }),
            }),
            signatures: validators
                .iter()
                .map(|v| tm::CommitSig {
                    block_id_flag: 2,
                    validator_address: v.address.clone(),
                    timestamp: None,
                    signature: vec![0; 64],
                })
                .collect(),
        };
        for (i, k) in keys.iter().enumerate() {
            let msg = vote_sign_bytes("c", &commit, i);
            let mut sig = crypto::ed25519::sign(k, &msg).unwrap().to_vec();
            if bad.contains(&i) {
                sig[0] ^= 1;
            }
            commit.signatures[i].signature = sig;
        }
        let set = ValidatorSet {
            proposer: validators[0].clone(),
            total_voting_power: 10 * i64::from(n),
            validators,
        };
        (set, commit)
    }

    /// celestia-core v1's `verifyCommitSingle` (celestia-app v3.13.0):
    /// signatures are checked in order until more than `needed` power is
    /// tallied: one after that point is never checked; the first bad one
    /// before it is reported; a structural error after a bad signature
    /// loses to it, and one before it wins.
    #[test]
    fn single_verification_checks_signatures_in_order() {
        let prev = sdk::version::app_version();
        // App version 1 runs as itself (the cosmos-sdk v0.46 driver).
        sdk::version::set_app_version(1);
        let needed = 40 * 10 * 2 / 3;
        let (vals, c) = signed(40, &[]);
        assert!(verify_commit_sigs("c", &vals, &c, needed, true).is_ok());
        // 27 signatures pass 266; the 36th is never checked.
        let (vals, c) = signed(40, &[35]);
        assert!(verify_commit_sigs("c", &vals, &c, needed, true).is_ok());
        let (vals, c) = signed(40, &[20, 5]);
        let err = verify_commit_sigs("c", &vals, &c, needed, true).unwrap_err();
        let sig = hex::encode_upper(&c.signatures[5].signature);
        assert_eq!(err, format!("wrong signature (#5): {sig}"));
        // by address: a double vote at 12 after a bad signature at 4
        let (vals, mut c) = signed(40, &[4]);
        c.signatures[12].validator_address = c.signatures[2].validator_address.clone();
        let err = verify_commit_sigs("c", &vals, &c, needed, false).unwrap_err();
        assert!(err.starts_with("wrong signature (#4)"), "{err}");
        // ... and before one at 20
        let (vals, mut c) = signed(40, &[20]);
        c.signatures[12].validator_address = c.signatures[2].validator_address.clone();
        let err = verify_commit_sigs("c", &vals, &c, needed, false).unwrap_err();
        let double = vals.validators[2].go_string();
        assert_eq!(err, format!("double vote from {double} (2 and 12)"));
        // not enough power: every counted signature is good
        let (vals, c) = signed(40, &[]);
        let err = verify_commit_sigs("c", &vals, &c, 400, true).unwrap_err();
        assert!(err.contains("insufficient voting power: got 400"), "{err}");
        sdk::version::set_app_version(prev);
    }

    /// celestia-core v0.38 on (celestia-app v4.1.0 on, v10 included)
    /// `verifyCommitBatch`: a structural error ends the loop before any
    /// signature is checked, the tally is checked next, and a bad signature
    /// is reported as `%X` of its `CommitSig.String()`.
    #[test]
    fn batch_verification_reports_structure_then_power_then_signatures() {
        let prev = sdk::version::app_version();
        sdk::version::set_app_version(10);
        let needed = 40 * 10 * 2 / 3;
        let (vals, c) = signed(40, &[]);
        assert!(verify_commit_sigs("c", &vals, &c, needed, true).is_ok());
        // 27 signatures pass 266; the 36th is never checked.
        let (vals, c) = signed(40, &[35]);
        assert!(verify_commit_sigs("c", &vals, &c, needed, true).is_ok());
        let (vals, c) = signed(40, &[20, 5]);
        let err = verify_commit_sigs("c", &vals, &c, needed, true).unwrap_err();
        let sig = &c.signatures[5];
        let want = format!(
            "CommitSig{{{} by {} on 2 @ 0001-01-01T00:00:00.000Z}}",
            hex::encode_upper(&sig.signature[..6]),
            hex::encode_upper(&sig.validator_address[..6])
        );
        assert_eq!(
            err,
            format!("wrong signature (#5): {}", hex::encode_upper(want))
        );
        // by address: a double vote at 12 wins over a bad signature at 4
        let (vals, mut c) = signed(40, &[4]);
        c.signatures[12].validator_address = c.signatures[2].validator_address.clone();
        let err = verify_commit_sigs("c", &vals, &c, needed, false).unwrap_err();
        let double = vals.validators[2].go_string();
        assert_eq!(err, format!("double vote from {double} (2 and 12)"));
        // not enough power wins over a bad signature
        let (vals, c) = signed(40, &[0]);
        let err = verify_commit_sigs("c", &vals, &c, 400, true).unwrap_err();
        assert!(err.contains("insufficient voting power: got 400"), "{err}");
        // by index: v8 on check the address; v4 to v6 verify the indexed
        // validator's key
        let (vals, mut c) = signed(40, &[]);
        c.signatures.swap(0, 1);
        let err = verify_commit_sigs("c", &vals, &c, needed, true).unwrap_err();
        assert!(
            err.starts_with("validator address mismatch at index 0"),
            "{err}"
        );
        sdk::version::set_app_version(6);
        let err = verify_commit_sigs("c", &vals, &c, needed, true).unwrap_err();
        assert!(err.starts_with("wrong signature (#0): "), "{err}");
        // ed25519 `BatchVerifier.Add` rejects a signature that is not 64
        // bytes before the tally.
        sdk::version::set_app_version(10);
        let (vals, mut c) = signed(40, &[]);
        c.signatures[3].signature.truncate(63);
        let err = verify_commit_sigs("c", &vals, &c, 400, true).unwrap_err();
        assert_eq!(err, "invalid signature");
        // `shouldBatchVerify` needs every key of one type: a mixed set takes
        // the single path, where a bad signature at 4 beats a double vote
        // at 12.
        let (mut vals, mut c) = signed(40, &[4]);
        vals.validators[39].pub_key = PubKey::Secp256k1([2; 33]);
        c.signatures[12].validator_address = c.signatures[2].validator_address.clone();
        let err = verify_commit_sigs("c", &vals, &c, needed, false).unwrap_err();
        let sig = hex::encode_upper(&c.signatures[4].signature);
        assert_eq!(err, format!("wrong signature (#4): {sig}"));
        sdk::version::set_app_version(prev);
    }

    /// Go's `BlockIDFlag(csp.BlockIdFlag)` keeps the low byte of the
    /// protobuf `int32` (security.md finding 73): `258` and `-254` are
    /// `Commit` signatures (verified and counted), `257` is `Absent`.
    #[test]
    fn block_id_flag_is_truncated_to_a_byte_like_go() {
        assert!(validate_commit_sig(&sig(258)).is_ok());
        assert!(validate_commit_sig(&sig(-254)).is_ok());
        assert_eq!(
            validate_commit_sig(&sig(260)).unwrap_err(),
            "unknown BlockIDFlag: 4"
        );
        let absent = tm::CommitSig {
            block_id_flag: 257,
            ..Default::default()
        };
        assert!(validate_commit_sig(&absent).is_ok());
        let commit = |flag| tm::Commit {
            height: 5,
            round: 0,
            block_id: Some(tm::BlockId {
                hash: vec![3; 32],
                part_set_header: Some(tm::PartSetHeader {
                    total: 1,
                    hash: vec![4; 32],
                }),
            }),
            signatures: vec![sig(flag)],
        };
        assert_eq!(
            vote_sign_bytes("c", &commit(2), 0),
            vote_sign_bytes("c", &commit(2 + 256), 0)
        );
        assert_eq!(
            vote_sign_bytes("c", &commit(i32::MIN + 2), 0),
            vote_sign_bytes("c", &commit(2), 0)
        );
    }
}
