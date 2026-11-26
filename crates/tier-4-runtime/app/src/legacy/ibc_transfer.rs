//! ibc-go v6 (celestiaorg `v6.3.0`) ICS-20 packets of celestia-app v1:
//! `MsgTransfer` (its v6 `ValidateBasic`, the transfer msg server with
//! `x/params` `SendEnabled`, the v0.46 bank send-enabled check, escrow
//! without v7's total-escrow tracking, and 04-channel `SendPacket` with
//! the v6 checks and event; the packet-forward middleware keeper that
//! wraps `SendPacket` passes it through) and `MsgAcknowledgement` (the
//! core msg server's no-op handling, 04-channel `AcknowledgePacket` with
//! the v6 delay-period checks, and the transfer callback behind the
//! token filter).

use celestia_proto::Any;
use celestia_proto::ibc::applications::transfer::v1 as trpb;
use celestia_proto::ibc::core::channel::v1::{self as chpb, Channel, Packet, State};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::lightclients::tendermint::v1 as tmpb;
use prost::{Message, Name};
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_ibc::errors::{channel as cherr, client as clerr};
use x_ibc::lightclients::tendermint as tmc;
use x_ibc::{channel, height, host};
use x_transfer::relay as tr;

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// The token of a `MsgTransfer` as gogoproto decodes it (a nil amount
/// when the field is empty).
fn token_of(m: &trpb::MsgTransfer) -> Option<Coin> {
    let t = m.token.clone().unwrap_or_default();
    if t.amount.is_empty() {
        return None;
    }
    Coin::from_proto(&t).ok()
}

/// `MsgTransfer.ValidateBasic` (v6: SDK error codes, no memo or receiver
/// length limits).
fn validate_transfer(m: &trpb::MsgTransfer) -> Result<()> {
    host::port_identifier_validator(&m.source_port)
        .map_err(|e| e.wrap("invalid source port ID"))?;
    host::channel_identifier_validator(&m.source_channel)
        .map_err(|e| e.wrap("invalid source channel ID"))?;
    let raw = m.token.clone().unwrap_or_default();
    let token = match token_of(m) {
        Some(c) if c.is_valid() => c,
        Some(c) => return Err(errs::invalid_coins(c.to_string())),
        None => {
            return Err(errs::invalid_coins(sdk::coins::raw_coins_string(
                std::slice::from_ref(&raw),
            )));
        }
    };
    if !token.amount.is_positive() {
        return Err(errs::insufficient_funds(token.to_string()));
    }
    sdk::address::acc_from_bech32_go(&m.sender).map_err(|e| {
        errs::invalid_address(format!("string could not be parsed as address: {e}"))
    })?;
    if m.receiver.trim().is_empty() {
        return Err(errs::invalid_address("missing recipient address"));
    }
    tr::validate_ibc_denom(&token.denom)
}

/// ibc-go v6 `ValidateBasic` of `MsgTransfer` (`None` for other messages).
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if msg.type_url == trpb::MsgTransfer::type_url() {
        return Some(decode(msg).and_then(|m| validate_transfer(&m)));
    }
    if msg.type_url == chpb::MsgRecvPacket::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgRecvPacket| {
            if m.proof_commitment.is_empty() {
                return Err(x_ibc::errors::commitment::invalid_proof(
                    "cannot submit an empty proof",
                ));
            }
            if height::is_zero(&m.proof_height.unwrap_or_default()) {
                return Err(errs::invalid_height("proof height must be non-zero"));
            }
            sdk::address::acc_from_bech32_go(&m.signer).map_err(|e| {
                errs::invalid_address(format!("string could not be parsed as address: {e}"))
            })?;
            validate_packet(&m.packet.clone().unwrap_or_default())
        }));
    }
    if msg.type_url == chpb::MsgAcknowledgement::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgAcknowledgement| {
            if m.proof_acked.is_empty() {
                return Err(x_ibc::errors::commitment::invalid_proof(
                    "cannot submit an empty proof",
                ));
            }
            if height::is_zero(&m.proof_height.unwrap_or_default()) {
                return Err(errs::invalid_height("proof height must be non-zero"));
            }
            if m.acknowledgement.is_empty() {
                return Err(cherr::invalid_acknowledgement("ack bytes cannot be empty"));
            }
            sdk::address::acc_from_bech32_go(&m.signer).map_err(|e| {
                errs::invalid_address(format!("string could not be parsed as address: {e}"))
            })?;
            validate_packet(&m.packet.clone().unwrap_or_default())
        }));
    }
    None
}

/// Executes `MsgTransfer` (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if msg.type_url == trpb::MsgTransfer::type_url() {
        return Some(decode(msg).and_then(|m| transfer(ctx, &m)));
    }
    if msg.type_url == chpb::MsgAcknowledgement::type_url() {
        return Some(decode(msg).and_then(|m| acknowledgement(ctx, &m)));
    }
    if msg.type_url == chpb::MsgRecvPacket::type_url() {
        return Some(decode(msg).and_then(|m| recv_packet_msg(ctx, &m)));
    }
    None
}

/// The `transfer` subspace's `SendEnabled` (`GetSendEnabled`).
fn send_enabled(ctx: &mut Context<'_>) -> Result<bool> {
    let raw = x_params::get_raw(ctx, "transfer", b"SendEnabled")?
        .ok_or_else(|| sdk::error::go_panic("UnmarshalJSON: transfer SendEnabled is not set"))?;
    serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace transfer: SendEnabled: {e}")))
}

/// transfer `msgServer.Transfer` (v6).
fn transfer(ctx: &mut Context<'_>, m: &trpb::MsgTransfer) -> Result<Any> {
    if !send_enabled(ctx)? {
        return Err(tr::errors::send_disabled(""));
    }
    let sender = tr::acc_address(&m.sender)?;
    let token = token_of(m).ok_or_else(|| errs::invalid_coins("nil amount"))?;
    if super::msgs::is_send_enabled_coins(ctx, &Coins(vec![token.clone()])).is_err() {
        return Err(tr::errors::send_disabled(&format!(
            "{} transfers are currently disabled",
            token.denom
        )));
    }
    if x_bank::is_blocked(&sender) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to send funds",
            m.sender
        )));
    }
    let sequence = send_transfer(ctx, m, &token, &sender)?;
    ctx.events.emit(event(
        "ibc_transfer",
        [
            ("sender", m.sender.clone()),
            ("receiver", m.receiver.clone()),
        ],
    ));
    ctx.events
        .emit(event("message", [("module", x_transfer::MODULE_NAME)]));
    Ok(Any::pack(&trpb::MsgTransferResponse { sequence }))
}

/// transfer keeper `sendTransfer` (v6).
fn send_transfer(
    ctx: &mut Context<'_>,
    m: &trpb::MsgTransfer,
    token: &Coin,
    sender: &[u8],
) -> Result<u64> {
    let (port, chan) = (&m.source_port, &m.source_channel);
    if channel::get_channel(ctx, port, chan)?.is_none() {
        return Err(cherr::channel_not_found(format!(
            "port ID ({port}) channel ID ({chan})"
        )));
    }
    let cap = x_transfer::SCOPED_KEEPER
        .get_capability(ctx, &host::channel_capability_path(port, chan))?
        .ok_or_else(|| {
            cherr::channel_capability_not_found("module does not own channel capability")
        })?;
    let mut full_denom_path = token.denom.clone();
    if token.denom.starts_with("ibc/") {
        full_denom_path = tr::denom_path_from_hash(ctx, &token.denom)?;
    }
    if tr::sender_chain_is_source(port, chan, &full_denom_path) {
        let escrow = x_transfer::escrow_address(port, chan);
        x_bank::send_coins(ctx, sender, &escrow, &tr::coins(token))?;
    } else {
        // The burn path of a voucher is not ported (its v0.46 module
        // account sends and burns are unverified).
        return Err(super::not_ported(errs::not_supported(
            "the ibc-go v6 transfer of a voucher (burn) is not ported",
        )));
    }
    let data = tr::PacketData {
        denom: full_denom_path,
        amount: token.amount.to_string(),
        sender: x_auth::bech32(sender)?,
        receiver: m.receiver.clone(),
        memo: m.memo.clone(),
    };
    send_packet(
        ctx,
        &cap,
        port,
        chan,
        &m.timeout_height.unwrap_or_default(),
        m.timeout_timestamp,
        &data.get_bytes(),
    )
}

/// `Packet.ValidateBasic` (v6).
pub(super) fn validate_packet(p: &Packet) -> Result<()> {
    host::port_identifier_validator(&p.source_port)
        .map_err(|e| e.wrap("invalid source port ID"))?;
    host::port_identifier_validator(&p.destination_port)
        .map_err(|e| e.wrap("invalid destination port ID"))?;
    host::channel_identifier_validator(&p.source_channel)
        .map_err(|e| e.wrap("invalid source channel ID"))?;
    host::channel_identifier_validator(&p.destination_channel)
        .map_err(|e| e.wrap("invalid destination channel ID"))?;
    if p.sequence == 0 {
        return Err(cherr::invalid_packet("packet sequence cannot be 0"));
    }
    if height::is_zero(&p.timeout_height.unwrap_or_default()) && p.timeout_timestamp == 0 {
        return Err(cherr::invalid_packet(
            "packet timeout height and packet timeout timestamp cannot both be 0",
        ));
    }
    if p.data.is_empty() {
        return Err(cherr::invalid_packet("packet data bytes cannot be empty"));
    }
    Ok(())
}

/// Go `time.Unix(0, n).String()` in UTC.
fn unix_nanos_string(n: u64) -> String {
    let n = i128::from(i64::try_from(n).unwrap_or(i64::MAX));
    sdk::time::go_time_string(&sdk::time::from_nanos(n))
}

/// 04-channel `SendPacket` (v6).
#[allow(clippy::trivially_copy_pass_by_ref, clippy::too_many_lines)]
fn send_packet(
    ctx: &mut Context<'_>,
    cap: &celestia_proto::capability::v1::Capability,
    port: &str,
    chan: &str,
    timeout_height: &Height,
    timeout_timestamp: u64,
    data: &[u8],
) -> Result<u64> {
    let ch: Channel =
        channel::get_channel(ctx, port, chan)?.ok_or_else(|| cherr::channel_not_found(chan))?;
    if ch.state == State::Closed as i32 {
        return Err(cherr::invalid_channel_state(
            "channel is CLOSED (got STATE_CLOSED)",
        ));
    }
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(
        ctx,
        cap,
        &host::channel_capability_path(port, chan),
    )? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    let seq = channel::get_next_sequence_send(ctx, port, chan)?.ok_or_else(|| {
        cherr::sequence_send_not_found(format!("source port: {port}, source channel: {chan}"))
    })?;
    let cp = ch.counterparty.clone().unwrap_or_default();
    let packet = Packet {
        sequence: seq,
        source_port: port.to_owned(),
        source_channel: chan.to_owned(),
        destination_port: cp.port_id,
        destination_channel: cp.channel_id,
        data: data.to_vec(),
        timeout_height: Some(*timeout_height),
        timeout_timestamp,
    };
    validate_packet(&packet).map_err(|e| e.wrap("constructed packet failed basic validation"))?;
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    let conn = x_ibc::connection::get_connection(ctx, &hop)?
        .ok_or_else(|| x_ibc::errors::connection::connection_not_found(&hop))?;
    let client_id = conn.client_id.clone();
    let cs: tmpb::ClientState = match x_ibc::client::client_state(ctx, &client_id)? {
        Some(x_ibc::client::ClientState::Tendermint(cs)) => *cs,
        Some(_) => {
            return Err(super::not_ported(errs::not_supported(
                "the ibc-go v6 send over this client type is not ported",
            )));
        }
        None => return Err(clerr::consensus_state_not_found("")),
    };
    let status = tmc::status(ctx, &client_id, &cs)?;
    if status != x_ibc::client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "cannot send packet using client ({client_id}) with status {}",
            status.as_str()
        )));
    }
    let latest = cs.latest_height.unwrap_or_default();
    if !height::is_zero(timeout_height) && height::gte(&latest, timeout_height) {
        return Err(cherr::packet_timeout(format!(
            "receiving chain block height >= packet timeout height ({} >= {})",
            height::to_string(&latest),
            height::to_string(timeout_height)
        )));
    }
    // `GetTimestampAtHeight`: the consensus state at the latest height.
    let latest_cons = tmc::get_consensus_state(ctx, &client_id, &latest)?.ok_or_else(|| {
        clerr::consensus_state_not_found(format!(
            "clientID ({client_id}), height ({})",
            height::to_string(&latest)
        ))
    })?;
    let latest_ts = x_ibc::lightclients::light::ts_nanos(latest_cons.timestamp.as_ref());
    let latest_ts = u64::try_from(latest_ts).unwrap_or(0);
    if timeout_timestamp != 0 && latest_ts >= timeout_timestamp {
        return Err(cherr::packet_timeout(format!(
            "receiving chain block timestamp >= packet timeout timestamp ({} >= {})",
            unix_nanos_string(latest_ts),
            unix_nanos_string(timeout_timestamp)
        )));
    }
    let commitment = channel::commit_packet(&packet);
    channel::set_next_sequence_send(ctx, port, chan, seq.wrapping_add(1))?;
    channel::set_packet_commitment(ctx, port, chan, seq, &commitment)?;
    ctx.events.emit(event(
        "send_packet",
        [
            (
                "packet_data",
                gostr::from_go_bytes(&packet.data).into_owned(),
            ),
            ("packet_data_hex", hex::encode(&packet.data)),
            ("packet_timeout_height", height::to_string(timeout_height)),
            ("packet_timeout_timestamp", timeout_timestamp.to_string()),
            ("packet_sequence", seq.to_string()),
            ("packet_src_port", packet.source_port.clone()),
            ("packet_src_channel", packet.source_channel.clone()),
            ("packet_dst_port", packet.destination_port.clone()),
            ("packet_dst_channel", packet.destination_channel.clone()),
            (
                "packet_channel_ordering",
                channel::order_name(ch.ordering).into_owned(),
            ),
            ("packet_connection", hop),
        ],
    ));
    ctx.events
        .emit(event("message", [("module", "ibc_channel")]));
    Ok(seq)
}

/// `channel.Ordering.String()` and the connection of a channel.
fn ordering_and_hop(ch: &Channel) -> (String, String) {
    (
        channel::order_name(ch.ordering).into_owned(),
        ch.connection_hops.first().cloned().unwrap_or_default(),
    )
}

/// `EmitAcknowledgePacketEvent` (v6).
fn emit_acknowledge_packet(ctx: &mut Context<'_>, p: &Packet, ch: &Channel) {
    let (ordering, hop) = ordering_and_hop(ch);
    ctx.events.emit(event(
        "acknowledge_packet",
        [
            (
                "packet_timeout_height",
                height::to_string(&p.timeout_height.unwrap_or_default()),
            ),
            ("packet_timeout_timestamp", p.timeout_timestamp.to_string()),
            ("packet_sequence", p.sequence.to_string()),
            ("packet_src_port", p.source_port.clone()),
            ("packet_src_channel", p.source_channel.clone()),
            ("packet_dst_port", p.destination_port.clone()),
            ("packet_dst_channel", p.destination_channel.clone()),
            ("packet_channel_ordering", ordering),
            ("packet_connection", hop),
        ],
    ));
    ctx.events
        .emit(event("message", [("module", "ibc_channel")]));
}

/// Why `AcknowledgePacket` did not apply.
enum AckFailure {
    /// `ErrNoOpMsg`: the packet commitment is gone (already relayed).
    NoOp,
    /// Any other error.
    Err(sdk::SdkError),
}

impl From<sdk::SdkError> for AckFailure {
    fn from(e: sdk::SdkError) -> Self {
        Self::Err(e)
    }
}

/// 04-channel `AcknowledgePacket` (v6).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn acknowledge_packet(
    ctx: &mut Context<'_>,
    cap: &celestia_proto::capability::v1::Capability,
    p: &Packet,
    ack: &[u8],
    proof: &[u8],
    proof_height: &Height,
) -> core::result::Result<(), AckFailure> {
    let (port, chan) = (&p.source_port, &p.source_channel);
    let ch = channel::get_channel(ctx, port, chan)?
        .ok_or_else(|| cherr::channel_not_found(format!("port ID ({port}) channel ID ({chan})")))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state is not OPEN (got {})",
            channel::state_name(ch.state)
        ))
        .into());
    }
    let cap_name = host::channel_capability_path(port, chan);
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        ))
        .into());
    }
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.destination_port != cp.port_id {
        return Err(cherr::invalid_packet(format!(
            "packet destination port doesn't match the counterparty's port ({} \u{2260} {})",
            p.destination_port, cp.port_id
        ))
        .into());
    }
    if p.destination_channel != cp.channel_id {
        return Err(cherr::invalid_packet(format!(
            "packet destination channel doesn't match the counterparty's channel ({} \u{2260} {})",
            p.destination_channel, cp.channel_id
        ))
        .into());
    }
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    let conn = x_ibc::connection::get_connection(ctx, &hop)?
        .ok_or_else(|| x_ibc::errors::connection::connection_not_found(&hop))?;
    if conn.state != celestia_proto::ibc::core::connection::v1::State::Open as i32 {
        return Err(x_ibc::errors::connection::invalid_connection_state(format!(
            "connection state is not OPEN (got {})",
            super::ibc::connection_state_name(conn.state)
        ))
        .into());
    }
    let commitment = channel::get_packet_commitment(ctx, port, chan, p.sequence)?;
    if commitment.is_empty() {
        emit_acknowledge_packet(ctx, p, &ch);
        return Err(AckFailure::NoOp);
    }
    let packet_commitment = channel::commit_packet(p);
    if channel::ack_marshalling_mismatch(ack) {
        return Err(cherr::invalid_acknowledgement("acknowledgement marshalling error").into());
    }
    if commitment != packet_commitment {
        return Err(cherr::invalid_packet(format!(
            "commitment bytes are not equal: got ({}), expected ({})",
            go_byte_slice(&packet_commitment),
            go_byte_slice(&commitment)
        ))
        .into());
    }
    super::ibc::verify_with_delay(
        ctx,
        &conn,
        proof_height,
        proof,
        &host::packet_acknowledgement_key(&p.destination_port, &p.destination_channel, p.sequence),
        &channel::commit_acknowledgement(ack),
        "failed packet acknowledgement verification for client",
        true,
    )?;
    if ch.ordering == chpb::Order::Ordered as i32 {
        let next = channel::get_next_sequence_ack(ctx, port, chan)?.ok_or_else(|| {
            cherr::sequence_ack_not_found(format!("source port: {port}, source channel: {chan}"))
        })?;
        if p.sequence != next {
            return Err(cherr::packet_sequence_out_of_order(format!(
                "packet sequence \u{2260} next ack sequence ({} \u{2260} {next})",
                p.sequence
            ))
            .into());
        }
        channel::set_next_sequence_ack(ctx, port, chan, next.wrapping_add(1))?;
    }
    ctx.delete(
        x_ibc::STORE_KEY,
        &host::packet_commitment_key(port, chan, p.sequence),
    )?;
    emit_acknowledge_packet(ctx, p, &ch);
    Ok(())
}

/// Go `%v` of a `[]byte`.
pub(super) fn go_byte_slice(b: &[u8]) -> String {
    let parts: Vec<String> = b.iter().map(u8::to_string).collect();
    format!("[{}]", parts.join(" "))
}

/// core `msgServer.Acknowledgement` (v6).
fn acknowledgement(ctx: &mut Context<'_>, m: &chpb::MsgAcknowledgement) -> Result<Any> {
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) = channel::lookup_module_by_channel(ctx, &p.source_port, &p.source_channel)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    if module != x_transfer::MODULE_NAME {
        return Err(super::not_ported(errs::not_supported(format!(
            "the ibc-go v6 packet callbacks of module {module} are not ported"
        ))));
    }
    // `ctx.CacheContext()`: its own store branch and event manager.
    let outer_events = std::mem::take(&mut ctx.events);
    let cp = ctx.branch();
    let r = acknowledge_packet(
        ctx,
        &cap,
        &p,
        &m.acknowledgement,
        &m.proof_acked,
        &m.proof_height.unwrap_or_default(),
    );
    let mut cache_events = std::mem::replace(&mut ctx.events, outer_events);
    match r {
        Ok(()) => {
            ctx.write(cp);
            ctx.events.emit_all(cache_events.take());
        }
        Err(AckFailure::NoOp) => {
            ctx.multistore().discard_branch();
            return Ok(Any::pack(&chpb::MsgAcknowledgementResponse {
                result: chpb::ResponseResultType::Noop as i32,
            }));
        }
        Err(AckFailure::Err(e)) => {
            ctx.multistore().discard_branch();
            return Err(e.wrap("acknowledge packet verification failed"));
        }
    }
    on_acknowledgement_packet(ctx, &p, &m.acknowledgement)
        .map_err(|e| e.wrap("acknowledge packet callback failed"))?;
    Ok(Any::pack(&chpb::MsgAcknowledgementResponse {
        result: chpb::ResponseResultType::Success as i32,
    }))
}

/// Transfer `OnAcknowledgementPacket` (v6; the token filter passes it).
fn on_acknowledgement_packet(ctx: &mut Context<'_>, p: &Packet, ack_bz: &[u8]) -> Result<()> {
    let ack = channel::decode_acknowledgement(ack_bz).map_err(|e| {
        errs::unknown_request(format!(
            "cannot unmarshal ICS-20 transfer packet acknowledgement: {e}"
        ))
    })?;
    let data = tr::PacketData::decode(&p.data).map_err(|e| {
        errs::unknown_request(format!("cannot unmarshal ICS-20 transfer packet data: {e}"))
    })?;
    super::pfm::in_flight(ctx, p)?;
    let marshaled = ack.marshal();
    let bz = gostr::to_go_bytes(&marshaled);
    if *bz != *ack_bz {
        return Err(errs::invalid_type(format!(
            "acknowledgement did not marshal to expected bytes: {} \u{2260} {}",
            hex::encode_upper(&*bz),
            hex::encode_upper(ack_bz)
        )));
    }
    if let channel::AckResponse::Error(_) = ack {
        refund_packet_token(ctx, p, &data)?;
    }
    let packet_event = |attrs: Vec<(&str, String)>| event("fungible_token_packet", attrs);
    ctx.events.emit(packet_event(vec![
        ("module", x_transfer::MODULE_NAME.to_owned()),
        ("sender", data.sender.clone()),
        ("receiver", data.receiver.clone()),
        ("denom", data.denom.clone()),
        ("amount", data.amount.clone()),
        ("memo", data.memo.clone()),
        ("acknowledgement", tr::ack_text(&ack)),
    ]));
    match ack {
        channel::AckResponse::Result(r) => ctx
            .events
            .emit(packet_event(vec![("success", gostr::from_go_vec(r))])),
        channel::AckResponse::Error(e) => ctx.events.emit(packet_event(vec![("error", e)])),
        channel::AckResponse::Empty => {}
    }
    Ok(())
}

/// Transfer `refundPacketToken` (v6: no total-escrow tracking). Only the
/// unescrow of a native token is ported.
pub(super) fn refund_packet_token(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &tr::PacketData,
) -> Result<()> {
    let trace = tr::parse_denom_trace(&data.denom);
    let amount = sdk::coins::Amount::parse(&data.amount).map_err(|_| {
        tr::errors::invalid_amount(&format!(
            "unable to parse transfer amount ({}) into math.Int",
            data.amount
        ))
    })?;
    let token = tr::new_coin(&x_transfer::ibc_denom(&trace), amount)?;
    let sender = tr::acc_address(&data.sender)?;
    if !tr::sender_chain_is_source(&p.source_port, &p.source_channel, &data.denom) {
        return Err(super::not_ported(errs::not_supported(
            "the ibc-go v6 refund of a voucher (mint) is not ported",
        )));
    }
    let escrow = x_transfer::escrow_address(&p.source_port, &p.source_channel);
    x_bank::send_coins(ctx, &escrow, &sender, &tr::coins(&token)).map_err(|e| {
        e.wrap("unable to unescrow tokens, this may be caused by a malicious counterparty module or a bug: please open an issue on counterparty module")
    })
}

/// A v6 packet event (`recv_packet`, `write_acknowledgement`): the packet
/// data and fields, then `ack` (with its hex) or the channel ordering,
/// then the connection; and the `ibc_channel` message event.
fn emit_packet_data_event(
    ctx: &mut Context<'_>,
    ty: &str,
    p: &Packet,
    ch: &Channel,
    ack: Option<&[u8]>,
) {
    let (ordering, hop) = ordering_and_hop(ch);
    let mut attrs = vec![
        ("packet_data", gostr::from_go_bytes(&p.data).into_owned()),
        ("packet_data_hex", hex::encode(&p.data)),
        (
            "packet_timeout_height",
            height::to_string(&p.timeout_height.unwrap_or_default()),
        ),
        ("packet_timeout_timestamp", p.timeout_timestamp.to_string()),
        ("packet_sequence", p.sequence.to_string()),
        ("packet_src_port", p.source_port.clone()),
        ("packet_src_channel", p.source_channel.clone()),
        ("packet_dst_port", p.destination_port.clone()),
        ("packet_dst_channel", p.destination_channel.clone()),
    ];
    match ack {
        Some(a) => {
            attrs.push(("packet_ack", gostr::from_go_bytes(a).into_owned()));
            attrs.push(("packet_ack_hex", hex::encode(a)));
        }
        None => attrs.push(("packet_channel_ordering", ordering)),
    }
    attrs.push(("packet_connection", hop));
    ctx.events.emit(event(ty, attrs));
    ctx.events
        .emit(event("message", [("module", "ibc_channel")]));
}

/// 04-channel `RecvPacket` (v6).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn recv_packet(
    ctx: &mut Context<'_>,
    cap: &celestia_proto::capability::v1::Capability,
    p: &Packet,
    proof: &[u8],
    proof_height: &Height,
) -> core::result::Result<(), AckFailure> {
    let (port, chan) = (&p.destination_port, &p.destination_channel);
    let ch =
        channel::get_channel(ctx, port, chan)?.ok_or_else(|| cherr::channel_not_found(chan))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state is not OPEN (got {})",
            channel::state_name(ch.state)
        ))
        .into());
    }
    let cap_name = host::channel_capability_path(port, chan);
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        ))
        .into());
    }
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.source_port != cp.port_id {
        return Err(cherr::invalid_packet(format!(
            "packet source port doesn't match the counterparty's port ({} \u{2260} {})",
            p.source_port, cp.port_id
        ))
        .into());
    }
    if p.source_channel != cp.channel_id {
        return Err(cherr::invalid_packet(format!(
            "packet source channel doesn't match the counterparty's channel ({} \u{2260} {})",
            p.source_channel, cp.channel_id
        ))
        .into());
    }
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    let conn = x_ibc::connection::get_connection(ctx, &hop)?
        .ok_or_else(|| x_ibc::errors::connection::connection_not_found(&hop))?;
    if conn.state != celestia_proto::ibc::core::connection::v1::State::Open as i32 {
        return Err(x_ibc::errors::connection::invalid_connection_state(format!(
            "connection state is not OPEN (got {})",
            super::ibc::connection_state_name(conn.state)
        ))
        .into());
    }
    let self_height = x_ibc::client::get_self_height(ctx)?;
    let timeout_height = p.timeout_height.unwrap_or_default();
    if !height::is_zero(&timeout_height) && height::gte(&self_height, &timeout_height) {
        return Err(cherr::packet_timeout(format!(
            "block height >= packet timeout height ({} >= {})",
            height::to_string(&self_height),
            height::to_string(&timeout_height)
        ))
        .into());
    }
    let now = u64::try_from(sdk::time::to_nanos(&ctx.header.time)).unwrap_or(0);
    if p.timeout_timestamp != 0 && now >= p.timeout_timestamp {
        return Err(cherr::packet_timeout(format!(
            "block timestamp >= packet timeout timestamp ({} >= {})",
            sdk::time::go_time_string(&ctx.header.time),
            unix_nanos_string(p.timeout_timestamp)
        ))
        .into());
    }
    let commitment = channel::commit_packet(p);
    super::ibc::verify_with_delay(
        ctx,
        &conn,
        proof_height,
        proof,
        &host::packet_commitment_key(&p.source_port, &p.source_channel, p.sequence),
        &commitment,
        "failed packet commitment verification for client",
        true,
    )
    .map_err(|e| e.wrap("couldn't verify counterparty packet commitment"))?;
    if ch.ordering == chpb::Order::Ordered as i32 {
        let next = channel::get_next_sequence_recv(ctx, port, chan)?.ok_or_else(|| {
            cherr::sequence_receive_not_found(format!(
                "destination port: {port}, destination channel: {chan}"
            ))
        })?;
        if p.sequence < next {
            emit_packet_data_event(ctx, "recv_packet", p, &ch, None);
            return Err(AckFailure::NoOp);
        }
        if p.sequence != next {
            return Err(cherr::packet_sequence_out_of_order(format!(
                "packet sequence \u{2260} next receive sequence ({} \u{2260} {next})",
                p.sequence
            ))
            .into());
        }
        channel::set_next_sequence_recv(ctx, port, chan, next.wrapping_add(1))?;
    } else {
        if channel::has_packet_receipt(ctx, port, chan, p.sequence)? {
            emit_packet_data_event(ctx, "recv_packet", p, &ch, None);
            return Err(AckFailure::NoOp);
        }
        channel::set_packet_receipt(ctx, port, chan, p.sequence)?;
    }
    emit_packet_data_event(ctx, "recv_packet", p, &ch, None);
    Ok(())
}

/// `NewErrorAcknowledgement` (v6): only the ABCI code of `e`.
fn error_ack(e: &sdk::SdkError) -> channel::AckResponse {
    channel::AckResponse::Error(format!(
        "ABCI code: {}: error handling packet: see events for details",
        e.code
    ))
}

/// The transfer stack's `OnRecvPacket`: celestia-app's token filter, the
/// packet-forward middleware from app version 2 ([`super::pfm`]), then
/// transfer (v6).
fn on_recv_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<channel::AckResponse> {
    // Token filter: only a token returning to this chain passes.
    if let Ok(data) = tr::PacketData::decode(&p.data)
        && !tr::receiver_chain_is_source(&p.source_port, &p.source_channel, &data.denom)
    {
        let err = errs::invalid_type(format!(
            "only native denom transfers accepted, got {}",
            data.denom
        ));
        ctx.events.emit(event(
            "fungible_token_packet",
            [
                ("module", "tokenfilter".to_owned()),
                ("sender", data.sender.clone()),
                ("receiver", data.receiver.clone()),
                ("denom", data.denom.clone()),
                ("amount", data.amount.clone()),
                ("memo", data.memo.clone()),
                ("success", "false".to_owned()),
                ("error", err.log.clone()),
            ],
        ));
        return Ok(error_ack(&err));
    }
    // packet-forward-middleware (app versions 2 and 3).
    if let Ok(data) = tr::PacketData::decode(&p.data) {
        super::pfm::on_recv(ctx, &data.memo)?;
    }
    // transfer `IBCModule.OnRecvPacket`.
    let decoded = tr::PacketData::decode(&p.data);
    let (data, mut ack_err) = match decoded {
        Ok(d) => (d, None),
        Err(_) => (
            tr::PacketData::default(),
            Some(errs::invalid_type(
                "cannot unmarshal ICS-20 transfer packet data",
            )),
        ),
    };
    if ack_err.is_none()
        && let Err(e) = transfer_on_recv(ctx, p, &data)
    {
        // A not-ported path inside stays recorded and aborts the block.
        ack_err = Some(e);
    }
    let ack = match &ack_err {
        None => channel::AckResponse::Result(vec![1]),
        Some(e) => error_ack(e),
    };
    let mut attrs = vec![
        ("module", x_transfer::MODULE_NAME.to_owned()),
        ("sender", data.sender.clone()),
        ("receiver", data.receiver.clone()),
        ("denom", data.denom.clone()),
        ("amount", data.amount.clone()),
        ("memo", data.memo.clone()),
        ("success", ack.success().to_string()),
    ];
    if let Some(e) = &ack_err {
        attrs.push(("error", e.log.clone()));
    }
    ctx.events.emit(event("fungible_token_packet", attrs));
    Ok(ack)
}

/// transfer keeper `OnRecvPacket` (v6) for a token returning to this
/// chain (the token filter passes no other).
fn transfer_on_recv(ctx: &mut Context<'_>, p: &Packet, data: &tr::PacketData) -> Result<()> {
    let amount = sdk::coins::Amount::parse(&data.amount).map_err(|_| {
        tr::errors::invalid_amount(&format!(
            "unable to parse transfer amount ({}) into math.Int",
            data.amount
        ))
    })?;
    if !amount.is_positive() {
        return Err(tr::errors::invalid_amount(&format!(
            "amount must be strictly positive: got {amount}"
        )));
    }
    if data.sender.trim().is_empty() {
        return Err(errs::invalid_address("sender address cannot be blank"));
    }
    if data.receiver.trim().is_empty() {
        return Err(errs::invalid_address("receiver address cannot be blank"));
    }
    tr::validate_prefixed_denom(&data.denom)?;
    let raw = x_params::get_raw(ctx, "transfer", b"ReceiveEnabled")?
        .ok_or_else(|| sdk::error::go_panic("UnmarshalJSON: transfer ReceiveEnabled is not set"))?;
    let enabled: bool = serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace transfer: ReceiveEnabled: {e}")))?;
    if !enabled {
        return Err(tr::errors::receive_disabled(""));
    }
    let receiver = tr::acc_address(&data.receiver)?;
    if !tr::receiver_chain_is_source(&p.source_port, &p.source_channel, &data.denom) {
        return Err(super::not_ported(errs::not_supported(
            "the ibc-go v6 receive of a foreign token (mint) is not ported",
        )));
    }
    let prefix = format!("{}/{}/", p.source_port, p.source_channel);
    let unprefixed = &data.denom[prefix.len()..];
    let trace = tr::parse_denom_trace(unprefixed);
    let denom = if trace.path.is_empty() {
        unprefixed.to_owned()
    } else {
        x_transfer::ibc_denom(&trace)
    };
    let token = tr::new_coin(&denom, amount)?;
    if x_bank::is_blocked(&receiver) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive funds",
            data.receiver
        )));
    }
    let escrow = x_transfer::escrow_address(&p.destination_port, &p.destination_channel);
    x_bank::send_coins(ctx, &escrow, &receiver, &tr::coins(&token)).map_err(|e| {
        e.wrap("unable to unescrow tokens, this may be caused by a malicious counterparty module or a bug: please open an issue on counterparty module")
    })
}

/// core `msgServer.RecvPacket` (v6).
fn recv_packet_msg(ctx: &mut Context<'_>, m: &chpb::MsgRecvPacket) -> Result<Any> {
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) =
        channel::lookup_module_by_channel(ctx, &p.destination_port, &p.destination_channel)
            .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    if module != x_transfer::MODULE_NAME {
        return Err(super::not_ported(errs::not_supported(format!(
            "the ibc-go v6 packet callbacks of module {module} are not ported"
        ))));
    }
    // First cache context: the core receive.
    let outer = std::mem::take(&mut ctx.events);
    let cp = ctx.branch();
    let r = recv_packet(
        ctx,
        &cap,
        &p,
        &m.proof_commitment,
        &m.proof_height.unwrap_or_default(),
    );
    let mut cached = std::mem::replace(&mut ctx.events, outer);
    match r {
        Ok(()) => {
            ctx.write(cp);
            ctx.events.emit_all(cached.take());
        }
        Err(AckFailure::NoOp) => {
            ctx.multistore().discard_branch();
            return Ok(Any::pack(&chpb::MsgRecvPacketResponse {
                result: chpb::ResponseResultType::Noop as i32,
            }));
        }
        Err(AckFailure::Err(e)) => {
            ctx.multistore().discard_branch();
            return Err(e.wrap("receive packet verification failed"));
        }
    }
    // Second cache context: the application callback, kept only for a
    // successful acknowledgement.
    let outer = std::mem::take(&mut ctx.events);
    let base = ctx.multistore().depth();
    let cp = ctx.branch();
    let ack = on_recv_packet(ctx, &p);
    let mut cached = std::mem::replace(&mut ctx.events, outer);
    let ack = match ack {
        Ok(a) => a,
        Err(e) => {
            while ctx.multistore().depth() > base {
                ctx.multistore().discard_branch();
            }
            return Err(e);
        }
    };
    if ack.success() {
        ctx.write(cp);
        ctx.events.emit_all(cached.take());
    } else {
        while ctx.multistore().depth() > base {
            ctx.multistore().discard_branch();
        }
    }
    write_acknowledgement(ctx, &cap, &p, &ack)?;
    Ok(Any::pack(&chpb::MsgRecvPacketResponse {
        result: chpb::ResponseResultType::Success as i32,
    }))
}

/// 04-channel `WriteAcknowledgement` (v6).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn write_acknowledgement(
    ctx: &mut Context<'_>,
    cap: &celestia_proto::capability::v1::Capability,
    p: &Packet,
    ack: &channel::AckResponse,
) -> Result<()> {
    let (port, chan) = (&p.destination_port, &p.destination_channel);
    let ch =
        channel::get_channel(ctx, port, chan)?.ok_or_else(|| cherr::channel_not_found(chan))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state is not OPEN (got {})",
            channel::state_name(ch.state)
        )));
    }
    let cap_name = host::channel_capability_path(port, chan);
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        )));
    }
    if channel::has_packet_acknowledgement(ctx, port, chan, p.sequence)? {
        return Err(cherr::acknowledgement_exists(""));
    }
    let bz = gostr::to_go_bytes(&ack.marshal()).into_owned();
    channel::set_packet_acknowledgement(
        ctx,
        port,
        chan,
        p.sequence,
        &channel::commit_acknowledgement(&bz),
    )?;
    emit_packet_data_event(ctx, "write_acknowledgement", p, &ch, Some(&bz));
    Ok(())
}
