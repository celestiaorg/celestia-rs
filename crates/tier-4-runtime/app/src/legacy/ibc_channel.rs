//! ibc-go v6 (celestiaorg `v6.3.0`) channel handshake of celestia-app v1
//! to v3: `MsgChannelOpenInit`, `MsgChannelOpenTry`, `MsgChannelOpenAck`,
//! `MsgChannelOpenConfirm`, `MsgChannelCloseInit` and `MsgChannelCloseConfirm` with their v6
//! `ValidateBasic`, the 04-channel keeper steps, port and channel
//! capabilities, and the callbacks of the ICS-20 transfer stack (behind
//! celestia-app's token filter, which only filters received packets) and
//! of the interchain accounts host ([`super::ica_host`], app versions 2
//! and 3). Built on the store helpers of `x-ibc`; proofs go
//! through the v6 connection verification of [`super::ibc`].

// A capability is its 8-byte index, passed by reference as in `x-ibc`.
#![allow(clippy::trivially_copy_pass_by_ref)]

use celestia_proto::Any;
use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::core::channel::v1::{self as chpb, Channel, Counterparty, Order, State};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::connection::v1::ConnectionEnd;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_ibc::errors::{channel as cherr, port as porterr};
use x_ibc::{channel, height, host};

/// The ICS-20 version of ibc-go v6 transfer.
const ICS20_VERSION: &str = "ics20-1";

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `Order.String()`.
fn order_name(o: i32) -> String {
    channel::order_name(o).into_owned()
}

/// `State.String()` of a v6 channel state (no flushing states in v6).
fn state_name(s: i32) -> String {
    match State::try_from(s) {
        Ok(
            State::UninitializedUnspecified
            | State::Init
            | State::Tryopen
            | State::Open
            | State::Closed,
        ) => channel::state_name(s).into_owned(),
        _ => s.to_string(),
    }
}

fn check_signer(signer: &str) -> Result<()> {
    sdk::address::acc_from_bech32_go(signer)
        .map(|_| ())
        .map_err(|e| errs::invalid_address(format!("string could not be parsed as address: {e}")))
}

fn non_empty_proof(p: &[u8], log: &str) -> Result<()> {
    if p.is_empty() {
        return Err(x_ibc::errors::commitment::invalid_proof(log));
    }
    Ok(())
}

fn non_zero_proof_height(h: Option<&Height>) -> Result<()> {
    if height::is_zero(&h.copied().unwrap_or_default()) {
        return Err(errs::invalid_height("proof height must be non-zero"));
    }
    Ok(())
}

fn port_id(id: &str) -> Result<()> {
    host::port_identifier_validator(id).map_err(|e| e.wrap("invalid port ID"))
}

/// `Channel.ValidateBasic` (v6).
fn validate_channel(ch: &Channel) -> Result<()> {
    if ch.state == State::UninitializedUnspecified as i32 {
        return Err(cherr::invalid_channel_state(""));
    }
    if ch.ordering != Order::Ordered as i32 && ch.ordering != Order::Unordered as i32 {
        return Err(cherr::invalid_channel_ordering(order_name(ch.ordering)));
    }
    if ch.connection_hops.len() != 1 {
        return Err(cherr::too_many_connection_hops(
            "current IBC version only supports one connection hop",
        ));
    }
    host::connection_identifier_validator(&ch.connection_hops[0])
        .map_err(|e| e.wrap("invalid connection hop ID"))?;
    let cp = ch.counterparty.clone().unwrap_or_default();
    host::port_identifier_validator(&cp.port_id)
        .map_err(|e| e.wrap("invalid counterparty port ID"))?;
    if !cp.channel_id.is_empty() {
        host::channel_identifier_validator(&cp.channel_id)
            .map_err(|e| e.wrap("invalid counterparty channel ID"))?;
    }
    Ok(())
}

/// ibc-go v6 `ValidateBasic` of the channel handshake messages (`None`
/// for other messages).
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if msg.type_url == chpb::MsgChannelOpenInit::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelOpenInit| {
            port_id(&m.port_id)?;
            let ch = m.channel.clone().unwrap_or_default();
            if ch.state != State::Init as i32 {
                return Err(cherr::invalid_channel_state(format!(
                    "channel state must be INIT in MsgChannelOpenInit. expected: STATE_INIT, got: {}",
                    state_name(ch.state)
                )));
            }
            if !ch.counterparty.clone().unwrap_or_default().channel_id.is_empty() {
                return Err(cherr::invalid_counterparty(
                    "counterparty channel identifier must be empty",
                ));
            }
            check_signer(&m.signer)?;
            validate_channel(&ch)
        }));
    }
    if msg.type_url == chpb::MsgChannelOpenTry::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelOpenTry| {
            port_id(&m.port_id)?;
            #[allow(deprecated)]
            if !m.previous_channel_id.is_empty() {
                return Err(cherr::invalid_channel_identifier(
                    "previous channel identifier must be empty, this field has been deprecated as crossing hellos are no longer supported",
                ));
            }
            non_empty_proof(&m.proof_init, "cannot submit an empty proof init")?;
            non_zero_proof_height(m.proof_height.as_ref())?;
            let ch = m.channel.clone().unwrap_or_default();
            if ch.state != State::Tryopen as i32 {
                return Err(cherr::invalid_channel_state(format!(
                    "channel state must be TRYOPEN in MsgChannelOpenTry. expected: STATE_TRYOPEN, got: {}",
                    state_name(ch.state)
                )));
            }
            host::channel_identifier_validator(&ch.counterparty.clone().unwrap_or_default().channel_id)
                .map_err(|e| e.wrap("invalid counterparty channel ID"))?;
            check_signer(&m.signer)?;
            validate_channel(&ch)
        }));
    }
    if msg.type_url == chpb::MsgChannelOpenAck::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelOpenAck| {
            port_id(&m.port_id)?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            host::channel_identifier_validator(&m.counterparty_channel_id)
                .map_err(|e| e.wrap("invalid counterparty channel ID"))?;
            non_empty_proof(&m.proof_try, "cannot submit an empty proof try")?;
            non_zero_proof_height(m.proof_height.as_ref())?;
            check_signer(&m.signer)
        }));
    }
    if msg.type_url == chpb::MsgChannelOpenConfirm::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelOpenConfirm| {
            port_id(&m.port_id)?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            non_empty_proof(&m.proof_ack, "cannot submit an empty proof ack")?;
            non_zero_proof_height(m.proof_height.as_ref())?;
            check_signer(&m.signer)
        }));
    }
    if msg.type_url == chpb::MsgChannelCloseInit::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelCloseInit| {
            port_id(&m.port_id)?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            check_signer(&m.signer)
        }));
    }
    if msg.type_url == chpb::MsgChannelCloseConfirm::type_url() {
        return Some(decode(msg).and_then(|m: chpb::MsgChannelCloseConfirm| {
            port_id(&m.port_id)?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            non_empty_proof(&m.proof_init, "cannot submit an empty proof init")?;
            non_zero_proof_height(m.proof_height.as_ref())?;
            check_signer(&m.signer)
        }));
    }
    None
}

/// Executes a channel handshake message (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if msg.type_url == chpb::MsgChannelOpenInit::type_url() {
        return Some(decode(msg).and_then(|m| chan_open_init_msg(ctx, &m)));
    }
    if msg.type_url == chpb::MsgChannelOpenTry::type_url() {
        return Some(decode(msg).and_then(|m| chan_open_try_msg(ctx, &m)));
    }
    if msg.type_url == chpb::MsgChannelOpenAck::type_url() {
        return Some(decode(msg).and_then(|m| chan_open_ack_msg(ctx, &m)));
    }
    if msg.type_url == chpb::MsgChannelOpenConfirm::type_url() {
        return Some(decode(msg).and_then(|m| chan_open_confirm_msg(ctx, &m)));
    }
    if msg.type_url == chpb::MsgChannelCloseInit::type_url() {
        return Some(decode(msg).and_then(|m| chan_close_init_msg(ctx, &m)));
    }
    if msg.type_url == chpb::MsgChannelCloseConfirm::type_url() {
        return Some(decode(msg).and_then(|m| chan_close_confirm_msg(ctx, &m)));
    }
    None
}

/// The app module a port or channel capability routes to: `true` for the
/// interchain accounts host (bound from app version 2), `false` for the
/// transfer stack. No other module is ported.
fn route_to(module: &str) -> Result<bool> {
    if module == x_transfer::MODULE_NAME {
        return Ok(false);
    }
    if module == super::ica_host::MODULE {
        return Ok(true);
    }
    Err(super::not_ported(errs::not_supported(format!(
        "the ibc-go v6 channel callbacks of module {module} are not ported"
    ))))
}

/// The `channel_open_*` event and the `ibc_channel` message event.
fn channel_events(
    ctx: &mut Context<'_>,
    ty: &str,
    port: &str,
    chan: &str,
    ch: &Channel,
    version: bool,
) {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let mut attrs = vec![
        ("port_id", port.to_owned()),
        ("channel_id", chan.to_owned()),
        ("counterparty_port_id", cp.port_id),
        ("counterparty_channel_id", cp.channel_id),
        (
            "connection_id",
            ch.connection_hops.first().cloned().unwrap_or_default(),
        ),
    ];
    if version {
        attrs.push(("version", ch.version.clone()));
    }
    ctx.events.emit(event(ty, attrs));
    ctx.events
        .emit(event("message", [("module", "ibc_channel")]));
}

/// `PortKeeper.Authenticate` (v6): panics on an invalid port ID.
fn authenticate_port(ctx: &mut Context<'_>, cap: &Capability, port: &str) -> Result<bool> {
    host::port_identifier_validator(port).map_err(|e| sdk::error::go_panic(e.log))?;
    x_ibc::port::SCOPED_KEEPER.authenticate_capability(ctx, cap, &host::port_path(port))
}

fn connection_end(ctx: &mut Context<'_>, id: &str) -> Result<ConnectionEnd> {
    x_ibc::connection::get_connection(ctx, id)?
        .ok_or_else(|| x_ibc::errors::connection::connection_not_found(id))
}

fn connection_open(c: &ConnectionEnd) -> Result<()> {
    if c.state != celestia_proto::ibc::core::connection::v1::State::Open as i32 {
        return Err(x_ibc::errors::connection::invalid_connection_state(
            format!(
                "connection state is not OPEN (got {})",
                super::ibc::connection_state_name(c.state)
            ),
        ));
    }
    Ok(())
}

fn new_channel_capability(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<Capability> {
    x_ibc::port::SCOPED_KEEPER
        .new_capability(ctx, &host::channel_capability_path(port, chan))
        .map_err(|e| {
            e.wrap(format!(
                "could not create channel capability for port ID {port} and channel ID {chan}"
            ))
        })
}

/// v6 `VerifyChannelState` of `conn`'s client: the channel end stored
/// on the counterparty at `port`/`chan`.
fn verify_channel_state(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    proof_height: &Height,
    proof: &[u8],
    port: &str,
    chan: &str,
    expected: &Channel,
) -> Result<()> {
    super::ibc::verify(
        ctx,
        conn,
        proof_height,
        proof,
        &host::channel_key(port, chan),
        &expected.encode_to_vec(),
        "failed channel state verification for client",
    )
}

fn get_channel(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<Channel> {
    channel::get_channel(ctx, port, chan)?
        .ok_or_else(|| cherr::channel_not_found(format!("port ID ({port}) channel ID ({chan})")))
}

fn authenticate_channel(
    ctx: &mut Context<'_>,
    cap: &Capability,
    port: &str,
    chan: &str,
) -> Result<()> {
    if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(
        ctx,
        cap,
        &host::channel_capability_path(port, chan),
    )? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    Ok(())
}

/// Transfer `ValidateTransferChannelParams` (`OnChanOpenInit` and
/// `OnChanOpenTry`).
fn transfer_channel_params(
    ctx: &mut Context<'_>,
    order: i32,
    port: &str,
    chan: &str,
) -> Result<()> {
    let order = Order::try_from(order).unwrap_or(Order::NoneUnspecified);
    x_transfer::relay::validate_channel_params(ctx, order, port, chan)
}

fn transfer_claim_capability(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cap: &Capability,
) -> Result<()> {
    x_transfer::SCOPED_KEEPER.claim_capability(ctx, cap, &host::channel_capability_path(port, chan))
}

fn lookup_port(ctx: &mut Context<'_>, port: &str) -> Result<(String, Capability)> {
    channel::lookup_module_by_port(ctx, port)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))
}

fn lookup_channel(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<(String, Capability)> {
    channel::lookup_module_by_channel(ctx, port, chan)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))
}

/// `MsgChannelOpenInit` (core msg server and 04-channel `ChanOpenInit`).
fn chan_open_init_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelOpenInit) -> Result<Any> {
    let (module, port_cap) = lookup_port(ctx, &m.port_id)?;
    let ica = route_to(&module)?;
    let ch = m.channel.clone().unwrap_or_default();
    let port = &m.port_id;
    let (chan, cap) = (|| -> Result<(String, Capability)> {
        let conn = connection_end(ctx, &ch.connection_hops[0])?;
        channel::single_version_supports(&conn, ch.ordering)?;
        if !authenticate_port(ctx, &port_cap, port)? {
            return Err(porterr::invalid_port(format!(
                "caller does not own port capability for port ID {port}"
            )));
        }
        let chan = channel::generate_channel_identifier(ctx)?;
        let cap = new_channel_capability(ctx, port, &chan)?;
        Ok((chan, cap))
    })()
    .map_err(|e| e.wrap("channel handshake open init failed"))?;
    let version = (|| -> Result<String> {
        if ica {
            return Err(super::ica_host::on_chan_open_init());
        }
        transfer_channel_params(ctx, ch.ordering, port, &chan)?;
        let version = if ch.version.trim().is_empty() {
            ICS20_VERSION
        } else {
            ch.version.as_str()
        };
        if version != ICS20_VERSION {
            return Err(x_transfer::relay::errors::invalid_version(&format!(
                "got {version}, expected {ICS20_VERSION}"
            )));
        }
        transfer_claim_capability(ctx, port, &chan, &cap)?;
        Ok(version.to_owned())
    })()
    .map_err(|e| {
        e.wrap(format!(
            "channel open init callback failed for port ID: {port}, channel ID: {chan}"
        ))
    })?;
    let written = Channel {
        state: State::Init as i32,
        ordering: ch.ordering,
        counterparty: Some(ch.counterparty.clone().unwrap_or_default()),
        connection_hops: ch.connection_hops.clone(),
        version: version.clone(),
        upgrade_sequence: 0,
    };
    channel::set_channel(ctx, port, &chan, &written)?;
    channel::set_next_sequence_send(ctx, port, &chan, 1)?;
    channel::set_next_sequence_recv(ctx, port, &chan, 1)?;
    channel::set_next_sequence_ack(ctx, port, &chan, 1)?;
    channel_events(ctx, "channel_open_init", port, &chan, &written, true);
    Ok(Any::pack(&chpb::MsgChannelOpenInitResponse {
        channel_id: chan,
        version,
    }))
}

/// `MsgChannelOpenTry` (core msg server and 04-channel `ChanOpenTry`).
fn chan_open_try_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelOpenTry) -> Result<Any> {
    let (module, port_cap) = lookup_port(ctx, &m.port_id)?;
    let ica = route_to(&module)?;
    let ch = m.channel.clone().unwrap_or_default();
    let cp = ch.counterparty.clone().unwrap_or_default();
    let port = &m.port_id;
    let proof_height = m.proof_height.unwrap_or_default();
    let (chan, cap) = (|| -> Result<(String, Capability)> {
        if ch.connection_hops.len() != 1 {
            return Err(cherr::too_many_connection_hops(format!(
                "expected 1, got {}",
                ch.connection_hops.len()
            )));
        }
        let chan = channel::generate_channel_identifier(ctx)?;
        if !authenticate_port(ctx, &port_cap, port)? {
            return Err(porterr::invalid_port(format!(
                "caller does not own port capability for port ID {port}"
            )));
        }
        let conn = connection_end(ctx, &ch.connection_hops[0])?;
        connection_open(&conn)?;
        channel::single_version_supports(&conn, ch.ordering)?;
        let expected = Channel {
            state: State::Init as i32,
            ordering: ch.ordering,
            counterparty: Some(Counterparty {
                port_id: port.clone(),
                channel_id: String::new(),
            }),
            connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
            version: m.counterparty_version.clone(),
            upgrade_sequence: 0,
        };
        verify_channel_state(
            ctx,
            &conn,
            &proof_height,
            &m.proof_init,
            &cp.port_id,
            &cp.channel_id,
            &expected,
        )?;
        let cap = new_channel_capability(ctx, port, &chan)?;
        Ok((chan, cap))
    })()
    .map_err(|e| e.wrap("channel handshake open try failed"))?;
    let version = (|| -> Result<String> {
        if ica {
            return super::ica_host::on_chan_open_try(
                ctx,
                ch.ordering,
                &ch.connection_hops,
                port,
                &chan,
                &cap,
                &cp,
                &m.counterparty_version,
            );
        }
        transfer_channel_params(ctx, ch.ordering, port, &chan)?;
        if m.counterparty_version != ICS20_VERSION {
            return Err(x_transfer::relay::errors::invalid_version(&format!(
                "invalid counterparty version: got: {}, expected {ICS20_VERSION}",
                m.counterparty_version
            )));
        }
        transfer_claim_capability(ctx, port, &chan, &cap)?;
        Ok(ICS20_VERSION.to_owned())
    })()
    .map_err(|e| {
        e.wrap(format!(
            "channel open try callback failed for port ID: {port}, channel ID: {chan}"
        ))
    })?;
    channel::set_next_sequence_send(ctx, port, &chan, 1)?;
    channel::set_next_sequence_recv(ctx, port, &chan, 1)?;
    channel::set_next_sequence_ack(ctx, port, &chan, 1)?;
    let written = Channel {
        state: State::Tryopen as i32,
        ordering: ch.ordering,
        counterparty: Some(cp),
        connection_hops: ch.connection_hops.clone(),
        version: version.clone(),
        upgrade_sequence: 0,
    };
    channel::set_channel(ctx, port, &chan, &written)?;
    channel_events(ctx, "channel_open_try", port, &chan, &written, true);
    Ok(Any::pack(&chpb::MsgChannelOpenTryResponse {
        version,
        channel_id: String::new(),
    }))
}

/// `MsgChannelOpenAck` (core msg server and 04-channel `ChanOpenAck`).
fn chan_open_ack_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelOpenAck) -> Result<Any> {
    let (port, chan) = (&m.port_id, &m.channel_id);
    let (module, cap) = lookup_channel(ctx, port, chan)?;
    let ica = route_to(&module)?;
    let proof_height = m.proof_height.unwrap_or_default();
    (|| -> Result<()> {
        let ch = get_channel(ctx, port, chan)?;
        if ch.state != State::Init as i32 {
            return Err(cherr::invalid_channel_state(format!(
                "channel state should be INIT (got {})",
                state_name(ch.state)
            )));
        }
        authenticate_channel(ctx, &cap, port, chan)?;
        let hop = ch.connection_hops.first().cloned().unwrap_or_default();
        let conn = connection_end(ctx, &hop)?;
        connection_open(&conn)?;
        let expected = Channel {
            state: State::Tryopen as i32,
            ordering: ch.ordering,
            counterparty: Some(Counterparty {
                port_id: port.clone(),
                channel_id: chan.clone(),
            }),
            connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
            version: m.counterparty_version.clone(),
            upgrade_sequence: 0,
        };
        verify_channel_state(
            ctx,
            &conn,
            &proof_height,
            &m.proof_try,
            &ch.counterparty.clone().unwrap_or_default().port_id,
            &m.counterparty_channel_id,
            &expected,
        )
    })()
    .map_err(|e| e.wrap("channel handshake open ack failed"))?;
    if ica {
        return Err(super::ica_host::on_chan_open_ack().wrap(format!(
            "channel open ack callback failed for port ID: {port}, channel ID: {chan}"
        )));
    }
    if m.counterparty_version != ICS20_VERSION {
        return Err(x_transfer::relay::errors::invalid_version(&format!(
            "invalid counterparty version: {}, expected {ICS20_VERSION}",
            m.counterparty_version
        ))
        .wrap(format!(
            "channel open ack callback failed for port ID: {port}, channel ID: {chan}"
        )));
    }
    // `WriteOpenAckChannel` reads the channel again.
    let mut ch = get_channel(ctx, port, chan)?;
    ch.state = State::Open as i32;
    ch.version.clone_from(&m.counterparty_version);
    let mut cp = ch.counterparty.clone().unwrap_or_default();
    cp.channel_id.clone_from(&m.counterparty_channel_id);
    ch.counterparty = Some(cp);
    channel::set_channel(ctx, port, chan, &ch)?;
    channel_events(ctx, "channel_open_ack", port, chan, &ch, false);
    Ok(Any::pack(&chpb::MsgChannelOpenAckResponse {}))
}

/// `MsgChannelOpenConfirm` (core msg server and 04-channel
/// `ChanOpenConfirm`).
fn chan_open_confirm_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelOpenConfirm) -> Result<Any> {
    let (port, chan) = (&m.port_id, &m.channel_id);
    let (module, cap) = lookup_channel(ctx, port, chan)?;
    let ica = route_to(&module)?;
    let proof_height = m.proof_height.unwrap_or_default();
    (|| -> Result<()> {
        let ch = get_channel(ctx, port, chan)?;
        if ch.state != State::Tryopen as i32 {
            return Err(cherr::invalid_channel_state(format!(
                "channel state is not TRYOPEN (got {})",
                state_name(ch.state)
            )));
        }
        authenticate_channel(ctx, &cap, port, chan)?;
        let hop = ch.connection_hops.first().cloned().unwrap_or_default();
        let conn = connection_end(ctx, &hop)?;
        connection_open(&conn)?;
        let cp = ch.counterparty.clone().unwrap_or_default();
        let expected = Channel {
            state: State::Open as i32,
            ordering: ch.ordering,
            counterparty: Some(Counterparty {
                port_id: port.clone(),
                channel_id: chan.clone(),
            }),
            connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
            version: ch.version.clone(),
            upgrade_sequence: 0,
        };
        verify_channel_state(
            ctx,
            &conn,
            &proof_height,
            &m.proof_ack,
            &cp.port_id,
            &cp.channel_id,
            &expected,
        )
    })()
    .map_err(|e| e.wrap("channel handshake open confirm failed"))?;
    // `OnChanOpenConfirm`: the host sets the active channel; transfer does
    // nothing. `WriteOpenConfirmChannel` reads the channel again.
    if ica {
        super::ica_host::on_chan_open_confirm(ctx, port, chan).map_err(|e| {
            e.wrap(format!(
                "channel open confirm callback failed for port ID: {port}, channel ID: {chan}"
            ))
        })?;
    }
    let mut ch = get_channel(ctx, port, chan)?;
    ch.state = State::Open as i32;
    channel::set_channel(ctx, port, chan, &ch)?;
    channel_events(ctx, "channel_open_confirm", port, chan, &ch, false);
    Ok(Any::pack(&chpb::MsgChannelOpenConfirmResponse {}))
}

/// `MsgChannelCloseInit` (core msg server). The callback runs first, and
/// in v6 both transfer's and the interchain accounts host's
/// `OnChanCloseInit` refuse (the token filter and packet-forward
/// middlewares pass it on), so 04-channel `ChanCloseInit` never runs.
fn chan_close_init_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelCloseInit) -> Result<Any> {
    let (port, chan) = (&m.port_id, &m.channel_id);
    let (module, _) = lookup_channel(ctx, port, chan)?;
    route_to(&module)?;
    Err(
        errs::invalid_request("user cannot close channel").wrap(format!(
            "channel close init callback failed for port ID: {port}, channel ID: {chan}"
        )),
    )
}

/// `MsgChannelCloseConfirm` (core msg server, then 04-channel
/// `ChanCloseConfirm`). The callback runs first: in v6 both transfer's
/// and the interchain accounts host's `OnChanCloseConfirm` do nothing.
fn chan_close_confirm_msg(ctx: &mut Context<'_>, m: &chpb::MsgChannelCloseConfirm) -> Result<Any> {
    let (port, chan) = (&m.port_id, &m.channel_id);
    let (module, cap) = lookup_channel(ctx, port, chan)?;
    route_to(&module)?;
    let proof_height = m.proof_height.unwrap_or_default();
    let ch = (|| -> Result<Channel> {
        if !x_ibc::port::SCOPED_KEEPER.authenticate_capability(
            ctx,
            &cap,
            &host::channel_capability_path(port, chan),
        )? {
            // Go's `Wrap` keeps the format verbs.
            return Err(cherr::channel_capability_not_found(
                "caller does not own capability for channel, port ID (%s) channel ID (%s)",
            ));
        }
        let mut ch = get_channel(ctx, port, chan)?;
        if ch.state == State::Closed as i32 {
            return Err(cherr::invalid_channel_state("channel is already CLOSED"));
        }
        let hop = ch.connection_hops.first().cloned().unwrap_or_default();
        let conn = connection_end(ctx, &hop)?;
        connection_open(&conn)?;
        let cp = ch.counterparty.clone().unwrap_or_default();
        let expected = Channel {
            state: State::Closed as i32,
            ordering: ch.ordering,
            counterparty: Some(Counterparty {
                port_id: port.clone(),
                channel_id: chan.clone(),
            }),
            connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
            version: ch.version.clone(),
            upgrade_sequence: 0,
        };
        verify_channel_state(
            ctx,
            &conn,
            &proof_height,
            &m.proof_init,
            &cp.port_id,
            &cp.channel_id,
            &expected,
        )?;
        ch.state = State::Closed as i32;
        channel::set_channel(ctx, port, chan, &ch)?;
        Ok(ch)
    })()
    .map_err(|e| e.wrap("channel handshake close confirm failed"))?;
    channel_events(ctx, "channel_close_confirm", port, chan, &ch, false);
    Ok(Any::pack(&chpb::MsgChannelCloseConfirmResponse {}))
}
