//! ibc-go v6 (celestiaorg `v6.3.0`) message servers of celestia-app v1:
//! `MsgCreateClient`, `MsgUpdateClient` and `MsgConnectionOpenTry` for
//! 07-tendermint clients, with v6's store accesses, self-client checks
//! and events. Built on the ibc-go v8 primitives of `x-ibc` (stores,
//! header verification, ICS-23 proofs); other IBC messages and client
//! types are not ported.

use celestia_proto::Any;
use celestia_proto::ibc::core::client::v1::{self as clpb, Height};
use celestia_proto::ibc::core::commitment::v1::MerklePrefix;
use celestia_proto::ibc::core::connection::v1::{self as copb, ConnectionEnd, Counterparty, State};
use celestia_proto::ibc::lightclients::tendermint::v1 as tmpb;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_ibc::errors::client as clerr;
use x_ibc::lightclients::tendermint as tmc;
use x_ibc::{client, commitment, connection, height, host};

const TENDERMINT: &str = "07-tendermint";

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `clienttypes.UnpackClientState` of a 07-tendermint client.
fn unpack_tm_client_state(any: Option<&Any>) -> Result<tmpb::ClientState> {
    let any = any.ok_or_else(|| errs::unpack_any("protobuf Any message cannot be nil"))?;
    if any.type_url != tmpb::ClientState::type_url() {
        return Err(super::not_ported(errs::unpack_any(format!(
            "the ibc-go v6 client type {} is not ported",
            any.type_url
        ))));
    }
    tmpb::ClientState::decode(any.value.as_slice()).map_err(|e| errs::unpack_any(e.to_string()))
}

/// `ValidateBasic` of the ported IBC messages (ibc-go v8's, plus the
/// fields v6 still checks on `MsgConnectionOpenTry`).
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if let Some(r) = validate_basic_v6(msg) {
        return Some(r);
    }
    let r = crate::ibc::validate_basic(msg)?;
    if r.is_err() || msg.type_url != copb::MsgConnectionOpenTry::type_url() {
        return Some(r);
    }
    Some(decode::<copb::MsgConnectionOpenTry>(msg).and_then(|m| {
        #[allow(deprecated)]
        let (cs, proof_client, proof_consensus, consensus_height) = (
            m.client_state.as_ref(),
            &m.proof_client,
            &m.proof_consensus,
            m.consensus_height.unwrap_or_default(),
        );
        if cs.is_none() {
            return Err(clerr::invalid_client("counterparty client is nil"));
        }
        tmc::validate(&unpack_tm_client_state(cs)?)?;
        if proof_client.is_empty() {
            return Err(commitment_err("cannot submit empty proofClient"));
        }
        if proof_consensus.is_empty() {
            return Err(commitment_err("cannot submit empty proofConsensus"));
        }
        if height::is_zero(&consensus_height) {
            return Err(errs::invalid_height("consensus height must be non-zero"));
        }
        Ok(())
    }))
}

/// v6 `sdk.AccAddressFromBech32(msg.Signer)` check of the IBC messages
/// (`sdkerrors.ErrInvalidAddress`).
fn check_signer_v6(signer: &str) -> Result<()> {
    sdk::address::acc_from_bech32_go(signer)
        .map(|_| ())
        .map_err(|e| errs::invalid_address(format!("string could not be parsed as address: {e}")))
}

fn non_empty_proof(p: &[u8], log: &str) -> Result<()> {
    if p.is_empty() {
        return Err(commitment_err(log));
    }
    Ok(())
}

/// ibc-go v6 `ValidateBasic` of the messages whose v6 checks differ from
/// ibc-go v8's (`None` for the others).
fn validate_basic_v6(msg: &Any) -> Option<Result<()>> {
    // v6 `MsgUpdateClient.ValidateBasic` also unpacks the client message
    // and runs its `ValidateBasic` (v8's checks the signer and client ID
    // only).
    if msg.type_url == clpb::MsgUpdateClient::type_url() {
        return Some(decode(msg).and_then(|m: clpb::MsgUpdateClient| {
            check_signer_v6(&m.signer)?;
            let Some(cm) = m.client_message.as_ref() else {
                return Err(errs::unpack_any("protobuf Any message cannot be nil"));
            };
            match tmc::unpack_client_message(cm) {
                Some(Ok(tmc::ClientMessage::Header(h))) => tmc::validate_header_basic(&h)?,
                Some(Ok(tmc::ClientMessage::Misbehaviour(mb))) => {
                    tmc::validate_misbehaviour_basic(&mb)?;
                }
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(super::not_ported(errs::not_supported(format!(
                        "the ibc-go v6 ValidateBasic of client message {}",
                        cm.type_url
                    ))));
                }
            }
            host::client_identifier_validator(&m.client_id)
        }));
    }
    if msg.type_url == copb::MsgConnectionOpenInit::type_url() {
        return Some(decode(msg).and_then(|m: copb::MsgConnectionOpenInit| {
            host::client_identifier_validator(&m.client_id)
                .map_err(|e| e.wrap("invalid client ID"))?;
            let cp = m.counterparty.clone().unwrap_or_default();
            if !cp.connection_id.is_empty() {
                return Err(x_ibc::errors::connection::invalid_counterparty(
                    "counterparty connection identifier must be empty",
                ));
            }
            if let Some(v) = m.version.as_ref() {
                connection::validate_version(Some(v))
                    .map_err(|e| e.wrap("basic validation of the provided version failed"))?;
            }
            check_signer_v6(&m.signer)?;
            connection::validate_counterparty(&cp)
        }));
    }
    if msg.type_url == copb::MsgConnectionOpenAck::type_url() {
        return Some(decode(msg).and_then(|m: copb::MsgConnectionOpenAck| {
            if !connection::is_valid_connection_id(&m.connection_id) {
                return Err(x_ibc::errors::connection::invalid_connection_identifier(""));
            }
            host::connection_identifier_validator(&m.counterparty_connection_id)
                .map_err(|e| e.wrap("invalid counterparty connection ID"))?;
            connection::validate_version(m.version.as_ref())?;
            #[allow(deprecated)]
            let (cs, proof_client, proof_consensus, consensus_height) = (
                m.client_state.as_ref(),
                &m.proof_client,
                &m.proof_consensus,
                m.consensus_height.unwrap_or_default(),
            );
            if cs.is_none() {
                return Err(clerr::invalid_client("counterparty client is nil"));
            }
            let cs = unpack_tm_client_state(cs)
                .map_err(|e| clerr::invalid_client(format!("unpack err: {}", e.log)))?;
            tmc::validate(&cs).map_err(|e| e.wrap("counterparty client is invalid"))?;
            non_empty_proof(&m.proof_try, "cannot submit an empty proof try")?;
            non_empty_proof(proof_client, "cannot submit empty proof client")?;
            non_empty_proof(
                proof_consensus,
                "cannot submit an empty proof of consensus state",
            )?;
            if height::is_zero(&m.proof_height.unwrap_or_default()) {
                return Err(errs::invalid_height("proof height must be non-zero"));
            }
            if height::is_zero(&consensus_height) {
                return Err(errs::invalid_height("consensus height must be non-zero"));
            }
            check_signer_v6(&m.signer)
        }));
    }
    if msg.type_url == copb::MsgConnectionOpenConfirm::type_url() {
        return Some(decode(msg).and_then(|m: copb::MsgConnectionOpenConfirm| {
            if !connection::is_valid_connection_id(&m.connection_id) {
                return Err(x_ibc::errors::connection::invalid_connection_identifier(""));
            }
            non_empty_proof(&m.proof_ack, "cannot submit an empty proof ack")?;
            if height::is_zero(&m.proof_height.unwrap_or_default()) {
                return Err(errs::invalid_height("proof height must be non-zero"));
            }
            check_signer_v6(&m.signer)
        }));
    }
    None
}

fn commitment_err(log: &str) -> sdk::SdkError {
    x_ibc::errors::commitment::invalid_proof(log)
}

/// Executes a ported IBC message (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if msg.type_url == clpb::MsgCreateClient::type_url() {
        return Some(decode(msg).and_then(|m| create_client(ctx, &m)));
    }
    if msg.type_url == clpb::MsgUpdateClient::type_url() {
        return Some(decode(msg).and_then(|m| update_client(ctx, &m)));
    }
    if msg.type_url == copb::MsgConnectionOpenTry::type_url() {
        return Some(decode(msg).and_then(|m| conn_open_try(ctx, &m)));
    }
    if msg.type_url == copb::MsgConnectionOpenInit::type_url() {
        return Some(decode(msg).and_then(|m| conn_open_init(ctx, &m)));
    }
    if msg.type_url == copb::MsgConnectionOpenAck::type_url() {
        return Some(decode(msg).and_then(|m| conn_open_ack(ctx, &m)));
    }
    if msg.type_url == copb::MsgConnectionOpenConfirm::type_url() {
        return Some(decode(msg).and_then(|m| conn_open_confirm(ctx, &m)));
    }
    None
}

/// The `ibc` subspace's `AllowedClients` (`k.GetParams(ctx)`).
fn allowed_clients(ctx: &mut Context<'_>) -> Result<Vec<String>> {
    let raw = x_params::get_raw(ctx, "ibc", b"AllowedClients")?.unwrap_or_default();
    serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace ibc: AllowedClients: {e}")))
}

fn client_message_event(ctx: &mut Context<'_>) {
    ctx.events
        .emit(event("message", [("module", "ibc_client")]));
}

/// `MsgCreateClient` (02-client keeper `CreateClient`).
fn create_client(ctx: &mut Context<'_>, m: &clpb::MsgCreateClient) -> Result<Any> {
    let cs = unpack_tm_client_state(m.client_state.as_ref())?;
    let cons_any = m
        .consensus_state
        .as_ref()
        .ok_or_else(|| errs::unpack_any("protobuf Any message cannot be nil"))?;
    if !allowed_clients(ctx)?.iter().any(|c| c == TENDERMINT) {
        return Err(clerr::invalid_client_type(format!(
            "client state type {TENDERMINT} is not registered in the allowlist"
        )));
    }
    let client_id = client::generate_client_identifier(ctx, TENDERMINT)?;
    tmc::set_client_state(ctx, &client_id, &cs)?;
    // `Initialize`: the consensus state's metadata.
    let Some(Ok(cons)) = cons_any.unpack::<tmpb::ConsensusState>() else {
        return Err(clerr::invalid_consensus(
            "invalid initial consensus state. expected type: *types.ConsensusState",
        ));
    };
    let latest = cs.latest_height.unwrap_or_default();
    tmc::set_consensus_metadata(ctx, &client_id, &latest)?;
    tmc::set_consensus_state(ctx, &client_id, &cons, &latest)?;
    ctx.events.emit(event(
        "create_client",
        [
            ("client_id", client_id),
            ("client_type", TENDERMINT.to_owned()),
            ("consensus_height", height::to_string(&latest)),
        ],
    ));
    client_message_event(ctx);
    Ok(Any::pack(&clpb::MsgCreateClientResponse {}))
}

/// A 07-tendermint header as gogoproto marshals it after decoding: the
/// non-nullable fields an encoder may leave out are present (empty
/// messages; `stdtime` timestamps as Go's zero time).
pub(super) fn gogo_header(mut h: tmpb::Header) -> tmpb::Header {
    use celestia_proto::celestia_core_proto::tendermint::types::BlockId;
    let fill_block_id = |b: &mut Option<BlockId>| {
        b.get_or_insert_with(BlockId::default)
            .part_set_header
            .get_or_insert_with(Default::default);
    };
    if let Some(sh) = h.signed_header.as_mut() {
        if let Some(hd) = sh.header.as_mut() {
            hd.version.get_or_insert_with(Default::default);
            hd.time.get_or_insert_with(sdk::time::go_zero);
            fill_block_id(&mut hd.last_block_id);
        }
        if let Some(c) = sh.commit.as_mut() {
            fill_block_id(&mut c.block_id);
            for sig in &mut c.signatures {
                sig.timestamp.get_or_insert_with(sdk::time::go_zero);
            }
        }
    }
    for vs in [h.validator_set.as_mut(), h.trusted_validators.as_mut()]
        .into_iter()
        .flatten()
    {
        for v in vs.validators.iter_mut().chain(vs.proposer.as_mut()) {
            v.pub_key.get_or_insert_with(Default::default);
        }
    }
    h.trusted_height.get_or_insert_with(Height::default);
    h
}

/// `exported.Frozen` of a 07-tendermint client: `FrozenHeight` `0-1`.
const FROZEN: Height = Height {
    revision_number: 0,
    revision_height: 1,
};

/// `MsgUpdateClient` (02-client keeper `UpdateClient` and 07-tendermint
/// `CheckHeaderAndUpdateState`).
fn update_client(ctx: &mut Context<'_>, m: &clpb::MsgUpdateClient) -> Result<Any> {
    let header_any = m
        .client_message
        .as_ref()
        .ok_or_else(|| errs::unpack_any("protobuf Any message cannot be nil"))?;
    if header_any.type_url != tmpb::Header::type_url() {
        return Err(super::not_ported(errs::unpack_any(format!(
            "the ibc-go v6 header type {} is not ported",
            header_any.type_url
        ))));
    }
    let header = tmpb::Header::decode(header_any.value.as_slice())
        .map_err(|e| errs::unpack_any(e.to_string()))?;
    let client_id = &m.client_id;
    let cs = match client::client_state(ctx, client_id)? {
        Some(client::ClientState::Tendermint(cs)) => *cs,
        Some(_) => {
            return Err(super::not_ported(errs::not_supported(
                "the ibc-go v6 update of this client type is not ported",
            )));
        }
        None => {
            return Err(clerr::client_not_found(format!(
                "cannot update client with ID {client_id}"
            )));
        }
    };
    let status = tmc::status(ctx, client_id, &cs)?;
    if status != client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "cannot update client ({client_id}) with status {}",
            status.as_str()
        )));
    }
    let (new_cs, new_cons) = check_header_and_update_state(ctx, client_id, cs, &header)
        .map_err(|e| e.wrap(format!("cannot update client with ID {client_id}")))?;
    // `MustMarshalClientMessage`: the decoded header marshaled again
    // (gogoproto encodes non-nullable fields the input may have left out).
    let header_str = hex::encode(Any::pack(&gogo_header(header.clone())).encode_to_vec());
    let consensus_height = tmc::header_height(&header)?;
    tmc::set_client_state(ctx, client_id, &new_cs)?;
    let frozen = !height::is_zero(&new_cs.frozen_height.unwrap_or_default());
    let attrs = |h: &Height| {
        [
            ("client_id", client_id.clone()),
            ("client_type", TENDERMINT.to_owned()),
            ("consensus_height", height::to_string(h)),
            ("header", header_str.clone()),
        ]
    };
    if frozen {
        ctx.events
            .emit(event("client_misbehaviour", attrs(&consensus_height)));
    } else {
        // `Status` of the new client state reads its latest consensus state.
        let _ = tmc::status(ctx, client_id, &new_cs)?;
        tmc::set_consensus_state(ctx, client_id, &new_cons, &consensus_height)?;
        ctx.events
            .emit(event("update_client", attrs(&consensus_height)));
        client_message_event(ctx);
    }
    Ok(Any::pack(&clpb::MsgUpdateClientResponse {}))
}

/// 07-tendermint v6 `CheckHeaderAndUpdateState`: the new client state and
/// consensus state (the client frozen on misbehaviour).
fn check_header_and_update_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    mut cs: tmpb::ClientState,
    header: &tmpb::Header,
) -> Result<(tmpb::ClientState, tmpb::ConsensusState)> {
    let hh = tmc::header_height(header)?;
    let cons = tmc::header_consensus_state(header);
    let mut conflicting = false;
    if let Some(prev) = tmc::get_consensus_state(ctx, client_id, &hh)? {
        if tmc::normalize_consensus_state(prev.clone())
            == tmc::normalize_consensus_state(cons.clone())
        {
            return Ok((cs, prev));
        }
        conflicting = true;
    }
    // `GetConsensusState(TrustedHeight)` and `checkValidity`.
    tmc::verify_header(ctx, client_id, &cs, header)?;
    if conflicting {
        cs.frozen_height = Some(FROZEN);
        return Ok((cs, cons));
    }
    let t = x_ibc::lightclients::light::ts_nanos(cons.timestamp.as_ref());
    let prev = tmc::get_previous_consensus_state(ctx, client_id, &hh)?;
    let next = tmc::get_next_consensus_state(ctx, client_id, &hh)?;
    let ts = |c: &tmpb::ConsensusState| x_ibc::lightclients::light::ts_nanos(c.timestamp.as_ref());
    if prev.as_ref().is_some_and(|p| ts(p) >= t) || next.as_ref().is_some_and(|n| ts(n) <= t) {
        cs.frozen_height = Some(FROZEN);
        return Ok((cs, cons));
    }
    tmc::prune_oldest_consensus_state(ctx, client_id, &cs)?;
    if height::gt(&hh, &cs.latest_height.unwrap_or_default()) {
        cs.latest_height = Some(hh);
    }
    tmc::set_consensus_metadata(ctx, client_id, &hh)?;
    Ok((cs, cons))
}

/// 02-client `ValidateSelfClient`.
fn validate_self_client(ctx: &mut Context<'_>, cs: &tmpb::ClientState) -> Result<()> {
    let invalid = |log: String| clerr::invalid_client(log);
    if !height::is_zero(&cs.frozen_height.unwrap_or_default()) {
        return Err(clerr::client_frozen(""));
    }
    let chain_id = ctx.header.chain_id.clone();
    if chain_id != cs.chain_id {
        return Err(invalid(format!(
            "invalid chain-id. expected: {chain_id}, got: {}",
            cs.chain_id
        )));
    }
    let revision = client::parse_chain_id(&chain_id).unwrap_or(0);
    let latest = cs.latest_height.unwrap_or_default();
    if latest.revision_number != revision {
        return Err(invalid(format!(
            "client is not in the same revision as the chain. expected revision: {}, got: {revision}",
            latest.revision_number
        )));
    }
    #[allow(clippy::cast_sign_loss)]
    let self_height = Height {
        revision_number: revision,
        revision_height: ctx.header.height as u64,
    };
    if height::gte(&latest, &self_height) {
        return Err(invalid(format!(
            "client has LatestHeight {} greater than or equal to chain height {}",
            height::to_string(&latest),
            height::to_string(&self_height)
        )));
    }
    if cs.proof_specs != commitment::sdk_specs() {
        return Err(invalid("client has invalid proof specs".to_owned()));
    }
    let tl = cs.trust_level.unwrap_or_default();
    let (num, den) = (u128::from(tl.numerator), u128::from(tl.denominator));
    if num * 3 < den || num > den || den == 0 {
        return Err(invalid(format!(
            "trust-level invalid: trustLevel must be within [1/3, 1], given {}/{}",
            tl.numerator, tl.denominator
        )));
    }
    let expected = x_staking::store::unbonding_time(ctx)?;
    let nanos = tmc::dur_nanos(Some(&expected));
    let unbonding = tmc::dur_nanos(cs.unbonding_period.as_ref());
    if nanos != unbonding {
        return Err(invalid(format!(
            "invalid unbonding period. expected: {nanos}, got: {unbonding}"
        )));
    }
    if unbonding < tmc::dur_nanos(cs.trusting_period.as_ref()) {
        return Err(invalid(
            "unbonding period must be greater than trusting period".to_owned(),
        ));
    }
    if !cs.upgrade_path.is_empty() && cs.upgrade_path != ["upgrade", "upgradedIBCState"] {
        return Err(invalid(
            "upgrade path must be the upgrade path defined by upgrade module".to_owned(),
        ));
    }
    Ok(())
}

/// 02-client `GetSelfConsensusState`: from the staking `HistoricalInfo`
/// header of the height.
fn get_self_consensus_state(ctx: &mut Context<'_>, h: &Height) -> Result<tmpb::ConsensusState> {
    let revision = client::parse_chain_id(&ctx.header.chain_id).unwrap_or(0);
    if revision != h.revision_number {
        return Err(clerr::invalid_height(format!(
            "chainID revision number does not match height revision number: expected {revision}, got {}",
            h.revision_number
        )));
    }
    let height = i64::try_from(h.revision_height).unwrap_or(i64::MAX);
    let info = ctx
        .get(
            x_staking::STORE_KEY,
            &x_staking::keys::historical_info_v046(height),
        )?
        .ok_or_else(|| {
            errs::not_found(format!(
                "no historical info found at height {}",
                h.revision_height
            ))
        })?;
    let info = celestia_proto::cosmos::staking::v1beta1::HistoricalInfo::decode(info.as_slice())
        .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    let header = info.header.unwrap_or_default();
    Ok(tmc::normalize_consensus_state(tmpb::ConsensusState {
        timestamp: header.time,
        root: Some(celestia_proto::ibc::core::commitment::v1::MerkleRoot {
            hash: header.app_hash,
        }),
        next_validators_hash: header.next_validators_hash,
    }))
}

/// v6 connection keeper verification: the connection's client (active),
/// then `produceVerificationArgs` and the membership proof of `value` at
/// `path` under the counterparty prefix.
pub(super) fn verify(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    proof_height: &Height,
    proof: &[u8],
    path: &[u8],
    value: &[u8],
    wrap: &str,
) -> Result<()> {
    verify_with_delay(ctx, conn, proof_height, proof, path, value, wrap, false)
}

/// A 07-tendermint client error (`tendermint-client` codespace).
fn tm_client_err(code: u32, desc: &str, log: String) -> sdk::SdkError {
    sdk::SdkError::new("tendermint-client", code, desc).wrap(log)
}

/// 03-connection `getBlockDelay`: the connection's delay period in
/// blocks of the `ibc` subspace's `MaxExpectedTimePerBlock`, rounded up
/// (Go divides as float64; exact here below 2^53).
fn block_delay(ctx: &mut Context<'_>, conn: &ConnectionEnd) -> Result<u64> {
    const EXACT: u64 = 1 << 53;
    let raw = x_params::get_raw(ctx, "ibc", b"MaxExpectedTimePerBlock")?.unwrap_or_default();
    let expected: u64 = serde_json::from_slice::<String>(&raw)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| sdk::error::go_panic("subspace ibc: MaxExpectedTimePerBlock"))?;
    if expected == 0 {
        return Ok(0);
    }
    let delay = conn.delay_period;
    if delay >= EXACT || expected >= EXACT {
        return Err(super::not_ported(errs::not_supported(
            "a connection delay or expected block time beyond 2^53 (float64 rounding) is not ported",
        )));
    }
    Ok(delay.div_ceil(expected))
}

/// 07-tendermint `verifyDelayPeriodPassed` (v6).
fn verify_delay_passed(
    ctx: &mut Context<'_>,
    client_id: &str,
    proof_height: &Height,
    time_delay: u64,
    block_delay: u64,
) -> Result<()> {
    let processed_time =
        tmc::get_processed_time(ctx, client_id, proof_height)?.ok_or_else(|| {
            tm_client_err(
                8,
                "processed time not found",
                format!(
                    "processed time not found for height: {}",
                    height::to_string(proof_height)
                ),
            )
        })?;
    let now = u64::try_from(sdk::time::to_nanos(&ctx.header.time)).unwrap_or(0);
    let valid_time = processed_time.wrapping_add(time_delay);
    if now < valid_time {
        return Err(tm_client_err(
            10,
            "packet-specified delay period has not been reached",
            format!("cannot verify packet until time: {valid_time}, current time: {now}"),
        ));
    }
    let processed_height =
        tmc::get_processed_height(ctx, client_id, proof_height)?.ok_or_else(|| {
            tm_client_err(
                9,
                "processed height not found",
                format!(
                    "processed height not found for height: {}",
                    height::to_string(proof_height)
                ),
            )
        })?;
    let current = client::get_self_height(ctx)?;
    let valid = Height {
        revision_number: processed_height.revision_number,
        revision_height: processed_height.revision_height.wrapping_add(block_delay),
    };
    if height::lt(&current, &valid) {
        return Err(tm_client_err(
            10,
            "packet-specified delay period has not been reached",
            format!(
                "cannot verify packet until height: {}, current height: {}",
                height::to_string(&valid),
                height::to_string(&current)
            ),
        ));
    }
    Ok(())
}

/// [`verify`]; `delay` adds the packet verifications' delay period
/// checks (`getBlockDelay` before, `verifyDelayPeriodPassed` after the
/// proving consensus state).
#[allow(clippy::too_many_arguments)]
pub(super) fn verify_with_delay(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    proof_height: &Height,
    proof: &[u8],
    path: &[u8],
    value: &[u8],
    wrap: &str,
    delay: bool,
) -> Result<()> {
    verify_path(
        ctx,
        conn,
        proof_height,
        proof,
        path,
        Some(value),
        wrap,
        delay,
    )
}

/// [`verify_with_delay`] of the absence of `path` (`VerifyNonMembership`,
/// e.g. `VerifyPacketReceiptAbsence`).
pub(super) fn verify_absence_with_delay(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    proof_height: &Height,
    proof: &[u8],
    path: &[u8],
    wrap: &str,
) -> Result<()> {
    verify_path(ctx, conn, proof_height, proof, path, None, wrap, true)
}

/// The membership (`value`) or non-membership proof of a tendermint
/// client at `proof_height`, after `produceVerificationArgs`.
#[allow(clippy::too_many_arguments)]
fn verify_path(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    proof_height: &Height,
    proof: &[u8],
    path: &[u8],
    value: Option<&[u8]>,
    wrap: &str,
    delay: bool,
) -> Result<()> {
    let client_id = &conn.client_id;
    let cs = match client::client_state(ctx, client_id)? {
        Some(client::ClientState::Tendermint(cs)) => *cs,
        Some(_) => {
            return Err(super::not_ported(errs::not_supported(
                "the ibc-go v6 verification of this client type is not ported",
            )));
        }
        None => return Err(clerr::client_not_found(client_id)),
    };
    let status = tmc::status(ctx, client_id, &cs)?;
    if status != client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "client ({client_id}) status is {}",
            status.as_str()
        )));
    }
    let block_delay = if delay { block_delay(ctx, conn)? } else { 0 };
    let r = (|| {
        if height::lt(&cs.latest_height.unwrap_or_default(), proof_height) {
            return Err(errs::invalid_height(format!(
                "client state height < proof height ({} < {}), please ensure the client has been updated",
                height::to_string(&cs.latest_height.unwrap_or_default()),
                height::to_string(proof_height)
            )));
        }
        let prefix: &MerklePrefix = conn
            .counterparty
            .as_ref()
            .and_then(|c| c.prefix.as_ref())
            .ok_or_else(|| commitment_err("prefix cannot be empty"))?;
        let merkle_proof = commitment::decode_proof(proof)?;
        let proving = tmc::get_consensus_state(ctx, client_id, proof_height)?.ok_or_else(|| {
            clerr::consensus_state_not_found(
                "please ensure the proof was constructed against a height that exists on the client",
            )
        })?;
        if delay {
            verify_delay_passed(ctx, client_id, proof_height, conn.delay_period, block_delay)?;
        }
        let path =
            commitment::apply_prefix(Some(prefix), &[gostr::from_go_bytes(path).into_owned()])?;
        let root = proving.root.unwrap_or_default().hash;
        match value {
            Some(v) => {
                commitment::verify_membership(&merkle_proof, &cs.proof_specs, &root, &path, v)
            }
            None => commitment::verify_non_membership(&merkle_proof, &cs.proof_specs, &root, &path),
        }
    })();
    r.map_err(|e| e.wrap(format!("{wrap} ({client_id})")))
}

/// `MsgConnectionOpenTry` (03-connection keeper `ConnOpenTry`, v6).
#[allow(deprecated)]
fn conn_open_try(ctx: &mut Context<'_>, m: &copb::MsgConnectionOpenTry) -> Result<Any> {
    let target = unpack_tm_client_state(m.client_state.as_ref())?;
    let counterparty =
        connection::normalize_counterparty(m.counterparty.clone().unwrap_or_default());
    let proof_height = m.proof_height.unwrap_or_default();
    let consensus_height = m.consensus_height.unwrap_or_default();
    let r = (|| {
        let conn_id = connection::generate_connection_identifier(ctx)?;
        let self_height = client::get_self_height(ctx)?;
        if height::gte(&consensus_height, &self_height) {
            return Err(errs::invalid_height(format!(
                "consensus height is greater than or equal to the current block height ({} >= {})",
                height::to_string(&consensus_height),
                height::to_string(&self_height)
            )));
        }
        validate_self_client(ctx, &target)?;
        let expected_cons = get_self_consensus_state(ctx, &consensus_height).map_err(|e| {
            e.wrap(format!(
                "self consensus state not found for height {}",
                height::to_string(&consensus_height)
            ))
        })?;
        let expected = ConnectionEnd {
            client_id: counterparty.client_id.clone(),
            versions: m.counterparty_versions.clone(),
            state: State::Init as i32,
            counterparty: Some(Counterparty {
                client_id: m.client_id.clone(),
                connection_id: String::new(),
                prefix: Some(connection::commitment_prefix()),
            }),
            delay_period: m.delay_period,
        };
        let version = connection::pick_version(
            &connection::get_compatible_versions(),
            &m.counterparty_versions,
        )?;
        let end = ConnectionEnd {
            client_id: m.client_id.clone(),
            versions: vec![version],
            state: State::Tryopen as i32,
            counterparty: Some(counterparty.clone()),
            delay_period: m.delay_period,
        };
        verify(
            ctx,
            &end,
            &proof_height,
            &m.proof_init,
            &host::connection_key(&counterparty.connection_id),
            &connection::normalize_connection(expected).encode_to_vec(),
            "failed connection state verification for client",
        )?;
        verify(
            ctx,
            &end,
            &proof_height,
            &m.proof_client,
            &host::full_client_state_key(&counterparty.client_id),
            &tmc::client_state_any(&target).encode_to_vec(),
            "failed client state verification for target client",
        )?;
        verify(
            ctx,
            &end,
            &proof_height,
            &m.proof_consensus,
            &host::full_consensus_state_key(&counterparty.client_id, &consensus_height),
            &tmc::consensus_state_any(&expected_cons).encode_to_vec(),
            "failed consensus state verification for client",
        )?;
        connection::add_connection_to_client(ctx, &m.client_id, &conn_id).map_err(|e| {
            e.wrap(format!(
                "failed to add connection with ID {conn_id} to client with ID {}",
                m.client_id
            ))
        })?;
        connection::set_connection(ctx, &conn_id, &end)?;
        ctx.events.emit(event(
            "connection_open_try",
            [
                ("connection_id", conn_id),
                ("client_id", m.client_id.clone()),
                ("counterparty_client_id", counterparty.client_id.clone()),
                (
                    "counterparty_connection_id",
                    counterparty.connection_id.clone(),
                ),
            ],
        ));
        ctx.events
            .emit(event("message", [("module", "ibc_connection")]));
        Ok(())
    })();
    r.map_err(|e| e.wrap("connection handshake open try failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenTryResponse {}))
}

/// The `connection_open_*` event and the `ibc_connection` message event.
fn connection_events(
    ctx: &mut Context<'_>,
    ty: &str,
    conn_id: &str,
    client_id: &str,
    counterparty: &Counterparty,
) {
    ctx.events.emit(event(
        ty,
        [
            ("connection_id", conn_id.to_owned()),
            ("client_id", client_id.to_owned()),
            ("counterparty_client_id", counterparty.client_id.clone()),
            (
                "counterparty_connection_id",
                counterparty.connection_id.clone(),
            ),
        ],
    ));
    ctx.events
        .emit(event("message", [("module", "ibc_connection")]));
}

/// `MsgConnectionOpenInit` (03-connection keeper `ConnOpenInit`, v6).
fn conn_open_init(ctx: &mut Context<'_>, m: &copb::MsgConnectionOpenInit) -> Result<Any> {
    let counterparty = m.counterparty.clone().unwrap_or_default();
    let r = (|| {
        let versions = match &m.version {
            Some(v) => {
                if !connection::is_supported_version(&connection::get_compatible_versions(), v) {
                    return Err(x_ibc::errors::connection::invalid_version(
                        "version is not supported",
                    ));
                }
                vec![v.clone()]
            }
            None => connection::get_compatible_versions(),
        };
        let conn_id = connection::generate_connection_identifier(ctx)?;
        let end = ConnectionEnd {
            client_id: m.client_id.clone(),
            versions,
            state: State::Init as i32,
            counterparty: Some(counterparty.clone()),
            delay_period: m.delay_period,
        };
        connection::set_connection(ctx, &conn_id, &end)?;
        connection::add_connection_to_client(ctx, &m.client_id, &conn_id)?;
        connection_events(
            ctx,
            "connection_open_init",
            &conn_id,
            &m.client_id,
            &counterparty,
        );
        Ok(())
    })();
    r.map_err(|e| e.wrap("connection handshake open init failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenInitResponse {}))
}

/// `GetConnection`, failing with `ErrConnectionNotFound`.
fn connection_end(ctx: &mut Context<'_>, conn_id: &str) -> Result<ConnectionEnd> {
    connection::get_connection(ctx, conn_id)?
        .ok_or_else(|| x_ibc::errors::connection::connection_not_found(conn_id))
}

/// The proto name of a connection state (`State.String()`).
pub(super) fn connection_state_name(s: i32) -> String {
    state_name(s)
}

fn state_name(s: i32) -> String {
    State::try_from(s).map_or_else(|_| s.to_string(), |s| s.as_str_name().to_owned())
}

/// `MsgConnectionOpenAck` (03-connection keeper `ConnOpenAck`, v6).
#[allow(deprecated)]
fn conn_open_ack(ctx: &mut Context<'_>, m: &copb::MsgConnectionOpenAck) -> Result<Any> {
    // `UnpackClientState` in the msg server, before the keeper.
    let target = unpack_tm_client_state(m.client_state.as_ref())?;
    let proof_height = m.proof_height.unwrap_or_default();
    let consensus_height = m.consensus_height.unwrap_or_default();
    let version = m.version.clone().unwrap_or_default();
    let r = (|| {
        let self_height = client::get_self_height(ctx)?;
        if height::gte(&consensus_height, &self_height) {
            return Err(errs::invalid_height(format!(
                "consensus height is greater than or equal to the current block height ({} >= {})",
                height::to_string(&consensus_height),
                height::to_string(&self_height)
            )));
        }
        let mut conn = connection_end(ctx, &m.connection_id)?;
        if conn.state != State::Init as i32 {
            return Err(x_ibc::errors::connection::invalid_connection_state(
                format!(
                    "connection state is not INIT (got {})",
                    state_name(conn.state)
                ),
            ));
        }
        if !connection::is_supported_version(&conn.versions, &version) {
            return Err(x_ibc::errors::connection::invalid_connection_state(
                format!(
                    "the counterparty selected version {} is not supported by versions selected on INIT",
                    version_string(&version)
                ),
            ));
        }
        validate_self_client(ctx, &target)?;
        let expected_cons = get_self_consensus_state(ctx, &consensus_height).map_err(|e| {
            e.wrap(format!(
                "self consensus state not found for height {}",
                height::to_string(&consensus_height)
            ))
        })?;
        let cp = conn.counterparty.clone().unwrap_or_default();
        let expected = ConnectionEnd {
            client_id: cp.client_id.clone(),
            versions: vec![version.clone()],
            state: State::Tryopen as i32,
            counterparty: Some(Counterparty {
                client_id: conn.client_id.clone(),
                connection_id: m.connection_id.clone(),
                prefix: Some(connection::commitment_prefix()),
            }),
            delay_period: conn.delay_period,
        };
        verify(
            ctx,
            &conn,
            &proof_height,
            &m.proof_try,
            &host::connection_key(&m.counterparty_connection_id),
            &connection::normalize_connection(expected).encode_to_vec(),
            "failed connection state verification for client",
        )?;
        verify(
            ctx,
            &conn,
            &proof_height,
            &m.proof_client,
            &host::full_client_state_key(&cp.client_id),
            &tmc::client_state_any(&target).encode_to_vec(),
            "failed client state verification for target client",
        )?;
        verify(
            ctx,
            &conn,
            &proof_height,
            &m.proof_consensus,
            &host::full_consensus_state_key(&cp.client_id, &consensus_height),
            &tmc::consensus_state_any(&expected_cons).encode_to_vec(),
            "failed consensus state verification for client",
        )?;
        conn.state = State::Open as i32;
        conn.versions = vec![version.clone()];
        let mut cp = cp;
        cp.connection_id.clone_from(&m.counterparty_connection_id);
        conn.counterparty = Some(cp.clone());
        connection::set_connection(ctx, &m.connection_id, &conn)?;
        connection_events(
            ctx,
            "connection_open_ack",
            &m.connection_id,
            &conn.client_id,
            &cp,
        );
        Ok(())
    })();
    r.map_err(|e| e.wrap("connection handshake open ack failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenAckResponse {}))
}

/// Go `%s` of a `*Version` (its gogoproto `String()`, compact text).
fn version_string(v: &copb::Version) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    if !v.identifier.is_empty() {
        let _ = write!(s, "identifier:{:?} ", v.identifier);
    }
    for f in &v.features {
        let _ = write!(s, "features:{f:?} ");
    }
    s
}

/// `MsgConnectionOpenConfirm` (03-connection keeper `ConnOpenConfirm`, v6).
fn conn_open_confirm(ctx: &mut Context<'_>, m: &copb::MsgConnectionOpenConfirm) -> Result<Any> {
    let proof_height = m.proof_height.unwrap_or_default();
    let r = (|| {
        let mut conn = connection_end(ctx, &m.connection_id)?;
        if conn.state != State::Tryopen as i32 {
            return Err(x_ibc::errors::connection::invalid_connection_state(
                format!(
                    "connection state is not TRYOPEN (got {})",
                    state_name(conn.state)
                ),
            ));
        }
        let cp = conn.counterparty.clone().unwrap_or_default();
        let expected = ConnectionEnd {
            client_id: cp.client_id.clone(),
            versions: conn.versions.clone(),
            state: State::Open as i32,
            counterparty: Some(Counterparty {
                client_id: conn.client_id.clone(),
                connection_id: m.connection_id.clone(),
                prefix: Some(connection::commitment_prefix()),
            }),
            delay_period: conn.delay_period,
        };
        verify(
            ctx,
            &conn,
            &proof_height,
            &m.proof_ack,
            &host::connection_key(&cp.connection_id),
            &connection::normalize_connection(expected).encode_to_vec(),
            "failed connection state verification for client",
        )?;
        conn.state = State::Open as i32;
        connection::set_connection(ctx, &m.connection_id, &conn)?;
        connection_events(
            ctx,
            "connection_open_confirm",
            &m.connection_id,
            &conn.client_id,
            &cp,
        );
        Ok(())
    })();
    r.map_err(|e| e.wrap("connection handshake open confirm failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenConfirmResponse {}))
}
