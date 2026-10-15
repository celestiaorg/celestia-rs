//! ICS-04 channel submodule (`core/04-channel`): channel and packet
//! storage, params and genesis.

use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::core::channel::v1::{
    Channel, Counterparty, GenesisState, Order, Packet, Params, State, Timeout, Upgrade,
};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::connection::v1::{ConnectionEnd, State as ConnState};
use prost::Message;
use sdk::{Context, Result};

use crate::errors::{channel as cherr, client as clerr, connection as conerr, port as porterr};
use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "channel";
/// `KeyNextChannelSequence`.
pub const KEY_NEXT_CHANNEL_SEQUENCE: &[u8] = b"nextChannelSequence";
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"channelParams";
/// `ChannelPrefix`.
pub const CHANNEL_PREFIX: &str = "channel-";

pub(crate) fn put_u64(ctx: &mut Context<'_>, key: Vec<u8>, v: u64) -> Result<()> {
    ctx.set(crate::STORE_KEY, key, v.to_be_bytes().to_vec())
}

/// Fills gogoproto non-nullable embedded messages so they are always
/// encoded (`upgrade_timeout`, `upgrade_timeout.height`).
#[must_use]
pub fn normalize_params(mut p: Params) -> Params {
    let mut t = p.upgrade_timeout.take().unwrap_or_default();
    if t.height.is_none() {
        t.height = Some(Height::default());
    }
    p.upgrade_timeout = Some(t);
    p
}

/// `Params.Validate`: the upgrade timeout must be timestamp-only.
pub fn validate_params(p: &Params) -> Result<()> {
    let t: Timeout = p.upgrade_timeout.unwrap_or_default();
    let h = t.height.unwrap_or_default();
    if h.revision_number != 0 || h.revision_height != 0 {
        return Err(crate::errors::channel::invalid_upgrade_timeout(format!(
            "upgrade timeout height must be zero. got : {}",
            host::height_string(&h)
        )));
    }
    if t.timestamp == 0 {
        return Err(crate::errors::channel::invalid_upgrade_timeout(
            "upgrade timeout timestamp invalid: 0",
        ));
    }
    Ok(())
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        PARAMS_KEY.to_vec(),
        normalize_params(*p).encode_to_vec(),
    )
}

/// `GetParams`.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(crate::STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("ibc channel params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `SetChannel`.
pub fn set_channel(
    ctx: &mut Context<'_>,
    port_id: &str,
    channel_id: &str,
    ch: &Channel,
) -> Result<()> {
    let mut ch = ch.clone();
    if ch.counterparty.is_none() {
        ch.counterparty = Some(Counterparty::default());
    }
    ctx.set(
        crate::STORE_KEY,
        host::channel_key(port_id, channel_id),
        ch.encode_to_vec(),
    )
}

/// `GetChannel`.
pub fn get_channel(
    ctx: &mut Context<'_>,
    port_id: &str,
    channel_id: &str,
) -> Result<Option<Channel>> {
    ctx.get(crate::STORE_KEY, &host::channel_key(port_id, channel_id))?
        .map(|b| {
            Channel::decode(b.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `SetNextChannelSequence`.
pub fn set_next_channel_sequence(ctx: &mut Context<'_>, seq: u64) -> Result<()> {
    put_u64(ctx, KEY_NEXT_CHANNEL_SEQUENCE.to_vec(), seq)
}

/// `GetNextChannelSequence`.
pub fn get_next_channel_sequence(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx
        .get(crate::STORE_KEY, KEY_NEXT_CHANNEL_SEQUENCE)?
        .unwrap_or_default();
    crate::be_u64(&bz).ok_or_else(|| sdk::error::sdk::logic("next channel sequence is nil"))
}

/// `SetNextSequenceSend`.
pub fn set_next_sequence_send(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_send_key(port, chan), seq)
}

/// `SetNextSequenceRecv`.
pub fn set_next_sequence_recv(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_recv_key(port, chan), seq)
}

/// `SetNextSequenceAck`.
pub fn set_next_sequence_ack(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_ack_key(port, chan), seq)
}

/// `SetPacketReceipt` (value `0x01`).
pub fn set_packet_receipt(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_receipt_key(port, chan, seq),
        vec![1],
    )
}

/// `SetPacketCommitment`.
pub fn set_packet_commitment(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
    hash: &[u8],
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_commitment_key(port, chan, seq),
        hash.to_vec(),
    )
}

/// `SetPacketAcknowledgement`.
pub fn set_packet_acknowledgement(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
    hash: &[u8],
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_acknowledgement_key(port, chan, seq),
        hash.to_vec(),
    )
}

/// Channel `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let params = gs.params.unwrap_or_default();
    validate_params(&params).map_err(|e| e.wrap("invalid ibc channel genesis state parameters"))?;
    set_params(ctx, &params)?;
    for c in &gs.channels {
        // `types.NewChannel` drops the upgrade sequence.
        let ch = Channel {
            state: c.state,
            ordering: c.ordering,
            counterparty: c.counterparty.clone(),
            connection_hops: c.connection_hops.clone(),
            version: c.version.clone(),
            upgrade_sequence: 0,
        };
        set_channel(ctx, &c.port_id, &c.channel_id, &ch)?;
    }
    for a in &gs.acknowledgements {
        set_packet_acknowledgement(ctx, &a.port_id, &a.channel_id, a.sequence, &a.data)?;
    }
    for c in &gs.commitments {
        set_packet_commitment(ctx, &c.port_id, &c.channel_id, c.sequence, &c.data)?;
    }
    for r in &gs.receipts {
        set_packet_receipt(ctx, &r.port_id, &r.channel_id, r.sequence)?;
    }
    for s in &gs.send_sequences {
        set_next_sequence_send(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.recv_sequences {
        set_next_sequence_recv(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.ack_sequences {
        set_next_sequence_ack(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    set_next_channel_sequence(ctx, gs.next_channel_sequence)
}

// ---------------------------------------------------------------------------
// Keeper: identifiers, lookups and packet state
// ---------------------------------------------------------------------------

/// `IsValidChannelID`: `^channel-[0-9]{1,20}$` with a `uint64` sequence.
#[must_use]
pub fn is_valid_channel_id(id: &str) -> bool {
    id.strip_prefix(CHANNEL_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.len() <= 20 && crate::height::parse_u64(n).is_some())
}

/// `GenerateChannelIdentifier`.
pub fn generate_channel_identifier(ctx: &mut Context<'_>) -> Result<String> {
    let seq = get_next_channel_sequence(ctx)?;
    set_next_channel_sequence(ctx, seq.wrapping_add(1))?;
    Ok(format!("{CHANNEL_PREFIX}{seq}"))
}

fn get_u64(ctx: &mut Context<'_>, key: &[u8]) -> Result<Option<u64>> {
    let bz = ctx.get(crate::STORE_KEY, key)?.unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::be_u64(&bz).unwrap_or_default()))
}

/// `GetNextSequenceSend`.
pub fn get_next_sequence_send(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<u64>> {
    get_u64(ctx, &host::next_sequence_send_key(port, chan))
}

/// `GetNextSequenceRecv`.
pub fn get_next_sequence_recv(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<u64>> {
    get_u64(ctx, &host::next_sequence_recv_key(port, chan))
}

/// `GetNextSequenceAck`.
pub fn get_next_sequence_ack(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<Option<u64>> {
    get_u64(ctx, &host::next_sequence_ack_key(port, chan))
}

/// `GetPacketCommitment`.
pub fn get_packet_commitment(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<Vec<u8>> {
    Ok(ctx
        .get(
            crate::STORE_KEY,
            &host::packet_commitment_key(port, chan, seq),
        )?
        .unwrap_or_default())
}

/// `HasPacketAcknowledgement`.
pub fn has_packet_acknowledgement(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<bool> {
    ctx.has(
        crate::STORE_KEY,
        &host::packet_acknowledgement_key(port, chan, seq),
    )
}

fn delete_packet_commitment(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> Result<()> {
    ctx.delete(
        crate::STORE_KEY,
        &host::packet_commitment_key(port, chan, seq),
    )
}

pub(crate) fn channel_upgrade_path(kind: &str, port: &str, chan: &str) -> Vec<u8> {
    format!("channelUpgrades/{kind}/ports/{port}/channels/{chan}").into_bytes()
}

/// `GetCounterpartyUpgrade`.
pub fn get_counterparty_upgrade(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<Upgrade>> {
    ctx.get(
        crate::STORE_KEY,
        &channel_upgrade_path("counterpartyUpgrade", port, chan),
    )?
    .map(|b| Upgrade::decode(b.as_slice()).map_err(crate::errors::must_unmarshal))
    .transpose()
}

/// `GetRecvStartSequence`.
pub fn get_recv_start_sequence(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<u64>> {
    get_u64(
        ctx,
        format!("recvStartSequence/ports/{port}/channels/{chan}").as_bytes(),
    )
}

/// `GetPacketReceipt`.
pub fn has_packet_receipt(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> Result<bool> {
    let bz = ctx
        .get(crate::STORE_KEY, &host::packet_receipt_key(port, chan, seq))?
        .unwrap_or_default();
    Ok(!bz.is_empty())
}

/// The `ibc` scoped capability keeper.
const SCOPED: x_capability::ScopedKeeper = crate::port::SCOPED_KEEPER;

/// `GetModuleOwner`: the owner that is not `ibc` (Go panics unless there
/// are exactly two owners).
pub fn get_module_owner(modules: &[String]) -> Result<String> {
    if modules.len() != 2 {
        return Err(sdk::error::go_panic(format!(
            "capability should only be owned by port or channel owner and ibc module, multiple owners currently not supported, owners: {modules:?}"
        )));
    }
    Ok(if modules[0] == "ibc" {
        modules[1].clone()
    } else {
        modules[0].clone()
    })
}

/// `LookupModuleByChannel`.
pub fn lookup_module_by_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<(String, Capability)> {
    let (mods, cap) = SCOPED.lookup_modules(ctx, &host::channel_capability_path(port, chan))?;
    Ok((get_module_owner(&mods)?, cap))
}

/// `LookupModuleByPort`.
pub fn lookup_module_by_port(ctx: &mut Context<'_>, port: &str) -> Result<(String, Capability)> {
    let (mods, cap) = SCOPED.lookup_modules(ctx, &host::port_path(port))?;
    Ok((get_module_owner(&mods)?, cap))
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn authenticate_channel(
    ctx: &mut Context<'_>,
    cap: &Capability,
    port: &str,
    chan: &str,
) -> Result<bool> {
    SCOPED.authenticate_capability(ctx, cap, &host::channel_capability_path(port, chan))
}

/// `CommitPacket`.
#[must_use]
pub fn commit_packet(p: &Packet) -> Vec<u8> {
    let th = p.timeout_height.unwrap_or_default();
    let mut buf = p.timeout_timestamp.to_be_bytes().to_vec();
    buf.extend_from_slice(&th.revision_number.to_be_bytes());
    buf.extend_from_slice(&th.revision_height.to_be_bytes());
    buf.extend_from_slice(&hash::sha256(&p.data));
    hash::sha256(&buf).to_vec()
}

/// `CommitAcknowledgement`.
#[must_use]
pub fn commit_acknowledgement(data: &[u8]) -> Vec<u8> {
    hash::sha256(data).to_vec()
}

/// `Packet.ValidateBasic`.
pub fn validate_packet(p: &Packet) -> Result<()> {
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
    if crate::height::is_zero(&p.timeout_height.unwrap_or_default()) && p.timeout_timestamp == 0 {
        return Err(cherr::invalid_packet(
            "packet timeout height and packet timeout timestamp cannot both be 0",
        ));
    }
    if p.data.is_empty() {
        return Err(cherr::invalid_packet("packet data bytes cannot be empty"));
    }
    Ok(())
}

/// `Channel.ValidateBasic` (`Counterparty.ValidateBasic` included).
pub fn validate_channel(ch: &Channel) -> Result<()> {
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

/// `Order.String()`.
#[must_use]
pub fn order_name(o: i32) -> &'static str {
    match Order::try_from(o) {
        Ok(Order::NoneUnspecified) => "ORDER_NONE_UNSPECIFIED",
        Ok(Order::Unordered) => "ORDER_UNORDERED",
        Ok(Order::Ordered) => "ORDER_ORDERED",
        Err(_) => "UNKNOWN",
    }
}

/// `State.String()`.
#[must_use]
pub fn state_name(s: i32) -> &'static str {
    match State::try_from(s) {
        Ok(State::UninitializedUnspecified) => "STATE_UNINITIALIZED_UNSPECIFIED",
        Ok(State::Init) => "STATE_INIT",
        Ok(State::Tryopen) => "STATE_TRYOPEN",
        Ok(State::Open) => "STATE_OPEN",
        Ok(State::Closed) => "STATE_CLOSED",
        Ok(State::Flushing) => "STATE_FLUSHING",
        Ok(State::Flushcomplete) => "STATE_FLUSHCOMPLETE",
        Err(_) => "UNKNOWN",
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub(crate) type Event = celestia_proto::celestia_core_proto::abci::Event;

pub(crate) fn with_message(e: Event) -> [Event; 2] {
    [e, sdk::event::event("message", [("module", "ibc_channel")])]
}

fn hex_lower(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

pub(crate) fn hop(ch: &Channel) -> String {
    ch.connection_hops.first().cloned().unwrap_or_default()
}

fn channel_event(
    ty: &str,
    port: &str,
    chan: &str,
    ch: &Channel,
    cp_chan: bool,
    version: bool,
) -> [Event; 2] {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let mut attrs = vec![
        ("port_id", port.to_owned()),
        ("channel_id", chan.to_owned()),
        ("counterparty_port_id", cp.port_id),
    ];
    if cp_chan {
        attrs.push(("counterparty_channel_id", cp.channel_id));
    }
    attrs.push(("connection_id", hop(ch)));
    if version {
        attrs.push(("version", ch.version.clone()));
    }
    with_message(sdk::event::event(ty, attrs))
}

fn packet_attrs(p: &Packet, data: bool) -> Vec<(&'static str, String)> {
    let mut attrs = Vec::new();
    if data {
        attrs.push(("packet_data", String::from_utf8_lossy(&p.data).into_owned()));
        attrs.push(("packet_data_hex", hex_lower(&p.data)));
    }
    attrs.extend([
        (
            "packet_timeout_height",
            crate::height::to_string(&p.timeout_height.unwrap_or_default()),
        ),
        ("packet_timeout_timestamp", p.timeout_timestamp.to_string()),
        ("packet_sequence", p.sequence.to_string()),
        ("packet_src_port", p.source_port.clone()),
        ("packet_src_channel", p.source_channel.clone()),
        ("packet_dst_port", p.destination_port.clone()),
        ("packet_dst_channel", p.destination_channel.clone()),
    ]);
    attrs
}

fn emit_packet_event(
    ctx: &mut Context<'_>,
    ty: &str,
    p: &Packet,
    ch: &Channel,
    ack: Option<&[u8]>,
) {
    let data = matches!(ty, "send_packet" | "recv_packet" | "write_acknowledgement");
    let mut attrs = packet_attrs(p, data);
    if let Some(a) = ack {
        attrs.push(("packet_ack", String::from_utf8_lossy(a).into_owned()));
        attrs.push(("packet_ack_hex", hex_lower(a)));
    }
    if ty == "timeout_packet" {
        attrs.push(("connection_id", hop(ch)));
        attrs.push((
            "packet_channel_ordering",
            order_name(ch.ordering).to_owned(),
        ));
    } else {
        attrs.push((
            "packet_channel_ordering",
            order_name(ch.ordering).to_owned(),
        ));
        attrs.push(("packet_connection", hop(ch)));
        attrs.push(("connection_id", hop(ch)));
    }
    ctx.events
        .emit_all(with_message(sdk::event::event(ty, attrs)));
}

// ---------------------------------------------------------------------------
// Handshake (`keeper/handshake.go`)
// ---------------------------------------------------------------------------

pub(crate) fn get_connection(ctx: &mut Context<'_>, id: &str) -> Result<ConnectionEnd> {
    crate::connection::get_connection(ctx, id)?.ok_or_else(|| conerr::connection_not_found(id))
}

pub(crate) fn connection_open(c: &ConnectionEnd) -> Result<()> {
    if c.state != ConnState::Open as i32 {
        return Err(conerr::invalid_connection_state(format!(
            "connection state is not OPEN (got {})",
            conn_state_name(c.state)
        )));
    }
    Ok(())
}

pub(crate) fn conn_state_name(s: i32) -> &'static str {
    match ConnState::try_from(s) {
        Ok(ConnState::UninitializedUnspecified) => "STATE_UNINITIALIZED_UNSPECIFIED",
        Ok(ConnState::Init) => "STATE_INIT",
        Ok(ConnState::Tryopen) => "STATE_TRYOPEN",
        Ok(ConnState::Open) => "STATE_OPEN",
        Err(_) => "UNKNOWN",
    }
}

fn single_version_supports(c: &ConnectionEnd, order: i32) -> Result<()> {
    if c.versions.len() != 1 {
        return Err(conerr::invalid_version(
            "single version must be negotiated on connection before opening channel",
        ));
    }
    if !crate::connection::verify_supported_feature(&c.versions[0], order_name(order)) {
        return Err(conerr::invalid_version(format!(
            "connection version does not support channel ordering: {}",
            order_name(order)
        )));
    }
    Ok(())
}

fn client_active(ctx: &mut Context<'_>, client_id: &str) -> Result<()> {
    let cs = crate::client::client_state(ctx, client_id)?
        .ok_or_else(|| clerr::client_not_found(format!("clientID ({client_id})")))?;
    let status = crate::client::client_status(ctx, client_id, &cs)?;
    if status != crate::client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "client ({client_id}) status is {}",
            status.as_str()
        )));
    }
    Ok(())
}

/// `ChanOpenInit`: returns the channel ID and its new capability.
#[allow(clippy::too_many_arguments)]
pub fn chan_open_init(
    ctx: &mut Context<'_>,
    order: i32,
    hops: &[String],
    port: &str,
    port_cap: &Capability,
) -> Result<(String, Capability)> {
    let conn = get_connection(ctx, &hops[0])?;
    single_version_supports(&conn, order)?;
    client_active(ctx, &conn.client_id)?;
    host::port_identifier_validator(port).map_err(|e| sdk::error::go_panic(e.log))?;
    if !SCOPED.authenticate_capability(ctx, port_cap, &host::port_path(port))? {
        return Err(porterr::invalid_port(format!(
            "caller does not own port capability for port ID {port}"
        )));
    }
    let chan = generate_channel_identifier(ctx)?;
    let cap = SCOPED
        .new_capability(ctx, &host::channel_capability_path(port, &chan))
        .map_err(|e| {
            e.wrap(format!(
                "could not create channel capability for port ID {port} and channel ID {chan}"
            ))
        })?;
    Ok((chan, cap))
}

/// `WriteOpenInitChannel`.
pub fn write_open_init_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    order: i32,
    hops: &[String],
    cp: &Counterparty,
    version: &str,
) -> Result<()> {
    let ch = Channel {
        state: State::Init as i32,
        ordering: order,
        counterparty: Some(cp.clone()),
        connection_hops: hops.to_vec(),
        version: version.to_owned(),
        upgrade_sequence: 0,
    };
    set_channel(ctx, port, chan, &ch)?;
    set_next_sequence_send(ctx, port, chan, 1)?;
    set_next_sequence_recv(ctx, port, chan, 1)?;
    set_next_sequence_ack(ctx, port, chan, 1)?;
    ctx.events.emit_all(channel_event(
        "channel_open_init",
        port,
        chan,
        &ch,
        false,
        true,
    ));
    Ok(())
}

/// `ChanOpenTry`.
#[allow(clippy::too_many_arguments)]
pub fn chan_open_try(
    ctx: &mut Context<'_>,
    order: i32,
    hops: &[String],
    port: &str,
    port_cap: &Capability,
    cp: &Counterparty,
    cp_version: &str,
    init_proof: &[u8],
    proof_height: &Height,
) -> Result<(String, Capability)> {
    if hops.len() != 1 {
        return Err(cherr::too_many_connection_hops(format!(
            "expected 1, got {}",
            hops.len()
        )));
    }
    let chan = generate_channel_identifier(ctx)?;
    host::port_identifier_validator(port).map_err(|e| sdk::error::go_panic(e.log))?;
    if !SCOPED.authenticate_capability(ctx, port_cap, &host::port_path(port))? {
        return Err(porterr::invalid_port(format!(
            "caller does not own port capability for port ID {port}"
        )));
    }
    let conn = get_connection(ctx, &hops[0])?;
    connection_open(&conn)?;
    single_version_supports(&conn, order)?;
    let expected = Channel {
        state: State::Init as i32,
        ordering: order,
        counterparty: Some(Counterparty {
            port_id: port.to_owned(),
            channel_id: String::new(),
        }),
        connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
        version: cp_version.to_owned(),
        upgrade_sequence: 0,
    };
    verify_channel_state(
        ctx,
        &conn,
        proof_height,
        init_proof,
        &cp.port_id,
        &cp.channel_id,
        &expected,
    )?;
    let cap = SCOPED
        .new_capability(ctx, &host::channel_capability_path(port, &chan))
        .map_err(|e| {
            e.wrap(format!(
                "could not create channel capability for port ID {port} and channel ID {chan}"
            ))
        })?;
    Ok((chan, cap))
}

/// `WriteOpenTryChannel`.
pub fn write_open_try_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    order: i32,
    hops: &[String],
    cp: &Counterparty,
    version: &str,
) -> Result<()> {
    set_next_sequence_send(ctx, port, chan, 1)?;
    set_next_sequence_recv(ctx, port, chan, 1)?;
    set_next_sequence_ack(ctx, port, chan, 1)?;
    let ch = Channel {
        state: State::Tryopen as i32,
        ordering: order,
        counterparty: Some(cp.clone()),
        connection_hops: hops.to_vec(),
        version: version.to_owned(),
        upgrade_sequence: 0,
    };
    set_channel(ctx, port, chan, &ch)?;
    ctx.events.emit_all(channel_event(
        "channel_open_try",
        port,
        chan,
        &ch,
        true,
        true,
    ));
    Ok(())
}

fn channel_or_err(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<Channel> {
    get_channel(ctx, port, chan)?
        .ok_or_else(|| cherr::channel_not_found(format!("port ID ({port}) channel ID ({chan})")))
}

/// `ChanOpenAck`.
#[allow(clippy::too_many_arguments)]
pub fn chan_open_ack(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cap: &Capability,
    cp_version: &str,
    cp_channel: &str,
    try_proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let ch = channel_or_err(ctx, port, chan)?;
    if ch.state != State::Init as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state should be INIT (got {})",
            state_name(ch.state)
        )));
    }
    if !authenticate_channel(ctx, cap, port, chan)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: State::Tryopen as i32,
        ordering: ch.ordering,
        counterparty: Some(Counterparty {
            port_id: port.to_owned(),
            channel_id: chan.to_owned(),
        }),
        connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
        version: cp_version.to_owned(),
        upgrade_sequence: 0,
    };
    let cp = ch.counterparty.clone().unwrap_or_default();
    verify_channel_state(
        ctx,
        &conn,
        proof_height,
        try_proof,
        &cp.port_id,
        cp_channel,
        &expected,
    )
}

/// `WriteOpenAckChannel`.
pub fn write_open_ack_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_version: &str,
    cp_channel: &str,
) -> Result<()> {
    let mut ch = get_channel(ctx, port, chan)?
        .ok_or_else(|| sdk::error::go_panic("could not find existing channel"))?;
    ch.state = State::Open as i32;
    cp_version.clone_into(&mut ch.version);
    let mut cp = ch.counterparty.clone().unwrap_or_default();
    cp_channel.clone_into(&mut cp.channel_id);
    ch.counterparty = Some(cp);
    set_channel(ctx, port, chan, &ch)?;
    ctx.events.emit_all(channel_event(
        "channel_open_ack",
        port,
        chan,
        &ch,
        true,
        false,
    ));
    Ok(())
}

/// `ChanOpenConfirm`.
pub fn chan_open_confirm(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cap: &Capability,
    ack_proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let ch = channel_or_err(ctx, port, chan)?;
    if ch.state != State::Tryopen as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel state is not TRYOPEN (got {})",
            state_name(ch.state)
        )));
    }
    if !authenticate_channel(ctx, cap, port, chan)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: State::Open as i32,
        ordering: ch.ordering,
        counterparty: Some(Counterparty {
            port_id: port.to_owned(),
            channel_id: chan.to_owned(),
        }),
        connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
        version: ch.version.clone(),
        upgrade_sequence: 0,
    };
    let cp = ch.counterparty.clone().unwrap_or_default();
    verify_channel_state(
        ctx,
        &conn,
        proof_height,
        ack_proof,
        &cp.port_id,
        &cp.channel_id,
        &expected,
    )
}

/// `WriteOpenConfirmChannel`.
pub fn write_open_confirm_channel(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<()> {
    let mut ch = get_channel(ctx, port, chan)?
        .ok_or_else(|| sdk::error::go_panic("could not find existing channel"))?;
    ch.state = State::Open as i32;
    set_channel(ctx, port, chan, &ch)?;
    ctx.events.emit_all(channel_event(
        "channel_open_confirm",
        port,
        chan,
        &ch,
        true,
        false,
    ));
    Ok(())
}

/// `ChanCloseInit`.
pub fn chan_close_init(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cap: &Capability,
) -> Result<()> {
    if !authenticate_channel(ctx, cap, port, chan)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    let mut ch = channel_or_err(ctx, port, chan)?;
    if ch.state == State::Closed as i32 {
        return Err(cherr::invalid_channel_state("channel is already CLOSED"));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    client_active(ctx, &conn.client_id)?;
    connection_open(&conn)?;
    ch.state = State::Closed as i32;
    set_channel(ctx, port, chan, &ch)?;
    ctx.events.emit_all(channel_event(
        "channel_close_init",
        port,
        chan,
        &ch,
        true,
        false,
    ));
    Ok(())
}

/// `ChanCloseConfirmWithCounterpartyUpgradeSequence`.
#[allow(clippy::too_many_arguments)]
pub fn chan_close_confirm(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cap: &Capability,
    init_proof: &[u8],
    proof_height: &Height,
    cp_upgrade_sequence: u64,
) -> Result<()> {
    if !authenticate_channel(ctx, cap, port, chan)? {
        return Err(cherr::channel_capability_not_found(
            "caller does not own capability for channel, port ID (%s) channel ID (%s)",
        ));
    }
    let mut ch = channel_or_err(ctx, port, chan)?;
    if ch.state == State::Closed as i32 {
        return Err(cherr::invalid_channel_state("channel is already CLOSED"));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: State::Closed as i32,
        ordering: ch.ordering,
        counterparty: Some(Counterparty {
            port_id: port.to_owned(),
            channel_id: chan.to_owned(),
        }),
        connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
        version: ch.version.clone(),
        upgrade_sequence: cp_upgrade_sequence,
    };
    let cp = ch.counterparty.clone().unwrap_or_default();
    verify_channel_state(
        ctx,
        &conn,
        proof_height,
        init_proof,
        &cp.port_id,
        &cp.channel_id,
        &expected,
    )?;
    ch.state = State::Closed as i32;
    set_channel(ctx, port, chan, &ch)?;
    ctx.events.emit_all(channel_event(
        "channel_close_confirm",
        port,
        chan,
        &ch,
        true,
        false,
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Verification helpers (`03-connection/keeper/verify.go`)
// ---------------------------------------------------------------------------

use crate::connection::{Kind, verify};

fn path(key: &[u8]) -> Vec<String> {
    vec![String::from_utf8_lossy(key).into_owned()]
}

/// `VerifyChannelState`.
#[allow(clippy::too_many_arguments)]
pub fn verify_channel_state(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    h: &Height,
    proof: &[u8],
    port: &str,
    chan: &str,
    expected: &Channel,
) -> Result<()> {
    let mut e = expected.clone();
    if e.counterparty.is_none() {
        e.counterparty = Some(Counterparty::default());
    }
    let bz = e.encode_to_vec();
    verify(
        ctx,
        conn,
        h,
        proof,
        &path(&host::channel_key(port, chan)),
        Some(&bz),
        Kind::State,
        "failed channel state verification for client",
    )
}

// ---------------------------------------------------------------------------
// Packets (`keeper/packet.go`, `keeper/timeout.go`)
// ---------------------------------------------------------------------------

/// `SendPacket`: returns the sequence.
#[allow(clippy::too_many_arguments)]
pub fn send_packet(
    ctx: &mut Context<'_>,
    cap: &Capability,
    port: &str,
    chan: &str,
    timeout_height: &Height,
    timeout_timestamp: u64,
    data: &[u8],
) -> Result<u64> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| cherr::channel_not_found(chan))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "channel is not OPEN (got {})",
            state_name(ch.state)
        )));
    }
    if !authenticate_channel(ctx, cap, port, chan)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel, port ID ({port}) channel ID ({chan})"
        )));
    }
    let seq = get_next_sequence_send(ctx, port, chan)?.ok_or_else(|| {
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
    let conn = get_connection(ctx, &hop(&ch))?;
    let cs = crate::client::client_state(ctx, &conn.client_id)?
        .ok_or_else(|| clerr::client_not_found(""))?;
    let status = crate::client::client_status(ctx, &conn.client_id, &cs)?;
    if status != crate::client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "cannot send packet using client ({}) with status {}",
            conn.client_id,
            status.as_str()
        )));
    }
    let latest = cs.latest_height();
    let latest_ts = crate::connection::get_timestamp_at_height(ctx, &conn, &latest)?;
    if crate::height::timeout_elapsed(timeout_height, timeout_timestamp, &latest, latest_ts) {
        return Err(cherr::timeout_elapsed("invalid packet timeout"));
    }
    let commitment = commit_packet(&packet);
    set_next_sequence_send(ctx, port, chan, seq.wrapping_add(1))?;
    set_packet_commitment(ctx, port, chan, seq, &commitment)?;
    emit_packet_event(ctx, "send_packet", &packet, &ch, None);
    Ok(seq)
}

fn recv_states_ok(s: i32) -> bool {
    s == State::Open as i32 || s == State::Flushing as i32 || s == State::Flushcomplete as i32
}

/// Outcome of a packet-verification step that may be a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Verified; state written.
    Done,
    /// Redundant relay (`ErrNoOpMsg`).
    NoOp,
}

/// `RecvPacket` (runs in the caller's cache branch).
pub fn recv_packet(
    ctx: &mut Context<'_>,
    cap: &Capability,
    p: &Packet,
    proof: &[u8],
    proof_height: &Height,
) -> Result<Outcome> {
    let ch = get_channel(ctx, &p.destination_port, &p.destination_channel)?
        .ok_or_else(|| cherr::channel_not_found(&p.destination_channel))?;
    if !recv_states_ok(ch.state) {
        return Err(cherr::invalid_channel_state(format!(
            "expected channel state to be one of [STATE_OPEN, STATE_FLUSHING, STATE_FLUSHCOMPLETE], but got {}",
            state_name(ch.state)
        )));
    }
    if let Some(up) = get_counterparty_upgrade(ctx, &p.destination_port, &p.destination_channel)?
        && p.sequence >= up.next_sequence_send
    {
        return Err(cherr::invalid_packet(
            "cannot flush packet at sequence greater than or equal to counterparty next sequence send",
        ));
    }
    let cap_name = host::channel_capability_path(&p.destination_port, &p.destination_channel);
    if !SCOPED.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        )));
    }
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.source_port != cp.port_id {
        return Err(cherr::invalid_packet(
            "packet source port doesn't match the counterparty's port",
        ));
    }
    if p.source_channel != cp.channel_id {
        return Err(cherr::invalid_packet(
            "packet source channel doesn't match the counterparty's channel",
        ));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    connection_open(&conn)?;
    let self_height = crate::client::get_self_height(ctx)?;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let self_ts = crate::lightclients::tendermint::block_time(ctx) as u64;
    let th = p.timeout_height.unwrap_or_default();
    if crate::height::timeout_elapsed(&th, p.timeout_timestamp, &self_height, self_ts) {
        return Err(cherr::timeout_elapsed("packet timeout elapsed"));
    }
    let commitment = commit_packet(p);
    verify(
        ctx,
        &conn,
        proof_height,
        proof,
        &path(&host::packet_commitment_key(
            &p.source_port,
            &p.source_channel,
            p.sequence,
        )),
        Some(&commitment),
        Kind::Packet,
        "failed packet commitment verification for client",
    )
    .map_err(|e| e.wrap("couldn't verify counterparty packet commitment"))?;
    apply_replay_protection(ctx, p, &ch)
}

/// `applyReplayProtection` (emits `recv_packet` also for a no-op).
fn apply_replay_protection(ctx: &mut Context<'_>, p: &Packet, ch: &Channel) -> Result<Outcome> {
    let start = get_recv_start_sequence(ctx, &p.destination_port, &p.destination_channel)?
        .unwrap_or_default();
    if p.sequence < start {
        return Err(cherr::packet_received(
            "packet already processed in previous channel upgrade",
        ));
    }
    if ch.ordering == Order::Unordered as i32 {
        if has_packet_receipt(ctx, &p.destination_port, &p.destination_channel, p.sequence)? {
            emit_packet_event(ctx, "recv_packet", p, ch, None);
            return Ok(Outcome::NoOp);
        }
        set_packet_receipt(ctx, &p.destination_port, &p.destination_channel, p.sequence)?;
    } else if ch.ordering == Order::Ordered as i32 {
        let next = get_next_sequence_recv(ctx, &p.destination_port, &p.destination_channel)?
            .ok_or_else(|| {
                cherr::sequence_receive_not_found(format!(
                    "destination port: {}, destination channel: {}",
                    p.destination_port, p.destination_channel
                ))
            })?;
        if p.sequence < next {
            emit_packet_event(ctx, "recv_packet", p, ch, None);
            return Ok(Outcome::NoOp);
        }
        if p.sequence != next {
            return Err(cherr::packet_sequence_out_of_order(format!(
                "packet sequence ≠ next receive sequence ({} ≠ {next})",
                p.sequence
            )));
        }
        set_next_sequence_recv(
            ctx,
            &p.destination_port,
            &p.destination_channel,
            next.wrapping_add(1),
        )?;
    }
    emit_packet_event(ctx, "recv_packet", p, ch, None);
    Ok(Outcome::Done)
}

/// `RecvPacketReCheckTx`.
pub fn recv_packet_recheck(ctx: &mut Context<'_>, p: &Packet) -> Result<Outcome> {
    let ch = get_channel(ctx, &p.destination_port, &p.destination_channel)?
        .ok_or_else(|| cherr::channel_not_found(&p.destination_channel))?;
    apply_replay_protection(ctx, p, &ch)
}

/// `WriteAcknowledgement`.
pub fn write_acknowledgement(
    ctx: &mut Context<'_>,
    cap: &Capability,
    p: &Packet,
    ack: &[u8],
) -> Result<()> {
    let ch = get_channel(ctx, &p.destination_port, &p.destination_channel)?
        .ok_or_else(|| cherr::channel_not_found(&p.destination_channel))?;
    if !recv_states_ok(ch.state) {
        return Err(cherr::invalid_channel_state(format!(
            "expected one of [STATE_OPEN, STATE_FLUSHING, STATE_FLUSHCOMPLETE], got {}",
            state_name(ch.state)
        )));
    }
    let cap_name = host::channel_capability_path(&p.destination_port, &p.destination_channel);
    if !SCOPED.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        )));
    }
    if has_packet_acknowledgement(ctx, &p.destination_port, &p.destination_channel, p.sequence)? {
        return Err(cherr::acknowledgement_exists(""));
    }
    if ack.is_empty() {
        return Err(cherr::invalid_acknowledgement(
            "acknowledgement cannot be empty",
        ));
    }
    set_packet_acknowledgement(
        ctx,
        &p.destination_port,
        &p.destination_channel,
        p.sequence,
        &commit_acknowledgement(ack),
    )?;
    emit_packet_event(ctx, "write_acknowledgement", p, &ch, Some(ack));
    Ok(())
}

/// A decoded `channeltypes.Acknowledgement` (`jsonpb`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckResponse {
    /// No response set (`{}`).
    Empty,
    /// `result` bytes.
    Result(Vec<u8>),
    /// `error` string.
    Error(String),
}

impl AckResponse {
    /// `Acknowledgement.Success()`.
    #[must_use]
    pub fn success(&self) -> bool {
        matches!(self, Self::Result(_))
    }

    /// `ModuleCdc.MarshalJSON(&ack)` (also `MustSortJSON` of it: one key).
    #[must_use]
    pub fn marshal(&self) -> String {
        match self {
            Self::Empty => "{}".into(),
            Self::Result(r) => format!(
                "{{\"result\":{}}}",
                crate::gojson::escape_string(&crate::router::base64_std(r))
            ),
            Self::Error(e) => format!("{{\"error\":{}}}", crate::gojson::escape_string(e)),
        }
    }
}

/// `SubModuleCdc.UnmarshalJSON(bz, &ack)`.
pub fn decode_acknowledgement(bz: &[u8]) -> core::result::Result<AckResponse, String> {
    let v = crate::gojson::first_value(bz)?;
    let map = match v {
        serde_json::Value::Null => return Ok(AckResponse::Empty),
        serde_json::Value::Object(m) => m,
        _ => return Err("cannot unmarshal into Acknowledgement".into()),
    };
    let mut out = AckResponse::Empty;
    for (k, v) in map {
        match (k.as_str(), v) {
            ("result", serde_json::Value::String(s)) => {
                out = AckResponse::Result(
                    crate::gojson::decode_base64_std(&s).ok_or("illegal base64 data")?,
                );
            }
            ("error", serde_json::Value::String(s)) => out = AckResponse::Error(s),
            ("result" | "error", serde_json::Value::Null) => {}
            ("result" | "error", _) => return Err(format!("invalid value for field {k}")),
            _ => return Err(format!("unknown field {k:?} in types.Acknowledgement")),
        }
    }
    Ok(out)
}

/// `AcknowledgePacket`'s marshalling check: a decodable JSON
/// acknowledgement must re-marshal (sorted) to the same bytes.
fn ack_marshalling_mismatch(ack: &[u8]) -> bool {
    match decode_acknowledgement(ack) {
        Ok(a) => *gostr::to_go_bytes(&a.marshal()) != *ack,
        Err(_) => false,
    }
}
/// `AcknowledgePacket`.
pub fn acknowledge_packet(
    ctx: &mut Context<'_>,
    cap: &Capability,
    p: &Packet,
    ack: &[u8],
    proof: &[u8],
    proof_height: &Height,
) -> Result<Outcome> {
    let ch = get_channel(ctx, &p.source_port, &p.source_channel)?.ok_or_else(|| {
        cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            p.source_port, p.source_channel
        ))
    })?;
    if ch.state != State::Open as i32 && ch.state != State::Flushing as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "packets cannot be acknowledged on channel with state ({})",
            state_name(ch.state)
        )));
    }
    let cap_name = host::channel_capability_path(&p.source_port, &p.source_channel);
    if !SCOPED.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication for capability name {cap_name}"
        )));
    }
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.destination_port != cp.port_id {
        return Err(cherr::invalid_packet(
            "packet destination port doesn't match the counterparty's port",
        ));
    }
    if p.destination_channel != cp.channel_id {
        return Err(cherr::invalid_packet(
            "packet destination channel doesn't match the counterparty's channel",
        ));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    connection_open(&conn)?;
    let commitment = get_packet_commitment(ctx, &p.source_port, &p.source_channel, p.sequence)?;
    if commitment.is_empty() {
        emit_packet_event(ctx, "acknowledge_packet", p, &ch, None);
        return Ok(Outcome::NoOp);
    }
    let packet_commitment = commit_packet(p);
    if ack_marshalling_mismatch(ack) {
        return Err(cherr::invalid_acknowledgement(
            "acknowledgement marshalling error",
        ));
    }
    if commitment != packet_commitment {
        return Err(cherr::invalid_packet("commitment bytes are not equal"));
    }
    verify(
        ctx,
        &conn,
        proof_height,
        proof,
        &path(&host::packet_acknowledgement_key(
            &p.destination_port,
            &p.destination_channel,
            p.sequence,
        )),
        Some(&commit_acknowledgement(ack)),
        Kind::Packet,
        "failed packet acknowledgement verification for client",
    )?;
    if ch.ordering == Order::Ordered as i32 {
        let next =
            get_next_sequence_ack(ctx, &p.source_port, &p.source_channel)?.ok_or_else(|| {
                cherr::sequence_ack_not_found(format!(
                    "source port: {}, source channel: {}",
                    p.source_port, p.source_channel
                ))
            })?;
        if p.sequence != next {
            return Err(cherr::packet_sequence_out_of_order(format!(
                "packet sequence ≠ next ack sequence ({} ≠ {next})",
                p.sequence
            )));
        }
        set_next_sequence_ack(ctx, &p.source_port, &p.source_channel, next.wrapping_add(1))?;
    }
    delete_packet_commitment(ctx, &p.source_port, &p.source_channel, p.sequence)?;
    emit_packet_event(ctx, "acknowledge_packet", p, &ch, None);
    crate::upgrade::flush_on_ack(ctx, &p.source_port, &p.source_channel, ch)?;
    Ok(Outcome::Done)
}

/// `TimeoutPacket`.
pub fn timeout_packet(
    ctx: &mut Context<'_>,
    p: &Packet,
    proof: &[u8],
    proof_height: &Height,
    next_seq_recv: u64,
) -> Result<Outcome> {
    let ch = get_channel(ctx, &p.source_port, &p.source_channel)?.ok_or_else(|| {
        cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            p.source_port, p.source_channel
        ))
    })?;
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.destination_port != cp.port_id {
        return Err(cherr::invalid_packet(
            "packet destination port doesn't match the counterparty's port",
        ));
    }
    if p.destination_channel != cp.channel_id {
        return Err(cherr::invalid_packet(
            "packet destination channel doesn't match the counterparty's channel",
        ));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    let proof_ts = crate::connection::get_timestamp_at_height(ctx, &conn, proof_height)?;
    let th = p.timeout_height.unwrap_or_default();
    if !crate::height::timeout_elapsed(&th, p.timeout_timestamp, proof_height, proof_ts) {
        return Err(cherr::timeout_not_reached("packet timeout not reached"));
    }
    let commitment = get_packet_commitment(ctx, &p.source_port, &p.source_channel, p.sequence)?;
    if commitment.is_empty() {
        emit_packet_event(ctx, "timeout_packet", p, &ch, None);
        return Ok(Outcome::NoOp);
    }
    if commitment != commit_packet(p) {
        return Err(cherr::invalid_packet(
            "packet commitment bytes are not equal",
        ));
    }
    verify_unreceived(
        ctx,
        &conn,
        &ch,
        p,
        proof,
        proof_height,
        next_seq_recv,
        false,
    )?;
    Ok(Outcome::Done)
}

#[allow(clippy::too_many_arguments)]
fn verify_unreceived(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    ch: &Channel,
    p: &Packet,
    proof: &[u8],
    proof_height: &Height,
    next_seq_recv: u64,
    on_close: bool,
) -> Result<()> {
    if ch.ordering == Order::Ordered as i32 {
        if next_seq_recv > p.sequence {
            let log = "packet already received, next sequence receive > packet sequence";
            return Err(if on_close {
                cherr::invalid_packet(log)
            } else {
                cherr::packet_received(log)
            });
        }
        verify(
            ctx,
            conn,
            proof_height,
            proof,
            &path(&host::next_sequence_recv_key(
                &p.destination_port,
                &p.destination_channel,
            )),
            Some(&next_seq_recv.to_be_bytes()),
            Kind::Packet,
            "failed next sequence receive verification for client",
        )
    } else if ch.ordering == Order::Unordered as i32 {
        verify(
            ctx,
            conn,
            proof_height,
            proof,
            &path(&host::packet_receipt_key(
                &p.destination_port,
                &p.destination_channel,
                p.sequence,
            )),
            None,
            Kind::Packet,
            "failed packet receipt absence verification for client",
        )
    } else {
        Err(sdk::error::go_panic(format!(
            "{}: invalid channel ordering",
            order_name(ch.ordering)
        )))
    }
}

/// `TimeoutExecuted`.
pub fn timeout_executed(ctx: &mut Context<'_>, cap: &Capability, p: &Packet) -> Result<()> {
    let mut ch = get_channel(ctx, &p.source_port, &p.source_channel)?.ok_or_else(|| {
        cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            p.source_port, p.source_channel
        ))
    })?;
    let cap_name = host::channel_capability_path(&p.source_port, &p.source_channel);
    if !SCOPED.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::channel_capability_not_found(format!(
            "caller does not own capability for channel with capability name {cap_name}"
        )));
    }
    delete_packet_commitment(ctx, &p.source_port, &p.source_channel, p.sequence)?;
    crate::upgrade::flush_on_timeout(ctx, &p.source_port, &p.source_channel, &mut ch)?;
    if ch.ordering == Order::Ordered as i32 {
        if ch.state == State::Flushing as i32 {
            crate::upgrade::abort_on_ordered_timeout(ctx, &p.source_port, &p.source_channel)?;
        }
        ch.state = State::Closed as i32;
        set_channel(ctx, &p.source_port, &p.source_channel, &ch)?;
        let cp = ch.counterparty.clone().unwrap_or_default();
        ctx.events.emit_all(with_message(sdk::event::event(
            "channel_close",
            [
                ("port_id", p.source_port.clone()),
                ("channel_id", p.source_channel.clone()),
                ("counterparty_port_id", cp.port_id),
                ("counterparty_channel_id", cp.channel_id),
                ("connection_id", hop(&ch)),
                (
                    "packet_channel_ordering",
                    order_name(ch.ordering).to_owned(),
                ),
            ],
        )));
    }
    emit_packet_event(ctx, "timeout_packet", p, &ch, None);
    Ok(())
}

/// `TimeoutOnCloseWithCounterpartyUpgradeSequence`.
#[allow(clippy::too_many_arguments)]
pub fn timeout_on_close(
    ctx: &mut Context<'_>,
    cap: &Capability,
    p: &Packet,
    proof: &[u8],
    close_proof: &[u8],
    proof_height: &Height,
    next_seq_recv: u64,
    cp_upgrade_sequence: u64,
) -> Result<Outcome> {
    let ch = get_channel(ctx, &p.source_port, &p.source_channel)?.ok_or_else(|| {
        cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            p.source_port, p.source_channel
        ))
    })?;
    let cap_name = host::channel_capability_path(&p.source_port, &p.source_channel);
    if !SCOPED.authenticate_capability(ctx, cap, &cap_name)? {
        return Err(cherr::invalid_channel_capability(format!(
            "channel capability failed authentication with capability name {cap_name}"
        )));
    }
    let cp = ch.counterparty.clone().unwrap_or_default();
    if p.destination_port != cp.port_id {
        return Err(cherr::invalid_packet(
            "packet destination port doesn't match the counterparty's port",
        ));
    }
    if p.destination_channel != cp.channel_id {
        return Err(cherr::invalid_packet(
            "packet destination channel doesn't match the counterparty's channel",
        ));
    }
    let conn = get_connection(ctx, &hop(&ch))?;
    let commitment = get_packet_commitment(ctx, &p.source_port, &p.source_channel, p.sequence)?;
    if commitment.is_empty() {
        emit_packet_event(ctx, "timeout_packet", p, &ch, None);
        return Ok(Outcome::NoOp);
    }
    if commitment != commit_packet(p) {
        return Err(cherr::invalid_packet(
            "packet commitment bytes are not equal",
        ));
    }
    let expected = Channel {
        state: State::Closed as i32,
        ordering: ch.ordering,
        counterparty: Some(Counterparty {
            port_id: p.source_port.clone(),
            channel_id: p.source_channel.clone(),
        }),
        connection_hops: vec![conn.counterparty.clone().unwrap_or_default().connection_id],
        version: ch.version.clone(),
        upgrade_sequence: cp_upgrade_sequence,
    };
    verify_channel_state(
        ctx,
        &conn,
        proof_height,
        close_proof,
        &cp.port_id,
        &cp.channel_id,
        &expected,
    )?;
    verify_unreceived(ctx, &conn, &ch, p, proof, proof_height, next_seq_recv, true)?;
    Ok(Outcome::Done)
}

/// `PruneAcknowledgements`: deletes up to `limit` acks and receipts from the
/// pruning start sequence (set by channel upgrades); returns
/// `(pruned, remaining)`.
pub fn prune_acknowledgements(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    limit: u64,
) -> Result<(u64, u64)> {
    let start_key = format!("pruningSequenceStart/ports/{port}/channels/{chan}");
    let Some(start) = get_u64(ctx, start_key.as_bytes())? else {
        return Err(cherr::pruning_sequence_start_not_found(format!(
            "port ID ({port}) channel ID ({chan})"
        )));
    };
    let Some(end) = get_recv_start_sequence(ctx, port, chan)? else {
        return Err(cherr::recv_start_sequence_not_found(format!(
            "port ID ({port}) channel ID ({chan})"
        )));
    };
    let stop = start.wrapping_add(limit);
    let mut seq = start;
    while seq < stop {
        if seq >= end {
            break;
        }
        ctx.delete(
            crate::STORE_KEY,
            &host::packet_acknowledgement_key(port, chan, seq),
        )?;
        ctx.delete(crate::STORE_KEY, &host::packet_receipt_key(port, chan, seq))?;
        seq += 1;
    }
    put_u64(ctx, start_key.into_bytes(), seq)?;
    Ok((seq - start, end.wrapping_sub(seq)))
}
