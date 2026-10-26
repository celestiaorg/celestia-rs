//! 07-tendermint light client (`modules/light-clients/07-tendermint`).
//!
//! Client-store layout (under `clients/<client-id>/`): `clientState`,
//! `consensusStates/<h>`, `consensusStates/<h>/processedTime` (BE u64 ns),
//! `consensusStates/<h>/processedHeight` (`"<rev>-<h>"`), and
//! `iterateConsensusStates || BE(rev) || BE(h)` → consensus-state key.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::tendermint::types as tm;
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::commitment::v1::{MerklePath, MerkleRoot};
use celestia_proto::ibc::lightclients::tendermint::v1::{
    ClientState, ConsensusState, Fraction, Header, Misbehaviour,
};
use pbjson_types::{Duration, Timestamp};
use prost::Message;
use sdk::{Context, Result, SdkError};

use super::light::{self, Nanos, ts_nanos};
use crate::client::{self, Status};
use crate::errors::{client as cerr, commitment as commerr, ibc as ierr, tendermint as tmerr};
use crate::height;

/// `KeyIterateConsensusStatePrefix`.
pub const KEY_ITERATE_CONSENSUS_STATE_PREFIX: &str = "iterateConsensusStates";
/// `SentinelRoot`.
pub const SENTINEL_ROOT: &str = "sentinel_root";
/// `FrozenHeight` (0-1).
pub const FROZEN_HEIGHT: Height = Height {
    revision_number: 0,
    revision_height: 1,
};

/// A client message.
#[derive(Debug, Clone)]
pub enum ClientMessage {
    /// A header.
    Header(Box<Header>),
    /// Misbehaviour (two conflicting headers).
    Misbehaviour(Box<Misbehaviour>),
}

// ---------------------------------------------------------------------------
// gogoproto normalization (non-nullable fields, stdtime/stdduration)
// ---------------------------------------------------------------------------

/// Nanoseconds of a duration (Go `time.Duration`).
#[must_use]
pub fn dur_nanos(d: Option<&Duration>) -> i128 {
    d.map_or(0, |d| {
        i128::from(d.seconds) * 1_000_000_000 + i128::from(d.nanos)
    })
}

fn norm_duration(d: Option<Duration>) -> Duration {
    let n = dur_nanos(d.as_ref());
    #[allow(clippy::cast_possible_truncation)]
    Duration {
        seconds: (n / 1_000_000_000) as i64,
        nanos: (n % 1_000_000_000) as i32,
    }
}

/// A Go `time.Time` (nanoseconds) back to a normalized protobuf timestamp.
#[must_use]
pub fn nanos_ts(n: Nanos) -> Timestamp {
    #[allow(clippy::cast_possible_truncation)]
    Timestamp {
        seconds: n.div_euclid(1_000_000_000) as i64,
        nanos: n.rem_euclid(1_000_000_000) as i32,
    }
}

fn norm_ts(t: Option<Timestamp>) -> Timestamp {
    nanos_ts(ts_nanos(t.as_ref()))
}

/// Fills the gogoproto non-nullable fields so the encoding matches Go.
#[must_use]
pub fn normalize_client_state(mut cs: ClientState) -> ClientState {
    cs.trust_level = Some(cs.trust_level.unwrap_or_default());
    cs.trusting_period = Some(norm_duration(cs.trusting_period));
    cs.unbonding_period = Some(norm_duration(cs.unbonding_period));
    cs.max_clock_drift = Some(norm_duration(cs.max_clock_drift));
    cs.frozen_height = Some(cs.frozen_height.unwrap_or_default());
    cs.latest_height = Some(cs.latest_height.unwrap_or_default());
    cs
}

/// Fills the non-nullable fields of a consensus state.
#[must_use]
pub fn normalize_consensus_state(mut c: ConsensusState) -> ConsensusState {
    c.timestamp = Some(norm_ts(c.timestamp));
    c.root = Some(c.root.unwrap_or_default());
    c
}

/// `MarshalInterface(clientState)`.
#[must_use]
pub fn client_state_any(cs: &ClientState) -> Any {
    Any::pack(&normalize_client_state(cs.clone()))
}

/// `MarshalInterface(consensusState)`.
#[must_use]
pub fn consensus_state_any(c: &ConsensusState) -> Any {
    Any::pack(&normalize_consensus_state(c.clone()))
}

fn latest(cs: &ClientState) -> Height {
    cs.latest_height.unwrap_or_default()
}

fn frozen(cs: &ClientState) -> Height {
    cs.frozen_height.unwrap_or_default()
}

fn trusting_period(cs: &ClientState) -> i128 {
    dur_nanos(cs.trusting_period.as_ref())
}

/// `ClientState.IsExpired(latestTimestamp, now)`.
#[must_use]
pub fn is_expired(cs: &ClientState, latest_timestamp: Nanos, now: Nanos) -> bool {
    latest_timestamp + trusting_period(cs) <= now
}

/// Current block time.
#[must_use]
pub fn block_time(ctx: &Context<'_>) -> Nanos {
    ts_nanos(Some(&ctx.header.time))
}

// ---------------------------------------------------------------------------
// Client store
// ---------------------------------------------------------------------------

fn ckey(client_id: &str, suffix: &[u8]) -> Vec<u8> {
    [
        crate::host::client_store_prefix(client_id).as_bytes(),
        suffix,
    ]
    .concat()
}

fn consensus_state_key(h: &Height) -> Vec<u8> {
    format!(
        "{}/{}",
        crate::host::KEY_CONSENSUS_STATE_PREFIX,
        height::to_string(h)
    )
    .into_bytes()
}

fn processed_time_key(h: &Height) -> Vec<u8> {
    [consensus_state_key(h).as_slice(), b"/processedTime"].concat()
}

fn processed_height_key(h: &Height) -> Vec<u8> {
    [consensus_state_key(h).as_slice(), b"/processedHeight"].concat()
}

fn big_endian_height(h: &Height) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h.revision_number.to_be_bytes());
    out[8..].copy_from_slice(&h.revision_height.to_be_bytes());
    out
}

fn iteration_key(h: &Height) -> Vec<u8> {
    [
        KEY_ITERATE_CONSENSUS_STATE_PREFIX.as_bytes(),
        &big_endian_height(h),
    ]
    .concat()
}

fn height_from_iteration_key(k: &[u8]) -> Height {
    let b = &k[KEY_ITERATE_CONSENSUS_STATE_PREFIX.len()..];
    let mut rn = [0u8; 8];
    let mut rh = [0u8; 8];
    rn.copy_from_slice(&b[..8]);
    rh.copy_from_slice(&b[8..16]);
    Height {
        revision_number: u64::from_be_bytes(rn),
        revision_height: u64::from_be_bytes(rh),
    }
}

/// `setClientState`.
pub fn set_client_state(ctx: &mut Context<'_>, client_id: &str, cs: &ClientState) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        ckey(client_id, crate::host::KEY_CLIENT_STATE.as_bytes()),
        client_state_any(cs).encode_to_vec(),
    )
}

/// `setConsensusState`.
pub fn set_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    c: &ConsensusState,
    h: &Height,
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        ckey(client_id, &consensus_state_key(h)),
        consensus_state_any(c).encode_to_vec(),
    )
}

fn decode_stored_consensus_state(bz: &[u8]) -> Result<ConsensusState> {
    let any = Any::decode(bz).map_err(crate::errors::must_unmarshal)?;
    match any.unpack::<ConsensusState>() {
        Some(Ok(c)) => Ok(c),
        // MustUnmarshalConsensusState + type assertion: Go panics.
        _ => Err(sdk::error::go_panic(
            "stored consensus state is not 07-tendermint",
        )),
    }
}

/// `GetConsensusState`.
pub fn get_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<Option<ConsensusState>> {
    let bz = ctx
        .get(crate::STORE_KEY, &ckey(client_id, &consensus_state_key(h)))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    decode_stored_consensus_state(&bz).map(Some)
}

/// `getTmConsensusState` (decode failures are "not found").
fn get_tm_consensus_state_by_key(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs_key: &[u8],
) -> Result<Option<ConsensusState>> {
    let bz = ctx
        .get(crate::STORE_KEY, &ckey(client_id, cs_key))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    let Ok(any) = Any::decode(bz.as_slice()) else {
        return Ok(None);
    };
    Ok(any
        .unpack::<ConsensusState>()
        .and_then(core::result::Result::ok))
}

/// `GetProcessedTime`.
pub fn get_processed_time(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<Option<u64>> {
    let bz = ctx
        .get(crate::STORE_KEY, &ckey(client_id, &processed_time_key(h)))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::be_u64(&bz).unwrap_or_default()))
}

/// `GetProcessedHeight`.
pub fn get_processed_height(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<Option<Height>> {
    let bz = ctx
        .get(crate::STORE_KEY, &ckey(client_id, &processed_height_key(h)))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    Ok(height::parse(&String::from_utf8_lossy(&bz)).ok())
}

fn set_consensus_metadata_with_values(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
    processed_height: &Height,
    processed_time: u64,
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        ckey(client_id, &processed_time_key(h)),
        processed_time.to_be_bytes().to_vec(),
    )?;
    ctx.set(
        crate::STORE_KEY,
        ckey(client_id, &processed_height_key(h)),
        height::to_string(processed_height).into_bytes(),
    )?;
    ctx.set(
        crate::STORE_KEY,
        ckey(client_id, &iteration_key(h)),
        consensus_state_key(h),
    )
}

/// `setConsensusMetadata`: self height and block time.
fn set_consensus_metadata(ctx: &mut Context<'_>, client_id: &str, h: &Height) -> Result<()> {
    let self_height = client::get_self_height(ctx)?;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let now = block_time(ctx) as u64;
    set_consensus_metadata_with_values(ctx, client_id, h, &self_height, now)
}

fn delete_consensus_state_and_metadata(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<()> {
    ctx.delete(crate::STORE_KEY, &ckey(client_id, &consensus_state_key(h)))?;
    ctx.delete(crate::STORE_KEY, &ckey(client_id, &processed_time_key(h)))?;
    ctx.delete(crate::STORE_KEY, &ckey(client_id, &processed_height_key(h)))?;
    ctx.delete(crate::STORE_KEY, &ckey(client_id, &iteration_key(h)))
}

/// `GetPreviousConsensusState`: highest consensus state below `h`.
fn get_previous_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<Option<ConsensusState>> {
    let start = ckey(client_id, KEY_ITERATE_CONSENSUS_STATE_PREFIX.as_bytes());
    let end = ckey(client_id, &iteration_key(h));
    let mut it = ctx.reverse_iterator(crate::STORE_KEY, Some(&start), Some(&end))?;
    let Some((_, cs_key)) = it.next(ctx)? else {
        return Ok(None);
    };
    get_tm_consensus_state_by_key(ctx, client_id, &cs_key)
}

/// `GetNextConsensusState`: lowest consensus state above `h`.
fn get_next_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    h: &Height,
) -> Result<Option<ConsensusState>> {
    let prefix = ckey(client_id, KEY_ITERATE_CONSENSUS_STATE_PREFIX.as_bytes());
    let start = ckey(client_id, &iteration_key(h));
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.iterator(crate::STORE_KEY, Some(&start), end.as_deref())?;
    let Some((_, mut cs_key)) = it.next(ctx)? else {
        return Ok(None);
    };
    if cs_key == consensus_state_key(h) {
        match it.next(ctx)? {
            Some((_, v)) => cs_key = v,
            None => return Ok(None),
        }
    }
    get_tm_consensus_state_by_key(ctx, client_id, &cs_key)
}

// ---------------------------------------------------------------------------
// ClientState methods
// ---------------------------------------------------------------------------

/// `ClientState.Validate`.
pub fn validate(cs: &ClientState) -> Result<()> {
    if cs.chain_id.trim().is_empty() {
        return Err(tmerr::invalid_chain_id("chain id cannot be empty string"));
    }
    if gostr::go_len(&cs.chain_id) > light::MAX_CHAIN_ID_LEN {
        return Err(tmerr::invalid_chain_id(format!(
            "chainID is too long; got: {}, max: {}",
            gostr::go_len(&cs.chain_id),
            light::MAX_CHAIN_ID_LEN
        )));
    }
    let tl = cs.trust_level.unwrap_or_default();
    light::validate_trust_level(tl.numerator, tl.denominator)
        .map_err(crate::errors::unregistered)?;
    if trusting_period(cs) <= 0 {
        return Err(tmerr::invalid_trusting_period(
            "trusting period must be greater than zero",
        ));
    }
    if dur_nanos(cs.unbonding_period.as_ref()) <= 0 {
        return Err(tmerr::invalid_unbonding_period(
            "unbonding period must be greater than zero",
        ));
    }
    if dur_nanos(cs.max_clock_drift.as_ref()) <= 0 {
        return Err(tmerr::invalid_max_clock_drift(
            "max clock drift must be greater than zero",
        ));
    }
    let lh = latest(cs);
    if lh.revision_number != client::parse_chain_id(&cs.chain_id)? {
        return Err(tmerr::invalid_header_height(
            "latest height revision number must match chain id revision number",
        ));
    }
    if lh.revision_height == 0 {
        return Err(tmerr::invalid_header_height(
            "tendermint client's latest height revision height cannot be zero",
        ));
    }
    if trusting_period(cs) >= dur_nanos(cs.unbonding_period.as_ref()) {
        return Err(tmerr::invalid_trusting_period(
            "trusting period should be < unbonding period",
        ));
    }
    // ProofSpecs == nil: proto decoding yields nil for an absent field.
    if cs.proof_specs.is_empty() {
        return Err(tmerr::invalid_proof_specs(
            "proof specs cannot be nil for tm client",
        ));
    }
    for (i, k) in cs.upgrade_path.iter().enumerate() {
        if k.trim().is_empty() {
            return Err(cerr::invalid_client(format!(
                "key in upgrade path at index {i} cannot be empty"
            )));
        }
    }
    Ok(())
}

/// `ConsensusState.ValidateBasic`.
pub fn validate_consensus_state(c: &ConsensusState) -> Result<()> {
    if c.root.as_ref().is_none_or(|r| r.hash.is_empty()) {
        return Err(cerr::invalid_consensus("root cannot be empty"));
    }
    if !c.next_validators_hash.is_empty() && c.next_validators_hash.len() != 32 {
        // `errorsmod.Wrap(cmttypes.ValidateHash(..), "next validators hash is invalid")`.
        return Err(crate::errors::unregistered(format!(
            "next validators hash is invalid: expected size to be 32 bytes, got {} bytes",
            c.next_validators_hash.len()
        )));
    }
    if ts_nanos(c.timestamp.as_ref()).div_euclid(1_000_000_000) <= 0 {
        return Err(cerr::invalid_consensus(
            "timestamp must be a positive Unix time",
        ));
    }
    Ok(())
}

/// `ClientState.ZeroCustomFields`.
#[must_use]
pub fn zero_custom_fields(cs: &ClientState) -> ClientState {
    ClientState {
        chain_id: cs.chain_id.clone(),
        unbonding_period: cs.unbonding_period,
        latest_height: cs.latest_height,
        proof_specs: cs.proof_specs.clone(),
        upgrade_path: cs.upgrade_path.clone(),
        ..ClientState::default()
    }
}

/// `ClientState.Status`.
pub fn status(ctx: &mut Context<'_>, client_id: &str, cs: &ClientState) -> Result<Status> {
    if !height::is_zero(&frozen(cs)) {
        return Ok(Status::Frozen);
    }
    let Some(cons) = get_consensus_state(ctx, client_id, &latest(cs))? else {
        return Ok(Status::Expired);
    };
    if is_expired(cs, ts_nanos(cons.timestamp.as_ref()), block_time(ctx)) {
        return Ok(Status::Expired);
    }
    Ok(Status::Active)
}

/// `ClientState.GetTimestampAtHeight`.
pub fn get_timestamp_at_height(ctx: &mut Context<'_>, client_id: &str, h: &Height) -> Result<u64> {
    let cons = get_consensus_state(ctx, client_id, h)?.ok_or_else(|| {
        cerr::consensus_state_not_found(format!("height ({})", height::to_string(h)))
    })?;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    Ok(ts_nanos(cons.timestamp.as_ref()) as u64)
}

/// `ClientState.Initialize`.
pub fn initialize(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    cons: &Any,
) -> Result<()> {
    let Some(Ok(c)) = cons.unpack::<ConsensusState>() else {
        return Err(cerr::invalid_consensus(
            "invalid initial consensus state. expected type: *types.ConsensusState",
        ));
    };
    set_client_state(ctx, client_id, cs)?;
    set_consensus_state(ctx, client_id, &c, &latest(cs))?;
    set_consensus_metadata(ctx, client_id, &latest(cs))
}

// ---------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------

fn tm_header(h: &Header) -> tm::Header {
    h.signed_header
        .as_ref()
        .and_then(|s| s.header.clone())
        .unwrap_or_default()
}

/// `Header.GetHeight`.
pub fn header_height(h: &Header) -> Result<Height> {
    let th = tm_header(h);
    #[allow(clippy::cast_sign_loss)]
    Ok(Height {
        revision_number: client::parse_chain_id(&th.chain_id)?,
        revision_height: th.height as u64,
    })
}

fn header_time(h: &Header) -> Nanos {
    ts_nanos(tm_header(h).time.as_ref())
}

/// `Header.ConsensusState`.
#[must_use]
pub fn header_consensus_state(h: &Header) -> ConsensusState {
    let th = tm_header(h);
    ConsensusState {
        timestamp: Some(nanos_ts(ts_nanos(th.time.as_ref()))),
        root: Some(MerkleRoot { hash: th.app_hash }),
        next_validators_hash: th.next_validators_hash,
    }
}

/// `Header.ValidateBasic`.
pub fn validate_header_basic(h: &Header) -> Result<()> {
    let Some(sh) = h.signed_header.as_ref() else {
        return Err(cerr::invalid_header(
            "tendermint signed header cannot be nil",
        ));
    };
    let Some(th) = sh.header.as_ref() else {
        return Err(cerr::invalid_header("tendermint header cannot be nil"));
    };
    light::signed_header_from_proto(Some(sh)).map_err(|e| {
        crate::errors::unregistered(format!("header is not a tendermint header: {e}"))
    })?;
    light::validate_signed_header(Some(sh), &th.chain_id)
        .map_err(|e| crate::errors::unregistered(format!("header failed basic validation: {e}")))?;
    let hh = header_height(h)?;
    let trusted = h.trusted_height.unwrap_or_default();
    if height::gte(&trusted, &hh) {
        return Err(tmerr::invalid_header_height(format!(
            "TrustedHeight {} must be less than header height {}",
            height::go_fmt_d(&trusted),
            height::go_fmt_d(&hh)
        )));
    }
    if h.validator_set.is_none() {
        return Err(cerr::invalid_header("validator set is nil"));
    }
    let vals = light::ValidatorSet::from_proto(h.validator_set.as_ref()).map_err(|e| {
        crate::errors::unregistered(format!(
            "validator set is not tendermint validator set: {e}"
        ))
    })?;
    if th.validators_hash != vals.hash() {
        return Err(cerr::invalid_header("validator set does not match hash"));
    }
    Ok(())
}

/// `Misbehaviour.ValidateBasic`.
pub fn validate_misbehaviour_basic(m: &Misbehaviour) -> Result<()> {
    let (Some(h1), Some(h2)) = (m.header_1.as_ref(), m.header_2.as_ref()) else {
        return Err(tmerr::invalid_header(if m.header_1.is_none() {
            "misbehaviour Header1 cannot be nil"
        } else {
            "misbehaviour Header2 cannot be nil"
        }));
    };
    if h1.trusted_height.unwrap_or_default().revision_height == 0 {
        return Err(tmerr::invalid_header_height(
            "misbehaviour Header1 cannot have zero revision height",
        ));
    }
    if h2.trusted_height.unwrap_or_default().revision_height == 0 {
        return Err(tmerr::invalid_header_height(
            "misbehaviour Header2 cannot have zero revision height",
        ));
    }
    if h1.trusted_validators.is_none() {
        return Err(tmerr::invalid_validator_set(
            "trusted validator set in Header1 cannot be empty",
        ));
    }
    if h2.trusted_validators.is_none() {
        return Err(tmerr::invalid_validator_set(
            "trusted validator set in Header2 cannot be empty",
        ));
    }
    // `Header1.Header.ChainID` dereferences the signed header (Go panics on nil).
    let (Some(sh1), Some(sh2)) = (h1.signed_header.as_ref(), h2.signed_header.as_ref()) else {
        return Err(sdk::error::go_panic("nil signed header in misbehaviour"));
    };
    let (Some(th1), Some(th2)) = (sh1.header.as_ref(), sh2.header.as_ref()) else {
        return Err(sdk::error::go_panic("nil header in misbehaviour"));
    };
    if th1.chain_id != th2.chain_id {
        return Err(cerr::invalid_misbehaviour(
            "headers must have identical chainIDs",
        ));
    }
    #[allow(deprecated)]
    crate::host::client_identifier_validator(&m.client_id)
        .map_err(|e| e.wrap("misbehaviour client ID is invalid"))?;
    validate_header_basic(h1).map_err(|e| {
        cerr::invalid_misbehaviour(format!("header 1 failed validation: {}", e.log))
    })?;
    validate_header_basic(h2).map_err(|e| {
        cerr::invalid_misbehaviour(format!("header 2 failed validation: {}", e.log))
    })?;
    if height::lt(&header_height(h1)?, &header_height(h2)?) {
        return Err(cerr::invalid_misbehaviour(
            "Header1 height is less than Header2 height",
        ));
    }
    for (th, h) in [(th1, h1), (th2, h2)] {
        let commit = h
            .signed_header
            .as_ref()
            .and_then(|s| s.commit.clone())
            .ok_or_else(|| sdk::error::go_panic("nil commit"))?;
        let bid = commit.block_id.clone().unwrap_or_default();
        light::validate_commit(&commit).map_err(|e| {
            crate::errors::unregistered(format!("commit is not tendermint commit type: {e}"))
        })?;
        let vals = light::ValidatorSet::from_proto(h.validator_set.as_ref()).map_err(|e| {
            crate::errors::unregistered(format!(
                "validator set is not tendermint validator set type: {e}"
            ))
        })?;
        if light::verify_commit_light(&th.chain_id, &vals, &bid, commit.height, &commit).is_err() {
            return Err(cerr::invalid_misbehaviour(
                "validator set did not commit to header",
            ));
        }
    }
    Ok(())
}

/// `checkTrustedHeader`.
fn check_trusted_header(h: &Header, cons: &ConsensusState) -> Result<()> {
    let tvals = light::ValidatorSet::from_proto(h.trusted_validators.as_ref()).map_err(|e| {
        crate::errors::unregistered(format!(
            "trusted validator set in not tendermint validator set type: {e}"
        ))
    })?;
    let got = tvals.hash();
    if cons.next_validators_hash != got {
        let trusted = h
            .trusted_validators
            .as_ref()
            .map(validator_set_text)
            .unwrap_or_default();
        return Err(tmerr::invalid_validator_set(format!(
            "trusted validators {trusted}, does not hash to latest trusted validators. Expected: {}, got: {}",
            hex::encode_upper(&cons.next_validators_hash),
            hex::encode_upper(got)
        )));
    }
    Ok(())
}

fn quoted_bytes(bytes: &[u8]) -> String {
    x_ante::gogotext::quote(&gostr::from_go_bytes(bytes))
}

fn public_key_text(
    key: &celestia_proto::celestia_core_proto::tendermint::crypto::PublicKey,
) -> String {
    use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;

    match key.sum.as_ref() {
        Some(Sum::Ed25519(bytes)) => format!("ed25519:{} ", quoted_bytes(bytes)),
        Some(Sum::Secp256k1(bytes)) => format!("secp256k1:{} ", quoted_bytes(bytes)),
        None => String::new(),
    }
}

fn validator_text(validator: &tm::Validator) -> String {
    use std::fmt::Write as _;

    let mut text = String::new();
    if !validator.address.is_empty() {
        let _ = write!(text, "address:{} ", quoted_bytes(&validator.address));
    }
    if let Some(key) = validator.pub_key.as_ref() {
        let _ = write!(text, "pub_key:<{}> ", public_key_text(key));
    }
    if validator.voting_power != 0 {
        let _ = write!(text, "voting_power:{} ", validator.voting_power);
    }
    if validator.proposer_priority != 0 {
        let _ = write!(text, "proposer_priority:{} ", validator.proposer_priority);
    }
    text
}

/// gogoproto `ValidatorSet.String()` (`proto.CompactTextString`).
fn validator_set_text(validators: &tm::ValidatorSet) -> String {
    use std::fmt::Write as _;

    let mut text = String::new();
    for validator in &validators.validators {
        let _ = write!(text, "validators:<{}> ", validator_text(validator));
    }
    if let Some(proposer) = validators.proposer.as_ref() {
        let _ = write!(text, "proposer:<{}> ", validator_text(proposer));
    }
    if validators.total_voting_power != 0 {
        let _ = write!(
            text,
            "total_voting_power:{} ",
            validators.total_voting_power
        );
    }
    text
}

fn trust_level(cs: &ClientState) -> Fraction {
    cs.trust_level.unwrap_or_default()
}

/// `verifyHeader`.
fn verify_header(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    h: &Header,
) -> Result<()> {
    let now = block_time(ctx);
    let trusted_height = h.trusted_height.unwrap_or_default();
    let cons = get_consensus_state(ctx, client_id, &trusted_height)?.ok_or_else(|| {
        cerr::consensus_state_not_found(format!(
            "could not get trusted consensus state from clientStore for Header at TrustedHeight: {}",
            height::to_string(&trusted_height)
        ))
    })?;
    check_trusted_header(h, &cons)?;
    let hh = header_height(h)?;
    if hh.revision_number != trusted_height.revision_number {
        return Err(tmerr::invalid_header_height(format!(
            "header height revision {} does not match trusted header revision {}",
            hh.revision_number, trusted_height.revision_number
        )));
    }
    let unreg = |what: &str, e: String| crate::errors::unregistered(format!("{what}: {e}"));
    let tvals = light::ValidatorSet::from_proto(h.trusted_validators.as_ref()).map_err(|e| {
        unreg(
            "trusted validator set in not tendermint validator set type",
            e,
        )
    })?;
    light::signed_header_from_proto(h.signed_header.as_ref())
        .map_err(|e| unreg("signed header in not tendermint signed header type", e))?;
    let vals = light::ValidatorSet::from_proto(h.validator_set.as_ref())
        .map_err(|e| unreg("validator set in not tendermint validator set type", e))?;
    if height::lte(&hh, &trusted_height) {
        return Err(cerr::invalid_header(format!(
            "header height ≤ consensus state height ({} ≤ {})",
            height::to_string(&hh),
            height::to_string(&trusted_height)
        )));
    }
    #[allow(clippy::cast_possible_wrap)]
    let trusted = light::Trusted {
        chain_id: cs.chain_id.clone(),
        height: trusted_height.revision_height as i64,
        time: ts_nanos(cons.timestamp.as_ref()),
        next_validators_hash: cons.next_validators_hash.clone(),
    };
    let tl = trust_level(cs);
    let p = light::Params {
        trusting_period: trusting_period(cs),
        now,
        max_clock_drift: dur_nanos(cs.max_clock_drift.as_ref()),
        trust_numerator: tl.numerator,
        trust_denominator: tl.denominator,
    };
    let sh = h
        .signed_header
        .as_ref()
        .ok_or_else(|| crate::errors::unregistered("nil SignedHeader"))?;
    light::verify(&trusted, &tvals, sh, &vals, &p).map_err(|e| unreg("failed to verify header", e))
}

/// `checkMisbehaviourHeader`.
fn check_misbehaviour_header(
    cs: &ClientState,
    cons: &ConsensusState,
    h: &Header,
    now: Nanos,
) -> Result<()> {
    let tvals = light::ValidatorSet::from_proto(h.trusted_validators.as_ref()).map_err(|e| {
        crate::errors::unregistered(format!(
            "trusted validator set is not tendermint validator set type: {e}"
        ))
    })?;
    let commit = h
        .signed_header
        .as_ref()
        .and_then(|s| s.commit.clone())
        .ok_or_else(|| crate::errors::unregistered("nil Commit"))?;
    light::validate_commit(&commit).map_err(|e| {
        crate::errors::unregistered(format!("commit is not tendermint commit type: {e}"))
    })?;
    check_trusted_header(h, cons)?;
    let elapsed = now - ts_nanos(cons.timestamp.as_ref());
    let trusting_period = trusting_period(cs);
    if elapsed >= trusting_period {
        return Err(tmerr::trusting_period_expired(format!(
            "current timestamp minus the latest consensus state timestamp is greater than or equal to the trusting period ({elapsed} >= {trusting_period})"
        )));
    }
    let mut chain_id = cs.chain_id.clone();
    if client::is_revision_format(&chain_id) {
        chain_id = client::set_revision_number(&chain_id, header_height(h)?.revision_number);
    }
    let tl = trust_level(cs);
    light::verify_commit_light_trusting(&chain_id, &tvals, &commit, tl.numerator, tl.denominator)
        .map_err(|e| {
            cerr::invalid_misbehaviour(format!(
                "validator set in header has too much change from trusted validator set: {e}"
            ))
        })
}

/// `verifyMisbehaviour`.
fn verify_misbehaviour(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    m: &Misbehaviour,
) -> Result<()> {
    let h1 = m.header_1.clone().unwrap_or_default();
    let h2 = m.header_2.clone().unwrap_or_default();
    let th1 = h1.trusted_height.unwrap_or_default();
    let th2 = h2.trusted_height.unwrap_or_default();
    let c1 = get_consensus_state(ctx, client_id, &th1)?.ok_or_else(|| {
        cerr::consensus_state_not_found(format!(
            "could not get trusted consensus state from clientStore for Header1 at TrustedHeight: {}",
            height::to_string(&th1)
        ))
    })?;
    let c2 = get_consensus_state(ctx, client_id, &th2)?.ok_or_else(|| {
        cerr::consensus_state_not_found(format!(
            "could not get trusted consensus state from clientStore for Header2 at TrustedHeight: {}",
            height::to_string(&th2)
        ))
    })?;
    let now = block_time(ctx);
    check_misbehaviour_header(cs, &c1, &h1, now)
        .map_err(|e| e.wrap("verifying Header1 in Misbehaviour failed"))?;
    check_misbehaviour_header(cs, &c2, &h2, now)
        .map_err(|e| e.wrap("verifying Header2 in Misbehaviour failed"))
}

/// `ClientState.VerifyClientMessage`.
pub fn verify_client_message(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    msg: &ClientMessage,
) -> Result<()> {
    match msg {
        ClientMessage::Header(h) => verify_header(ctx, client_id, cs, h),
        ClientMessage::Misbehaviour(m) => verify_misbehaviour(ctx, client_id, cs, m),
    }
}

/// `ClientState.CheckForMisbehaviour`.
pub fn check_for_misbehaviour(
    ctx: &mut Context<'_>,
    client_id: &str,
    msg: &ClientMessage,
) -> Result<bool> {
    match msg {
        ClientMessage::Header(h) => {
            let cons = header_consensus_state(h);
            let hh = header_height(h)?;
            if let Some(existing) = get_consensus_state(ctx, client_id, &hh)? {
                // reflect.DeepEqual on the decoded structs.
                return Ok(normalize_consensus_state(existing) != normalize_consensus_state(cons));
            }
            let t = ts_nanos(cons.timestamp.as_ref());
            let prev = get_previous_consensus_state(ctx, client_id, &hh)?;
            let next = get_next_consensus_state(ctx, client_id, &hh)?;
            if prev.is_some_and(|p| ts_nanos(p.timestamp.as_ref()) >= t) {
                return Ok(true);
            }
            if next.is_some_and(|n| ts_nanos(n.timestamp.as_ref()) <= t) {
                return Ok(true);
            }
            Ok(false)
        }
        ClientMessage::Misbehaviour(m) => {
            let h1 = m.header_1.clone().unwrap_or_default();
            let h2 = m.header_2.clone().unwrap_or_default();
            if header_height(&h1)? == header_height(&h2)? {
                let bid = |h: &Header| {
                    h.signed_header
                        .as_ref()
                        .and_then(|s| s.commit.as_ref())
                        .and_then(|c| c.block_id.clone())
                        .unwrap_or_default()
                };
                let (b1, b2) = (bid(&h1), bid(&h2));
                for b in [&b1, &b2] {
                    let psh_hash = b.part_set_header.clone().unwrap_or_default().hash;
                    let bad = |x: &[u8]| !x.is_empty() && x.len() != 32;
                    if bad(&b.hash) || bad(&psh_hash) {
                        return Ok(false);
                    }
                }
                return Ok(b1.hash != b2.hash);
            }
            Ok(header_time(&h1) <= header_time(&h2))
        }
    }
}

/// `pruneOldestConsensusState`.
fn prune_oldest_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
) -> Result<()> {
    let prefix = ckey(client_id, KEY_ITERATE_CONSENSUS_STATE_PREFIX.as_bytes());
    let mut it = ctx.prefix_iter(crate::STORE_KEY, &prefix)?;
    let mut prune: Option<Height> = None;
    if let Some((k, _)) = it.next(ctx)? {
        let h = height_from_iteration_key(
            &k[prefix.len() - KEY_ITERATE_CONSENSUS_STATE_PREFIX.len()..],
        );
        let cons = get_consensus_state(ctx, client_id, &h)?.ok_or_else(|| {
            sdk::error::go_panic("failed to retrieve consensus state while pruning")
        })?;
        if is_expired(cs, ts_nanos(cons.timestamp.as_ref()), block_time(ctx)) {
            prune = Some(h);
        }
    }
    if let Some(h) = prune {
        delete_consensus_state_and_metadata(ctx, client_id, &h)?;
    }
    Ok(())
}

/// `ClientState.UpdateState`: returns the consensus heights.
pub fn update_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    msg: &ClientMessage,
) -> Result<Vec<Height>> {
    let ClientMessage::Header(h) = msg else {
        return Ok(Vec::new());
    };
    if !matches!(ctx.mode, sdk::ExecMode::Check | sdk::ExecMode::ReCheck) {
        prune_oldest_consensus_state(ctx, client_id, cs)?;
    }
    let hh = header_height(h)?;
    if get_consensus_state(ctx, client_id, &hh)?.is_some() {
        return Ok(vec![hh]);
    }
    let mut cs = cs.clone();
    if height::gt(&hh, &latest(&cs)) {
        cs.latest_height = Some(hh);
    }
    let cons = header_consensus_state(h);
    set_client_state(ctx, client_id, &cs)?;
    set_consensus_state(ctx, client_id, &cons, &hh)?;
    set_consensus_metadata(ctx, client_id, &hh)?;
    Ok(vec![hh])
}

/// `ClientState.UpdateStateOnMisbehaviour`: freezes the client.
pub fn update_state_on_misbehaviour(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
) -> Result<()> {
    let mut cs = cs.clone();
    cs.frozen_height = Some(FROZEN_HEIGHT);
    set_client_state(ctx, client_id, &cs)
}

// ---------------------------------------------------------------------------
// Proof verification
// ---------------------------------------------------------------------------

fn verify_delay_period_passed(
    ctx: &mut Context<'_>,
    store_client: &str,
    proof_height: &Height,
    delay_time: u64,
    delay_block: u64,
) -> Result<()> {
    if delay_time != 0 {
        let processed = get_processed_time(ctx, store_client, proof_height)?.ok_or_else(|| {
            tmerr::processed_time_not_found(format!(
                "processed time not found for height: {}",
                height::to_string(proof_height)
            ))
        })?;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let now = block_time(ctx) as u64;
        let valid = processed.wrapping_add(delay_time);
        if now < valid {
            return Err(tmerr::delay_period_not_passed(format!(
                "cannot verify packet until time: {valid}, current time: {now}"
            )));
        }
    }
    if delay_block != 0 {
        let processed =
            get_processed_height(ctx, store_client, proof_height)?.ok_or_else(|| {
                tmerr::processed_height_not_found(format!(
                    "processed height not found for height: {}",
                    height::to_string(proof_height)
                ))
            })?;
        let current = client::get_self_height(ctx)?;
        let valid = Height {
            revision_number: processed.revision_number,
            revision_height: processed.revision_height.wrapping_add(delay_block),
        };
        if height::lt(&current, &valid) {
            return Err(tmerr::delay_period_not_passed(format!(
                "cannot verify packet until height: {}, current height: {}",
                height::to_string(&valid),
                height::to_string(&current)
            )));
        }
    }
    Ok(())
}

/// `ClientState.VerifyMembership` (`value = None` for `VerifyNonMembership`).
#[allow(clippy::too_many_arguments)]
pub fn verify_membership(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    h: &Height,
    delay_time: u64,
    delay_block: u64,
    proof: &[u8],
    path: &MerklePath,
    value: Option<&[u8]>,
) -> Result<()> {
    if height::lt(&latest(cs), h) {
        // Go formats both heights with `%d`.
        return Err(ierr::invalid_height(format!(
            "client state height < proof height ({} < {}), please ensure the client has been updated",
            height::go_fmt_d(&latest(cs)),
            height::go_fmt_d(h)
        )));
    }
    verify_delay_period_passed(ctx, client_id, h, delay_time, delay_block)?;
    let merkle_proof = crate::commitment::decode_proof(proof)?;
    let cons = get_consensus_state(ctx, client_id, h)?.ok_or_else(|| {
        cerr::consensus_state_not_found(
            "please ensure the proof was constructed against a height that exists on the client",
        )
    })?;
    let root = cons.root.unwrap_or_default().hash;
    match value {
        Some(v) => {
            crate::commitment::verify_membership(&merkle_proof, &cs.proof_specs, &root, path, v)
        }
        None => {
            crate::commitment::verify_non_membership(&merkle_proof, &cs.proof_specs, &root, path)
        }
    }
}

// ---------------------------------------------------------------------------
// Upgrades and recovery
// ---------------------------------------------------------------------------

fn upgrade_path(upgrade_path: &[String], last: &Height, key: &str) -> MerklePath {
    let mut p: Vec<String> = upgrade_path[..upgrade_path.len() - 1].to_vec();
    let last_key = &upgrade_path[upgrade_path.len() - 1];
    p.push(format!("{last_key}/{}/{key}", last.revision_height));
    MerklePath { key_path: p }
}

fn consensus_state_go_type(any: &Any) -> &'static str {
    match any.type_url.as_str() {
        "/ibc.lightclients.tendermint.v1.ConsensusState" => "*tendermint.ConsensusState",
        "/ibc.lightclients.solomachine.v3.ConsensusState" => "*solomachine.ConsensusState",
        _ => "<nil>",
    }
}

/// `ClientState.VerifyUpgradeAndUpdateState`.
pub fn verify_upgrade_and_update_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    upgraded_client: &Any,
    upgraded_cons: &Any,
    proof_client: &[u8],
    proof_cons: &[u8],
) -> Result<()> {
    if cs.upgrade_path.is_empty() {
        return Err(cerr::invalid_upgrade_client(
            "cannot upgrade client, no upgrade path set",
        ));
    }
    let last = latest(cs);
    let decoded_upgraded_client = client::ClientState::from_any(upgraded_client)?;
    let up_latest = decoded_upgraded_client.latest_height();
    if !height::gt(&up_latest, &last) {
        return Err(ierr::invalid_height(format!(
            "upgraded client height {} must be at greater than current client height {}",
            height::to_string(&up_latest),
            height::to_string(&last)
        )));
    }
    let client::ClientState::Tendermint(up_cs) = decoded_upgraded_client else {
        return Err(cerr::invalid_client_type(format!(
            "upgraded client must be Tendermint client. expected: *tendermint.ClientState got: {}",
            decoded_upgraded_client.go_type()
        )));
    };
    let Some(Ok(up_cons)) = upgraded_cons.unpack::<ConsensusState>() else {
        return Err(cerr::invalid_consensus(format!(
            "upgraded consensus state must be Tendermint consensus state. expected *tendermint.ConsensusState, got: {}",
            consensus_state_go_type(upgraded_cons)
        )));
    };
    let mp_client = ics23_proof(proof_client, "client")?;
    let mp_cons = ics23_proof(proof_cons, "consensus state")?;
    let cons = get_consensus_state(ctx, client_id, &last)?.ok_or_else(|| {
        cerr::consensus_state_not_found("could not retrieve consensus state for lastHeight")
    })?;
    let root = cons.root.clone().unwrap_or_default().hash;
    let bz = client_state_any(&zero_custom_fields(&up_cs)).encode_to_vec();
    let client_path = upgrade_path(&cs.upgrade_path, &last, "upgradedClient");
    crate::commitment::verify_membership(&mp_client, &cs.proof_specs, &root, &client_path, &bz)
        .map_err(|e| {
            e.wrap(format!(
                "client state proof failed. Path: [{}]",
                client_path.key_path.join(" ")
            ))
        })?;
    let bz = consensus_state_any(&up_cons).encode_to_vec();
    let consensus_path = upgrade_path(&cs.upgrade_path, &last, "upgradedConsState");
    crate::commitment::verify_membership(&mp_cons, &cs.proof_specs, &root, &consensus_path, &bz)
        .map_err(|e| {
            e.wrap(format!(
                "consensus state proof failed. Path: [{}]",
                consensus_path.key_path.join(" ")
            ))
        })?;
    let new_cs = ClientState {
        chain_id: up_cs.chain_id.clone(),
        trust_level: cs.trust_level,
        trusting_period: cs.trusting_period,
        unbonding_period: up_cs.unbonding_period,
        max_clock_drift: cs.max_clock_drift,
        frozen_height: Some(Height::default()),
        latest_height: up_cs.latest_height,
        proof_specs: up_cs.proof_specs.clone(),
        upgrade_path: up_cs.upgrade_path.clone(),
        ..ClientState::default()
    };
    validate(&new_cs).map_err(|e| e.wrap("updated client state failed basic validation"))?;
    let new_cons = ConsensusState {
        timestamp: up_cons.timestamp,
        root: Some(MerkleRoot {
            hash: SENTINEL_ROOT.as_bytes().to_vec(),
        }),
        next_validators_hash: up_cons.next_validators_hash.clone(),
    };
    let nl = latest(&new_cs);
    set_client_state(ctx, client_id, &new_cs)?;
    set_consensus_state(ctx, client_id, &new_cons, &nl)?;
    set_consensus_metadata(ctx, client_id, &latest(&up_cs))
}

fn ics23_proof(
    bz: &[u8],
    what: &str,
) -> Result<celestia_proto::ibc::core::commitment::v1::MerkleProof> {
    crate::gogo_decode::<celestia_proto::ibc::core::commitment::v1::MerkleProof>(bz).map_err(|e| {
        commerr::invalid_proof(format!("could not unmarshal {what} merkle proof: {e}"))
    })
}

/// `IsMatchingClientState`.
fn is_matching(subject: &ClientState, substitute: &ClientState) -> bool {
    let strip = |c: &ClientState| {
        let mut c = normalize_client_state(c.clone());
        c.latest_height = Some(Height::default());
        c.frozen_height = Some(Height::default());
        c.trusting_period = Some(Duration::default());
        c.chain_id = String::new();
        #[allow(deprecated)]
        {
            c.allow_update_after_expiry = true;
            c.allow_update_after_misbehaviour = true;
        }
        c
    };
    strip(subject) == strip(substitute)
}

/// `ClientState.CheckSubstituteAndUpdateState`.
pub fn check_substitute_and_update_state(
    ctx: &mut Context<'_>,
    subject_id: &str,
    cs: &ClientState,
    substitute_id: &str,
    substitute: &Any,
) -> Result<()> {
    let decoded_substitute = client::ClientState::from_any(substitute)?;
    let client::ClientState::Tendermint(sub) = decoded_substitute else {
        return Err(cerr::invalid_client(format!(
            "expected type *tendermint.ClientState, got {}",
            decoded_substitute.go_type()
        )));
    };
    if !is_matching(cs, &sub) {
        return Err(cerr::invalid_substitute(
            "subject client state does not match substitute client state",
        ));
    }
    let mut cs = cs.clone();
    if status(ctx, subject_id, &cs)? == Status::Frozen {
        cs.frozen_height = Some(Height::default());
    }
    let h = latest(&sub);
    let cons = get_consensus_state(ctx, substitute_id, &h)?.ok_or_else(|| {
        cerr::consensus_state_not_found(
            "unable to retrieve latest consensus state for substitute client",
        )
    })?;
    set_consensus_state(ctx, subject_id, &cons, &h)?;
    let ph = get_processed_height(ctx, substitute_id, &h)?.ok_or_else(|| {
        cerr::update_client_failed(
            "unable to retrieve processed height for substitute client latest height",
        )
    })?;
    let pt = get_processed_time(ctx, substitute_id, &h)?.ok_or_else(|| {
        cerr::update_client_failed(
            "unable to retrieve processed time for substitute client latest height",
        )
    })?;
    set_consensus_metadata_with_values(ctx, subject_id, &h, &ph, pt)?;
    cs.latest_height = sub.latest_height;
    cs.chain_id.clone_from(&sub.chain_id);
    cs.trusting_period = sub.trusting_period;
    set_client_state(ctx, subject_id, &cs)
}

/// Decodes a client message `Any` of this client type.
#[must_use]
pub fn unpack_client_message(any: &Any) -> Option<core::result::Result<ClientMessage, SdkError>> {
    if let Some(r) = any.unpack::<Header>() {
        return Some(
            r.map(|h| ClientMessage::Header(Box::new(h)))
                .map_err(|e| ierr::unpack_any(e.to_string())),
        );
    }
    if let Some(r) = any.unpack::<Misbehaviour>() {
        return Some(
            r.map(|m| ClientMessage::Misbehaviour(Box::new(m)))
                .map_err(|e| ierr::unpack_any(e.to_string())),
        );
    }
    None
}

/// `ClientMessage.ValidateBasic`.
pub fn validate_client_message(msg: &ClientMessage) -> Result<()> {
    match msg {
        ClientMessage::Header(h) => validate_header_basic(h),
        ClientMessage::Misbehaviour(m) => validate_misbehaviour_basic(m),
    }
}

/// `Height` of a stored client state (for events).
#[must_use]
pub fn latest_height(cs: &ClientState) -> Height {
    latest(cs)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod delay_tests {
    use super::*;

    fn context() -> (sdk::MultiStore, sdk::Header) {
        let mut ms = sdk::MultiStore::new();
        ms.mount(crate::STORE_KEY, sdk::Kind::Iavl)
            .expect("mount IBC store");
        (ms, sdk::Header::default())
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn block_delay_error_includes_both_heights_like_go() {
        let mut ms = sdk::MultiStore::new();
        ms.mount(crate::STORE_KEY, sdk::Kind::Iavl).unwrap();
        let mut ctx = Context::new(
            &mut ms,
            sdk::Header {
                chain_id: "test-4".into(),
                height: 10,
                ..Default::default()
            },
            sdk::ExecMode::Finalize,
        );
        let proof = Height {
            revision_number: 1,
            revision_height: 5,
        };
        let processed = Height {
            revision_number: 4,
            revision_height: 9,
        };
        set_consensus_metadata_with_values(&mut ctx, "07-tendermint-0", &proof, &processed, 0)
            .unwrap();
        let error =
            verify_delay_period_passed(&mut ctx, "07-tendermint-0", &proof, 0, 3).unwrap_err();
        let reference: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../conformance/fixtures/ibc-delay.json"
        )))
        .unwrap();
        assert_eq!(error.log, reference["error"]);
        ctx.header.height = 12;
        verify_delay_period_passed(&mut ctx, "07-tendermint-0", &proof, 0, 3).unwrap();
    }

    #[test]
    fn missing_misbehaviour_consensus_state_includes_trusted_height() {
        let (mut ms, header) = context();
        let mut ctx = Context::new(&mut ms, header, sdk::ExecMode::Finalize);
        let trusted = Height {
            revision_number: 4,
            revision_height: 99,
        };
        let m = Misbehaviour {
            header_1: Some(Header {
                trusted_height: Some(trusted),
                ..Header::default()
            }),
            ..Misbehaviour::default()
        };
        let error = verify_misbehaviour(&mut ctx, "07-tendermint-0", &ClientState::default(), &m)
            .expect_err("the trusted consensus state is absent");
        assert!(error.log.starts_with(
            "could not get trusted consensus state from clientStore for Header1 at TrustedHeight: 4-99"
        ));
    }

    #[test]
    fn recovery_type_error_names_expected_and_actual_go_types() {
        use celestia_proto::ibc::lightclients::solomachine::v3 as sm;

        let (mut ms, header) = context();
        let mut ctx = Context::new(&mut ms, header, sdk::ExecMode::Finalize);
        let substitute = Any::pack(&sm::ClientState {
            sequence: 2,
            ..sm::ClientState::default()
        });
        let error = check_substitute_and_update_state(
            &mut ctx,
            "07-tendermint-0",
            &ClientState::default(),
            "06-solomachine-1",
            &substitute,
        )
        .expect_err("a solo-machine substitute has the wrong concrete type");
        assert!(
            error
                .log
                .starts_with("expected type *tendermint.ClientState, got *solomachine.ClientState")
        );
    }

    #[test]
    fn upgrade_paths_use_go_slice_format() {
        let path = upgrade_path(
            &["upgrade".into(), "upgradedIBCState".into()],
            &Height {
                revision_number: 1,
                revision_height: 10,
            },
            "upgradedClient",
        );
        assert_eq!(
            format!("[{}]", path.key_path.join(" ")),
            "[upgrade upgradedIBCState/10/upgradedClient]"
        );
    }

    #[test]
    fn trusted_validator_error_keeps_proto_and_hash_details() {
        use celestia_proto::celestia_core_proto::tendermint::crypto::{PublicKey, public_key::Sum};

        let key = [1_u8; 32];
        let validator = tm::Validator {
            address: crypto::ed25519::address(&key).to_vec(),
            pub_key: Some(PublicKey {
                sum: Some(Sum::Ed25519(key.to_vec())),
            }),
            voting_power: 1,
            proposer_priority: 0,
        };
        let validators = tm::ValidatorSet {
            validators: vec![validator.clone()],
            proposer: Some(validator),
            total_voting_power: 1,
        };
        let got = light::ValidatorSet::from_proto(Some(&validators))
            .expect("valid test validator set")
            .hash();
        let trusted = validator_set_text(&validators);
        assert_eq!(
            trusted,
            r#"validators:<address:"r\315n\204\"\304\007\373m\t\206\220\361\023\013}\355~\302\367" pub_key:<ed25519:"\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001" > voting_power:1 > proposer:<address:"r\315n\204\"\304\007\373m\t\206\220\361\023\013}\355~\302\367" pub_key:<ed25519:"\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001\001" > voting_power:1 > total_voting_power:1 "#
        );
        let expected_hash = vec![0xAA; 32];
        let error = check_trusted_header(
            &Header {
                trusted_validators: Some(validators),
                ..Header::default()
            },
            &ConsensusState {
                next_validators_hash: expected_hash.clone(),
                ..ConsensusState::default()
            },
        )
        .expect_err("the hashes differ");
        assert_eq!(
            error.log,
            format!(
                "trusted validators {trusted}, does not hash to latest trusted validators. Expected: {}, got: {}: invalid validator set",
                hex::encode_upper(expected_hash),
                hex::encode_upper(got)
            )
        );
    }
}
