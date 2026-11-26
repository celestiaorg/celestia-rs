//! ibc-go v6 (celestiaorg `v6.3.0`) ICS-20 `MsgTransfer` of celestia-app
//! v1: its v6 `ValidateBasic`, the transfer msg server (`x/params`
//! `SendEnabled`, v0.46 bank send-enabled check, escrow or burn without
//! v7's total-escrow tracking) and 04-channel `SendPacket` with the v6
//! checks and event. The packet-forward middleware keeper that wraps
//! `SendPacket` passes it through.

use celestia_proto::Any;
use celestia_proto::ibc::applications::transfer::v1 as trpb;
use celestia_proto::ibc::core::channel::v1::{Channel, Packet, State};
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
    None
}

/// Executes `MsgTransfer` (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if msg.type_url == trpb::MsgTransfer::type_url() {
        return Some(decode(msg).and_then(|m| transfer(ctx, &m)));
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
fn validate_packet(p: &Packet) -> Result<()> {
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
