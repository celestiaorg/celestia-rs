//! ibc-go v6 (celestiaorg `v6.3.0`) `MsgTimeout` of celestia-app v1: its
//! `ValidateBasic`, the core msg server (the no-op of an already timed-out
//! packet), 04-channel `TimeoutPacket` and `TimeoutExecuted`, and the
//! transfer callback behind the token filter (the refund and the `timeout`
//! event). Only unordered channels and tendermint clients are ported: the
//! next-sequence-receive proof of an ordered channel aborts the block.

use celestia_proto::Any;
use celestia_proto::ibc::core::channel::v1::{self as chpb, Channel, Packet, State};
use celestia_proto::ibc::core::client::v1::Height;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_ibc::errors::{channel as cherr, client as clerr};
use x_ibc::lightclients::tendermint as tmc;
use x_ibc::{channel, height, host};
use x_transfer::relay as tr;

use super::ibc_transfer::{go_byte_slice, refund_packet_token, validate_packet};

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `MsgTimeout.ValidateBasic` (v6); `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == chpb::MsgTimeout::type_url()).then(|| {
        let m: chpb::MsgTimeout = decode(msg)?;
        if m.proof_unreceived.is_empty() {
            return Err(x_ibc::errors::commitment::invalid_proof(
                "cannot submit an empty unreceived proof",
            ));
        }
        if height::is_zero(&m.proof_height.unwrap_or_default()) {
            return Err(errs::invalid_height("proof height must be non-zero"));
        }
        if m.next_sequence_recv == 0 {
            return Err(errs::invalid_sequence("next sequence receive cannot be 0"));
        }
        sdk::address::acc_from_bech32_go(&m.signer).map_err(|e| {
            errs::invalid_address(format!("string could not be parsed as address: {e}"))
        })?;
        validate_packet(&m.packet.unwrap_or_default())
    })
}

/// Executes `MsgTimeout`; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == chpb::MsgTimeout::type_url())
        .then(|| decode(msg).and_then(|m| timeout(ctx, &m)))
}

/// `EmitTimeoutPacketEvent` (v6).
fn emit_timeout_packet(ctx: &mut Context<'_>, p: &Packet, ch: &Channel) {
    ctx.events.emit(event(
        "timeout_packet",
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
            (
                "packet_channel_ordering",
                channel::order_name(ch.ordering).into_owned(),
            ),
        ],
    ));
    ctx.events
        .emit(event("message", [("module", "ibc_channel")]));
}

/// Why `TimeoutPacket` did not apply.
enum Failure {
    /// `ErrNoOpMsg`: the packet commitment is gone.
    NoOp,
    Err(sdk::SdkError),
}

impl From<sdk::SdkError> for Failure {
    fn from(e: sdk::SdkError) -> Self {
        Self::Err(e)
    }
}

/// `uint64(t.UnixNano())` (Go's conversions wrap).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn go_unix_nano_u64(t: &pbjson_types::Timestamp) -> u64 {
    sdk::time::to_nanos(t) as i64 as u64
}

fn channel_not_found(port: &str, chan: &str) -> sdk::SdkError {
    cherr::channel_not_found(format!("port ID ({port}) channel ID ({chan})"))
}

/// 04-channel `TimeoutPacket` (v6).
fn timeout_packet(
    ctx: &mut Context<'_>,
    p: &Packet,
    proof: &[u8],
    proof_height: &Height,
    next_sequence_recv: u64,
) -> core::result::Result<(), Failure> {
    let (port, chan) = (&p.source_port, &p.source_channel);
    let ch = channel::get_channel(ctx, port, chan)?.ok_or_else(|| channel_not_found(port, chan))?;
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
    // `GetTimestampAtHeight`: the client's consensus state at the proof
    // height (one store read). Only a tendermint consensus state decodes
    // here; another client type's is not ported.
    let stored = match tmc::get_consensus_state(ctx, &conn.client_id, proof_height) {
        Ok(cs) => cs,
        // The read itself ran out of gas.
        Err(e) if ctx.gas.first_out_of_gas().is_some() => return Err(e.into()),
        Err(e) => {
            return Err(super::not_ported(errs::not_supported(format!(
                "the ibc-go v6 consensus state of this client type is not ported: {}",
                e.log
            )))
            .into());
        }
    };
    let proof_ts = stored
        .ok_or_else(|| {
            clerr::consensus_state_not_found(format!(
                "clientID ({}), height ({})",
                conn.client_id,
                height::to_string(proof_height)
            ))
        })?
        .timestamp
        .map_or(0, |t| go_unix_nano_u64(&t));
    let timeout_height = p.timeout_height.unwrap_or_default();
    if (height::is_zero(&timeout_height) || height::lt(proof_height, &timeout_height))
        && (p.timeout_timestamp == 0 || proof_ts < p.timeout_timestamp)
    {
        return Err(cherr::packet_timeout(
            "packet timeout has not been reached for height or timestamp",
        )
        .into());
    }
    let commitment = channel::get_packet_commitment(ctx, port, chan, p.sequence)?;
    if commitment.is_empty() {
        emit_timeout_packet(ctx, p, &ch);
        return Err(Failure::NoOp);
    }
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state is not OPEN (got {})",
            channel::state_name(ch.state)
        ))
        .into());
    }
    let packet_commitment = channel::commit_packet(p);
    if commitment != packet_commitment {
        return Err(cherr::invalid_packet(format!(
            "packet commitment bytes are not equal: got ({}), expected ({})",
            go_byte_slice(&commitment),
            go_byte_slice(&packet_commitment)
        ))
        .into());
    }
    if ch.ordering == chpb::Order::Unordered as i32 {
        super::ibc::verify_absence_with_delay(
            ctx,
            &conn,
            proof_height,
            proof,
            &host::packet_receipt_key(&p.destination_port, &p.destination_channel, p.sequence),
            "failed packet receipt absence verification for client",
        )?;
    } else if ch.ordering == chpb::Order::Ordered as i32 {
        let _ = next_sequence_recv;
        return Err(super::not_ported(errs::not_supported(
            "the ibc-go v6 timeout of an ordered channel is not ported",
        ))
        .into());
    } else {
        return Err(sdk::error::go_panic(
            cherr::invalid_channel_ordering(channel::order_name(ch.ordering)).log,
        )
        .into());
    }
    Ok(())
}

/// 04-channel `TimeoutExecuted` (v6) of an unordered channel.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn timeout_executed(
    ctx: &mut Context<'_>,
    cap: &celestia_proto::capability::v1::Capability,
    p: &Packet,
) -> Result<()> {
    let (port, chan) = (&p.source_port, &p.source_channel);
    let ch = channel::get_channel(ctx, port, chan)?.ok_or_else(|| channel_not_found(port, chan))?;
    let cap_name = host::channel_capability_path(port, chan);
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel with capability name {cap_name}"
        )));
    }
    ctx.delete(
        x_ibc::STORE_KEY,
        &host::packet_commitment_key(port, chan, p.sequence),
    )?;
    if ch.ordering == chpb::Order::Ordered as i32 {
        return Err(super::not_ported(errs::not_supported(
            "the ibc-go v6 timeout of an ordered channel is not ported",
        )));
    }
    emit_timeout_packet(ctx, p, &ch);
    Ok(())
}

/// Transfer `OnTimeoutPacket` (v6; the token filter passes it).
fn on_timeout_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<()> {
    let data = tr::PacketData::decode(&p.data).map_err(|e| {
        errs::unknown_request(format!("cannot unmarshal ICS-20 transfer packet data: {e}"))
    })?;
    refund_packet_token(ctx, p, &data)?;
    ctx.events.emit(event(
        "timeout",
        [
            ("module", x_transfer::MODULE_NAME.to_owned()),
            ("refund_receiver", data.sender.clone()),
            ("refund_denom", data.denom.clone()),
            ("refund_amount", data.amount.clone()),
            ("memo", data.memo.clone()),
        ],
    ));
    Ok(())
}

/// core `msgServer.Timeout` (v6).
fn timeout(ctx: &mut Context<'_>, m: &chpb::MsgTimeout) -> Result<Any> {
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
    let r = timeout_packet(
        ctx,
        &p,
        &m.proof_unreceived,
        &m.proof_height.unwrap_or_default(),
        m.next_sequence_recv,
    );
    let mut cache_events = std::mem::replace(&mut ctx.events, outer_events);
    match r {
        Ok(()) => {
            ctx.write(cp);
            ctx.events.emit_all(cache_events.take());
        }
        Err(Failure::NoOp) => {
            ctx.multistore().discard_branch();
            return Ok(Any::pack(&chpb::MsgTimeoutResponse {
                result: chpb::ResponseResultType::Noop as i32,
            }));
        }
        Err(Failure::Err(e)) => {
            ctx.multistore().discard_branch();
            return Err(e.wrap("timeout packet verification failed"));
        }
    }
    on_timeout_packet(ctx, &p).map_err(|e| e.wrap("timeout packet callback failed"))?;
    timeout_executed(ctx, &cap, &p)?;
    Ok(Any::pack(&chpb::MsgTimeoutResponse {
        result: chpb::ResponseResultType::Success as i32,
    }))
}
