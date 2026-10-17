//! Channel upgradability (ibc-go v8.7.2 `04-channel/keeper/upgrade.go`,
//! the upgrade handlers of `core/keeper/msg_server.go` and the upgrade
//! events): the Init / Try / Ack / Confirm / Open handshake, flushing,
//! cancellation, timeouts and error receipts.

use celestia_proto::ibc::core::channel::v1::{
    self as chpb, Channel, Counterparty, ErrorReceipt, Order, State, Timeout, Upgrade,
    UpgradeFields,
};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::connection::v1::ConnectionEnd;
use prost::Message;
use sdk::{Context, Result, SdkError};

use crate::channel::{
    self, Event, conn_state_name, connection_open, get_channel, get_connection, hop, order_name,
    set_channel, state_name, with_message,
};
use crate::connection::{Kind, verify_with};
use crate::errors::{channel as cherr, commitment as commerr, connection as conerr};
use crate::host;

/// `types.UpgradeError`: an error that aborts the upgrade (an error
/// receipt is written) instead of failing the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeError {
    /// Upgrade sequence of the receipt.
    pub sequence: u64,
    /// The cause.
    pub err: SdkError,
}

impl UpgradeError {
    /// `GetErrorReceipt`: only the ABCI code of the cause is kept.
    #[must_use]
    pub fn receipt(&self) -> ErrorReceipt {
        ErrorReceipt {
            sequence: self.sequence,
            message: format!(
                "ABCI code: {}: restored channel to pre-upgrade state",
                self.err.code
            ),
        }
    }
}

/// A failed upgrade step: an `UpgradeError` or a plain error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// `IsUpgradeError(err)`.
    Upgrade(UpgradeError),
    /// Any other error.
    Err(SdkError),
}

impl From<SdkError> for Failure {
    fn from(e: SdkError) -> Self {
        Self::Err(e)
    }
}

fn upgrade_err(sequence: u64, err: SdkError) -> Failure {
    Failure::Upgrade(UpgradeError { sequence, err })
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Fills the gogoproto non-nullable embedded messages (`fields`,
/// `timeout`, `timeout.height`) so they are always encoded.
#[must_use]
pub fn normalize_upgrade(mut u: Upgrade) -> Upgrade {
    if u.fields.is_none() {
        u.fields = Some(UpgradeFields::default());
    }
    let mut t = u.timeout.take().unwrap_or_default();
    if t.height.is_none() {
        t.height = Some(Height::default());
    }
    u.timeout = Some(t);
    u
}

fn key(kind: &str, port: &str, chan: &str) -> Vec<u8> {
    channel::channel_upgrade_path(kind, port, chan)
}

/// `GetUpgrade`.
pub fn get_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<Option<Upgrade>> {
    ctx.get(crate::STORE_KEY, &key("upgrades", port, chan))?
        .map(|b| Upgrade::decode(b.as_slice()).map_err(crate::errors::must_unmarshal))
        .transpose()
}

/// `SetUpgrade`.
pub fn set_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str, u: &Upgrade) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        key("upgrades", port, chan),
        normalize_upgrade(u.clone()).encode_to_vec(),
    )
}

fn has_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<bool> {
    ctx.has(crate::STORE_KEY, &key("upgrades", port, chan))
}

/// `SetCounterpartyUpgrade`.
pub fn set_counterparty_upgrade(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    u: &Upgrade,
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        key("counterpartyUpgrade", port, chan),
        normalize_upgrade(u.clone()).encode_to_vec(),
    )
}

fn has_counterparty_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<bool> {
    ctx.has(crate::STORE_KEY, &key("counterpartyUpgrade", port, chan))
}

/// `deleteUpgradeInfo`.
fn delete_upgrade_info(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<()> {
    ctx.delete(crate::STORE_KEY, &key("upgrades", port, chan))?;
    ctx.delete(crate::STORE_KEY, &key("counterpartyUpgrade", port, chan))
}

/// `GetUpgradeErrorReceipt`.
pub fn get_error_receipt(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<ErrorReceipt>> {
    ctx.get(crate::STORE_KEY, &key("upgradeError", port, chan))?
        .map(|b| ErrorReceipt::decode(b.as_slice()).map_err(crate::errors::must_unmarshal))
        .transpose()
}

/// `HasInflightPackets`: any packet commitment under the channel (a gas
/// metered prefix iterator, only its validity is read).
pub fn has_inflight_packets(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<bool> {
    let prefix = format!("commitments/ports/{port}/channels/{chan}/sequences");
    let mut it = ctx.prefix_iter(crate::STORE_KEY, prefix.as_bytes())?;
    Ok(it.next(ctx)?.is_some())
}

fn set_recv_start_sequence(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> Result<()> {
    channel::put_u64(
        ctx,
        format!("recvStartSequence/ports/{port}/channels/{chan}").into_bytes(),
        seq,
    )
}

fn pruning_start_key(port: &str, chan: &str) -> Vec<u8> {
    format!("pruningSequenceStart/ports/{port}/channels/{chan}").into_bytes()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn not_found(port: &str, chan: &str) -> SdkError {
    cherr::channel_not_found(format!("port ID ({port}) channel ID ({chan})"))
}

fn fields(u: &Upgrade) -> UpgradeFields {
    u.fields.clone().unwrap_or_default()
}

fn first_hop(f: &UpgradeFields) -> String {
    f.connection_hops.first().cloned().unwrap_or_default()
}

fn index0_panic() -> SdkError {
    sdk::error::go_panic("runtime error: index out of range [0] with length 0")
}

fn open_connection(ctx: &mut Context<'_>, ch: &Channel) -> Result<ConnectionEnd> {
    let id = ch.connection_hops.first().ok_or_else(index0_panic)?;
    let conn = get_connection(ctx, id)?;
    connection_open(&conn)?;
    Ok(conn)
}

fn cp_conn_id(conn: &ConnectionEnd) -> String {
    conn.counterparty.clone().unwrap_or_default().connection_id
}

fn self_height_time(ctx: &mut Context<'_>) -> Result<(Height, u64)> {
    let h = crate::client::get_self_height(ctx)?;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let t = crate::lightclients::tendermint::block_time(ctx) as u64;
    Ok((h, t))
}

fn height_elapsed(t: &Timeout, h: &Height) -> bool {
    let th = t.height.unwrap_or_default();
    !(th.revision_number == 0 && th.revision_height == 0) && crate::height::gte(h, &th)
}

/// `Timeout.Elapsed`.
fn elapsed(t: &Timeout, h: &Height, ts: u64) -> bool {
    height_elapsed(t, h) || (t.timestamp != 0 && ts >= t.timestamp)
}

/// `Timeout.ErrTimeoutElapsed`.
pub(crate) fn err_timeout_elapsed(t: &Timeout, h: &Height, ts: u64) -> SdkError {
    if height_elapsed(t, h) {
        return cherr::timeout_elapsed(format!(
            "current height: {}, timeout height {}",
            host::height_string(h),
            host::height_string(&t.height.unwrap_or_default())
        ));
    }
    cherr::timeout_elapsed(format!(
        "current timestamp: {ts}, timeout timestamp {}",
        t.timestamp
    ))
}

/// `Timeout.ErrTimeoutNotReached`.
pub(crate) fn err_timeout_not_reached(t: &Timeout, h: &Height, ts: u64) -> SdkError {
    let th = t.height.unwrap_or_default();
    let zero = th.revision_number == 0 && th.revision_height == 0;
    if !zero && !height_elapsed(t, h) {
        return cherr::timeout_not_reached(format!(
            "current height: {}, timeout height {}",
            host::height_string(h),
            host::height_string(&th)
        ));
    }
    cherr::timeout_not_reached(format!(
        "current timestamp: {ts}, timeout timestamp {}",
        t.timestamp
    ))
}

fn counterparty(port: &str, chan: &str) -> Option<Counterparty> {
    Some(Counterparty {
        port_id: port.to_owned(),
        channel_id: chan.to_owned(),
    })
}

/// `VerifyChannelState` against the channel's own counterparty.
fn verify_cp_channel(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    ch: &Channel,
    h: &Height,
    proof: &[u8],
    expected: &Channel,
) -> Result<()> {
    let cp = ch.counterparty.clone().unwrap_or_default();
    channel::verify_channel_state(ctx, conn, h, proof, &cp.port_id, &cp.channel_id, expected)
}

/// `VerifyChannelUpgrade`.
fn verify_cp_upgrade(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    ch: &Channel,
    h: &Height,
    proof: &[u8],
    upgrade: &Upgrade,
) -> Result<()> {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let bz = normalize_upgrade(upgrade.clone()).encode_to_vec();
    let path = String::from_utf8_lossy(&key("upgrades", &cp.port_id, &cp.channel_id)).into_owned();
    verify_with(
        ctx,
        conn,
        h,
        proof,
        &[path],
        Some(&bz),
        Kind::State,
        |e, client| {
            e.wrap(format!(
                "failed upgrade verification for client ({client}) on channel ({})",
                cp.channel_id
            ))
        },
    )
}

/// `VerifyChannelUpgradeError`.
fn verify_cp_error_receipt(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    ch: &Channel,
    h: &Height,
    proof: &[u8],
    receipt: &ErrorReceipt,
) -> Result<()> {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let bz = receipt.encode_to_vec();
    let path =
        String::from_utf8_lossy(&key("upgradeError", &cp.port_id, &cp.channel_id)).into_owned();
    verify_with(
        ctx,
        conn,
        h,
        proof,
        &[path],
        Some(&bz),
        Kind::State,
        |e, client| {
            e.wrap(format!(
                "failed upgrade error receipt verification for client ({client})"
            ))
        },
    )
}

/// `connectiontypes.Version` in gogoproto compact text form.
fn version_text(v: &celestia_proto::ibc::core::connection::v1::Version) -> String {
    let mut parts = vec![format!("identifier:{:?}", v.identifier)];
    parts.extend(v.features.iter().map(|f| format!("features:{f:?}")));
    parts.join(" ")
}

/// `UpgradeFields` in gogoproto compact text form.
fn fields_text(f: &UpgradeFields) -> String {
    let mut parts = Vec::new();
    if f.ordering != 0 {
        parts.push(format!("ordering:{}", order_name(f.ordering)));
    }
    parts.extend(
        f.connection_hops
            .iter()
            .map(|h| format!("connection_hops:{h:?}")),
    );
    if !f.version.is_empty() {
        parts.push(format!("version:{:?}", f.version));
    }
    parts.join(" ")
}

/// `validateSelfUpgradeFields`.
fn validate_self_upgrade_fields(
    ctx: &mut Context<'_>,
    proposed: &UpgradeFields,
    ch: &Channel,
) -> Result<()> {
    if proposed.ordering == ch.ordering
        && proposed.connection_hops == ch.connection_hops
        && proposed.version == ch.version
    {
        return Err(cherr::invalid_upgrade(format!(
            "existing channel end is identical to proposed upgrade channel end: got {}",
            fields_text(proposed)
        )));
    }
    let id = proposed.connection_hops.first().ok_or_else(index0_panic)?;
    let Some(conn) = crate::connection::get_connection(ctx, id)? else {
        return Err(conerr::connection_not_found(format!(
            "failed to retrieve connection: {id}"
        )));
    };
    connection_open(&conn)?;
    if conn.versions.len() != 1 {
        return Err(conerr::invalid_version(format!(
            "single version must be negotiated on connection before opening channel, got: [{}]",
            conn.versions
                .iter()
                .map(version_text)
                .collect::<Vec<_>>()
                .join(" ")
        )));
    }
    if !crate::connection::verify_supported_feature(
        &conn.versions[0],
        &order_name(proposed.ordering),
    ) {
        return Err(conerr::invalid_version(format!(
            "connection version {} does not support channel ordering: {}",
            version_text(&conn.versions[0]),
            order_name(proposed.ordering)
        )));
    }
    Ok(())
}

/// `checkForUpgradeCompatibility`.
fn check_compatibility(
    ctx: &mut Context<'_>,
    own: &UpgradeFields,
    cp: &UpgradeFields,
) -> Result<()> {
    if own.ordering != cp.ordering {
        return Err(cherr::incompatible_counterparty_upgrade(format!(
            "expected upgrade ordering ({}) to match counterparty upgrade ordering ({})",
            order_name(own.ordering),
            order_name(cp.ordering)
        )));
    }
    if own.version != cp.version {
        return Err(cherr::incompatible_counterparty_upgrade(format!(
            "expected upgrade version ({}) to match counterparty upgrade version ({})",
            own.version, cp.version
        )));
    }
    let id = own.connection_hops.first().ok_or_else(index0_panic)?;
    let conn = get_connection(ctx, id)?;
    if conn.state != celestia_proto::ibc::core::connection::v1::State::Open as i32 {
        return Err(conerr::invalid_connection_state(format!(
            "expected proposed connection to be OPEN (got {})",
            conn_state_name(conn.state)
        )));
    }
    let cp_hop = cp.connection_hops.first().ok_or_else(index0_panic)?;
    if *cp_hop != cp_conn_id(&conn) {
        return Err(cherr::incompatible_counterparty_upgrade(format!(
            "counterparty upgrade connection end is not a counterparty of self proposed connection end ({cp_hop} != {})",
            cp_conn_id(&conn)
        )));
    }
    Ok(())
}

/// `startFlushing`.
fn start_flushing(ctx: &mut Context<'_>, port: &str, chan: &str, up: &mut Upgrade) -> Result<()> {
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    open_connection(ctx, &ch)?;
    ch.state = State::Flushing as i32;
    set_channel(ctx, port, chan, &ch)?;
    let next = channel::get_next_sequence_send(ctx, port, chan)?.ok_or_else(|| {
        cherr::sequence_send_not_found(format!("port ID ({port}) channel ID ({chan})"))
    })?;
    up.next_sequence_send = next;
    let timeout = channel::get_params(ctx)?
        .upgrade_timeout
        .unwrap_or_default();
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let now = crate::lightclients::tendermint::block_time(ctx) as u64;
    up.timeout = Some(Timeout {
        height: Some(Height::default()),
        timestamp: now.wrapping_add(timeout.timestamp),
    });
    set_upgrade(ctx, port, chan, up)
}

/// `restoreChannel`.
fn restore_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
    mut ch: Channel,
) -> Result<Channel> {
    ch.state = State::Open as i32;
    ch.upgrade_sequence = seq;
    set_channel(ctx, port, chan, &ch)?;
    delete_upgrade_info(ctx, port, chan)?;
    Ok(ch)
}

/// `WriteErrorReceipt` (Go panics on the invariant violations).
pub fn write_error_receipt(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    e: &UpgradeError,
) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?
        .ok_or_else(|| sdk::error::go_panic(not_found(port, chan).log))?;
    let receipt = e.receipt();
    if let Some(existing) = get_error_receipt(ctx, port, chan)?
        && existing.sequence >= receipt.sequence
    {
        return Err(sdk::error::go_panic(
            cherr::invalid_upgrade_sequence(format!(
                "error receipt sequence ({}) must be greater than existing error receipt sequence ({})",
                receipt.sequence, existing.sequence
            ))
            .log,
        ));
    }
    if ch.upgrade_sequence <= receipt.sequence
        && (has_upgrade(ctx, port, chan)? || has_counterparty_upgrade(ctx, port, chan)?)
    {
        return Err(sdk::error::go_panic(
            cherr::invalid_upgrade_sequence(format!(
                "attempting to write error receipt at sequence ({}) while upgrade information exists at the same sequence",
                receipt.sequence
            ))
            .log,
        ));
    }
    ctx.set(
        crate::STORE_KEY,
        key("upgradeError", port, chan),
        receipt.encode_to_vec(),
    )?;
    let cp = ch.counterparty.clone().unwrap_or_default();
    ctx.events.emit_all(with_message(sdk::event::event(
        "channel_upgrade_error",
        [
            ("port_id", port.to_owned()),
            ("channel_id", chan.to_owned()),
            ("counterparty_port_id", cp.port_id),
            ("counterparty_channel_id", cp.channel_id),
            ("upgrade_sequence", ch.upgrade_sequence.to_string()),
            ("error_receipt", e.err.log.clone()),
        ],
    )));
    Ok(())
}

/// `abortUpgrade` / `MustAbortUpgrade` (`err` is wrapped into an
/// `UpgradeError` at the channel's upgrade sequence unless it is one).
pub fn abort_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str, err: Failure) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?
        .ok_or_else(|| sdk::error::go_panic(not_found(port, chan).log))?;
    let ue = match err {
        Failure::Upgrade(u) => u,
        Failure::Err(e) => UpgradeError {
            sequence: ch.upgrade_sequence,
            err: e,
        },
    };
    let seq = ch.upgrade_sequence;
    restore_channel(ctx, port, chan, seq, ch)?;
    write_error_receipt(ctx, port, chan, &ue)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

fn upgrade_event(ty: &str, port: &str, chan: &str, ch: &Channel, up: &Upgrade) -> [Event; 2] {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let f = fields(up);
    with_message(sdk::event::event(
        ty,
        [
            ("port_id", port.to_owned()),
            ("channel_id", chan.to_owned()),
            ("counterparty_port_id", cp.port_id),
            ("counterparty_channel_id", cp.channel_id),
            ("connection_hops", first_hop(&f)),
            ("version", f.version.clone()),
            ("ordering", order_name(f.ordering).into_owned()),
            ("upgrade_sequence", ch.upgrade_sequence.to_string()),
        ],
    ))
}

fn state_event(ty: &str, port: &str, chan: &str, ch: &Channel, open: bool) -> [Event; 2] {
    let cp = ch.counterparty.clone().unwrap_or_default();
    let mut attrs = vec![
        ("port_id", port.to_owned()),
        ("channel_id", chan.to_owned()),
        ("channel_state", state_name(ch.state).into_owned()),
        ("counterparty_port_id", cp.port_id),
        ("counterparty_channel_id", cp.channel_id),
    ];
    if open {
        attrs.push(("connection_hops", hop(ch)));
        attrs.push(("version", ch.version.clone()));
        attrs.push(("ordering", order_name(ch.ordering).into_owned()));
    }
    attrs.push(("upgrade_sequence", ch.upgrade_sequence.to_string()));
    with_message(sdk::event::event(ty, attrs))
}

/// `emitChannelFlushCompleteEvent`.
#[must_use]
pub fn flush_complete_event(port: &str, chan: &str, ch: &Channel) -> [Event; 2] {
    let cp = ch.counterparty.clone().unwrap_or_default();
    with_message(sdk::event::event(
        "channel_flush_complete",
        [
            ("port_id", port.to_owned()),
            ("channel_id", chan.to_owned()),
            ("counterparty_port_id", cp.port_id),
            ("counterparty_channel_id", cp.channel_id),
            ("channel_state", state_name(ch.state).into_owned()),
        ],
    ))
}

// ---------------------------------------------------------------------------
// Keeper steps
// ---------------------------------------------------------------------------

/// `ChanUpgradeInit`.
pub fn chan_upgrade_init(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    f: &UpgradeFields,
) -> Result<Upgrade> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected STATE_OPEN, got {}",
            state_name(ch.state)
        )));
    }
    validate_self_upgrade_fields(ctx, f, &ch)?;
    Ok(Upgrade {
        fields: Some(f.clone()),
        ..Upgrade::default()
    })
}

/// `WriteUpgradeInitChannel`.
pub fn write_upgrade_init_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    mut up: Upgrade,
    version: &str,
) -> Result<(Channel, Upgrade)> {
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state in successful ChanUpgradeInit step, channelID: {chan}, portID: {port}"
        ))
    })?;
    if has_upgrade(ctx, port, chan)? {
        delete_upgrade_info(ctx, port, chan)?;
        let e = UpgradeError {
            sequence: ch.upgrade_sequence,
            err: cherr::invalid_upgrade(""),
        };
        write_error_receipt(ctx, port, chan, &e)?;
    }
    ch.upgrade_sequence = ch.upgrade_sequence.wrapping_add(1);
    let mut f = fields(&up);
    version.clone_into(&mut f.version);
    up.fields = Some(f);
    set_channel(ctx, port, chan, &ch)?;
    set_upgrade(ctx, port, chan, &up)?;
    Ok((ch, up))
}

/// `ChanUpgradeTry`.
#[allow(clippy::too_many_arguments)]
pub fn chan_upgrade_try(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    proposed_hops: &[String],
    cp_fields: &UpgradeFields,
    cp_seq: u64,
    channel_proof: &[u8],
    upgrade_proof: &[u8],
    proof_height: &Height,
) -> core::result::Result<(Channel, Upgrade), Failure> {
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Open as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected STATE_OPEN, got {}",
            state_name(ch.state)
        ))
        .into());
    }
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: State::Open as i32,
        ordering: ch.ordering,
        counterparty: counterparty(port, chan),
        connection_hops: vec![cp_conn_id(&conn)],
        version: ch.version.clone(),
        upgrade_sequence: cp_seq,
    };
    verify_cp_channel(ctx, &conn, &ch, proof_height, channel_proof, &expected)
        .map_err(|e| e.wrap("failed to verify counterparty channel state"))?;
    let existing = get_upgrade(ctx, port, chan)?;
    let crossing_hello = existing.is_some();
    let mut up = existing.unwrap_or_default();
    let expected_seq = if crossing_hello {
        ch.upgrade_sequence
    } else {
        ch.upgrade_sequence.wrapping_add(1)
    };
    if cp_seq < expected_seq {
        return Err(upgrade_err(
            expected_seq.wrapping_sub(1),
            cherr::invalid_upgrade_sequence(format!(
                "counterparty upgrade sequence < current upgrade sequence ({cp_seq} < {})",
                ch.upgrade_sequence
            )),
        ));
    }
    let cp_upgrade = Upgrade {
        fields: Some(cp_fields.clone()),
        timeout: Some(Timeout::default()),
        next_sequence_send: 0,
    };
    verify_cp_upgrade(ctx, &conn, &ch, proof_height, upgrade_proof, &cp_upgrade)
        .map_err(|e| e.wrap("failed to verify counterparty upgrade"))?;
    let mut proposed = UpgradeFields {
        ordering: cp_fields.ordering,
        connection_hops: proposed_hops.to_vec(),
        version: cp_fields.version.clone(),
    };
    if crossing_hello {
        proposed = fields(&up);
    } else {
        let init = chan_upgrade_init(ctx, port, chan, &proposed)
            .map_err(|e| e.wrap("failed to initialize upgrade"))?;
        let version = fields(&init).version;
        (ch, up) = write_upgrade_init_channel(ctx, port, chan, init, &version)?;
    }
    check_compatibility(ctx, &proposed, cp_fields)
        .map_err(|e| e.wrap("failed upgrade compatibility check"))?;
    if cp_seq > ch.upgrade_sequence {
        ch.upgrade_sequence = cp_seq;
        set_channel(ctx, port, chan, &ch)?;
    }
    start_flushing(ctx, port, chan, &mut up)?;
    Ok((ch, up))
}

/// `WriteUpgradeTryChannel`.
pub fn write_upgrade_try_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    mut up: Upgrade,
    version: &str,
) -> Result<(Channel, Upgrade)> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state in successful ChanUpgradeTry step, channelID: {chan}, portID: {port}"
        ))
    })?;
    let mut f = fields(&up);
    version.clone_into(&mut f.version);
    up.fields = Some(f);
    set_upgrade(ctx, port, chan, &up)?;
    Ok((ch, up))
}

/// `ChanUpgradeAck`.
#[allow(clippy::too_many_arguments)]
pub fn chan_upgrade_ack(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_upgrade: &Upgrade,
    channel_proof: &[u8],
    upgrade_proof: &[u8],
    proof_height: &Height,
) -> core::result::Result<(), Failure> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Open as i32 && ch.state != State::Flushing as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected one of [STATE_OPEN, STATE_FLUSHING], got {}",
            state_name(ch.state)
        ))
        .into());
    }
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: State::Flushing as i32,
        ordering: ch.ordering,
        connection_hops: vec![cp_conn_id(&conn)],
        counterparty: counterparty(port, chan),
        version: ch.version.clone(),
        upgrade_sequence: ch.upgrade_sequence,
    };
    verify_cp_channel(ctx, &conn, &ch, proof_height, channel_proof, &expected)
        .map_err(|e| e.wrap("failed to verify counterparty channel state"))?;
    verify_cp_upgrade(ctx, &conn, &ch, proof_height, upgrade_proof, cp_upgrade)
        .map_err(|e| e.wrap("failed to verify counterparty upgrade"))?;
    let mut up = get_upgrade(ctx, port, chan)?.ok_or_else(|| {
        cherr::upgrade_not_found(format!(
            "failed to retrieve channel upgrade: port ID ({port}) channel ID ({chan})"
        ))
    })?;
    let cpf = fields(cp_upgrade);
    if ch.state == State::Open as i32 {
        let mut f = fields(&up);
        f.version.clone_from(&cpf.version);
        up.fields = Some(f);
    }
    if let Err(e) = check_compatibility(ctx, &fields(&up), &cpf) {
        return Err(upgrade_err(ch.upgrade_sequence, e));
    }
    if ch.state == State::Open as i32 {
        start_flushing(ctx, port, chan, &mut up)?;
    }
    let t = cp_upgrade.timeout.unwrap_or_default();
    let (h, ts) = self_height_time(ctx)?;
    if elapsed(&t, &h, ts) {
        return Err(upgrade_err(
            ch.upgrade_sequence,
            err_timeout_elapsed(&t, &h, ts).wrap("counterparty upgrade timeout elapsed"),
        ));
    }
    Ok(())
}

/// `WriteUpgradeAckChannel`.
pub fn write_upgrade_ack_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_upgrade: &Upgrade,
) -> Result<(Channel, Upgrade)> {
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state in successful ChanUpgradeAck step, channelID: {chan}, portID: {port}"
        ))
    })?;
    if !has_inflight_packets(ctx, port, chan)? {
        ch.state = State::Flushcomplete as i32;
        set_channel(ctx, port, chan, &ch)?;
    }
    let mut up = get_upgrade(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing upgrade when updating channel state in successful ChanUpgradeAck step, channelID: {chan}, portID: {port}"
        ))
    })?;
    let mut f = fields(&up);
    f.version = fields(cp_upgrade).version;
    up.fields = Some(f);
    set_upgrade(ctx, port, chan, &up)?;
    set_counterparty_upgrade(ctx, port, chan, cp_upgrade)?;
    Ok((ch, up))
}

/// `ChanUpgradeConfirm`.
#[allow(clippy::too_many_arguments)]
pub fn chan_upgrade_confirm(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_state: i32,
    cp_upgrade: &Upgrade,
    channel_proof: &[u8],
    upgrade_proof: &[u8],
    proof_height: &Height,
) -> core::result::Result<(), Failure> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Flushing as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected STATE_FLUSHING, got {}",
            state_name(ch.state)
        ))
        .into());
    }
    if cp_state != State::Flushing as i32 && cp_state != State::Flushcomplete as i32 {
        return Err(cherr::invalid_counterparty(format!(
            "expected one of [STATE_FLUSHING, STATE_FLUSHCOMPLETE], got {}",
            state_name(cp_state)
        ))
        .into());
    }
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    let expected = Channel {
        state: cp_state,
        ordering: ch.ordering,
        connection_hops: vec![cp_conn_id(&conn)],
        counterparty: counterparty(port, chan),
        version: ch.version.clone(),
        upgrade_sequence: ch.upgrade_sequence,
    };
    verify_cp_channel(ctx, &conn, &ch, proof_height, channel_proof, &expected)
        .map_err(|e| e.wrap("failed to verify counterparty channel state"))?;
    verify_cp_upgrade(ctx, &conn, &ch, proof_height, upgrade_proof, cp_upgrade)
        .map_err(|e| e.wrap("failed to verify counterparty upgrade"))?;
    let up = get_upgrade(ctx, port, chan)?.ok_or_else(|| {
        cherr::upgrade_not_found(format!(
            "failed to retrieve channel upgrade: port ID ({port}) channel ID ({chan})"
        ))
    })?;
    if let Err(e) = check_compatibility(ctx, &fields(&up), &fields(cp_upgrade)) {
        return Err(upgrade_err(ch.upgrade_sequence, e));
    }
    let t = cp_upgrade.timeout.unwrap_or_default();
    let (h, ts) = self_height_time(ctx)?;
    if elapsed(&t, &h, ts) {
        return Err(upgrade_err(
            ch.upgrade_sequence,
            err_timeout_elapsed(&t, &h, ts).wrap("counterparty upgrade timeout elapsed"),
        ));
    }
    Ok(())
}

/// `WriteUpgradeConfirmChannel`.
pub fn write_upgrade_confirm_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_upgrade: &Upgrade,
) -> Result<Channel> {
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state in successful ChanUpgradeConfirm step, channelID: {chan}, portID: {port}"
        ))
    })?;
    if !has_inflight_packets(ctx, port, chan)? {
        ch.state = State::Flushcomplete as i32;
        set_channel(ctx, port, chan, &ch)?;
    }
    set_counterparty_upgrade(ctx, port, chan, cp_upgrade)?;
    Ok(ch)
}

/// `ChanUpgradeOpen`.
pub fn chan_upgrade_open(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_state: i32,
    cp_seq: u64,
    channel_proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Flushcomplete as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected STATE_FLUSHCOMPLETE, got {}",
            state_name(ch.state)
        )));
    }
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    let expected = if cp_state == State::Open as i32 {
        let up = get_upgrade(ctx, port, chan)?.ok_or_else(|| {
            cherr::upgrade_not_found(format!(
                "failed to retrieve channel upgrade: port ID ({port}) channel ID ({chan})"
            ))
        })?;
        let f = fields(&up);
        let up_hop = f
            .connection_hops
            .first()
            .cloned()
            .ok_or_else(index0_panic)?;
        let up_conn = crate::connection::get_connection(ctx, &up_hop)?
            .ok_or_else(|| conerr::connection_not_found(&up_hop))?;
        connection_open(&up_conn)?;
        if cp_seq < ch.upgrade_sequence {
            return Err(cherr::invalid_upgrade_sequence(format!(
                "counterparty channel upgrade sequence ({cp_seq}) must be greater than or equal to current upgrade sequence ({})",
                ch.upgrade_sequence
            )));
        }
        Channel {
            state: State::Open as i32,
            ordering: f.ordering,
            connection_hops: vec![cp_conn_id(&up_conn)],
            counterparty: counterparty(port, chan),
            version: f.version,
            upgrade_sequence: cp_seq,
        }
    } else if cp_state == State::Flushcomplete as i32 {
        Channel {
            state: State::Flushcomplete as i32,
            ordering: ch.ordering,
            connection_hops: vec![cp_conn_id(&conn)],
            counterparty: counterparty(port, chan),
            version: ch.version.clone(),
            upgrade_sequence: ch.upgrade_sequence,
        }
    } else {
        return Err(cherr::invalid_counterparty(format!(
            "counterparty channel state must be one of [STATE_OPEN, STATE_FLUSHCOMPLETE], got {}",
            state_name(cp_state)
        )));
    };
    verify_cp_channel(ctx, &conn, &ch, proof_height, channel_proof, &expected)
        .map_err(|e| e.wrap("failed to verify counterparty channel"))
}

/// `WriteUpgradeOpenChannel`.
pub fn write_upgrade_open_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Channel> {
    let panic = |what: &str| {
        sdk::error::go_panic(format!(
            "could not find {what} when updating channel state, channelID: {chan}, portID: {port}"
        ))
    };
    let mut ch = get_channel(ctx, port, chan)?.ok_or_else(|| panic("existing channel"))?;
    let up = get_upgrade(ctx, port, chan)?.ok_or_else(|| panic("upgrade"))?;
    let cp_up = get_counterparty_upgrade_or(ctx, port, chan)?
        .ok_or_else(|| panic("counterparty upgrade"))?;
    let f = fields(&up);
    if ch.ordering == Order::Ordered as i32 && f.ordering == Order::Unordered as i32 {
        channel::set_next_sequence_recv(ctx, port, chan, 1)?;
        channel::set_next_sequence_ack(ctx, port, chan, 1)?;
    }
    if ch.ordering == Order::Unordered as i32 && f.ordering == Order::Ordered as i32 {
        channel::set_next_sequence_recv(ctx, port, chan, cp_up.next_sequence_send)?;
        channel::set_next_sequence_ack(ctx, port, chan, up.next_sequence_send)?;
    }
    set_recv_start_sequence(ctx, port, chan, cp_up.next_sequence_send)?;
    if !ctx.has(crate::STORE_KEY, &pruning_start_key(port, chan))? {
        channel::put_u64(ctx, pruning_start_key(port, chan), 1)?;
    }
    ch.ordering = f.ordering;
    ch.version = f.version;
    ch.connection_hops = f.connection_hops;
    ch.state = State::Open as i32;
    set_channel(ctx, port, chan, &ch)?;
    delete_upgrade_info(ctx, port, chan)?;
    Ok(ch)
}

fn get_counterparty_upgrade_or(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<Option<Upgrade>> {
    channel::get_counterparty_upgrade(ctx, port, chan)
}

/// `ChanUpgradeCancel` (the non-authority path).
pub fn chan_upgrade_cancel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    receipt: &ErrorReceipt,
    proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if get_upgrade(ctx, port, chan)?.is_none() {
        return Err(cherr::upgrade_not_found(format!(
            "port ID ({port}) channel ID ({chan})"
        )));
    }
    if proof.is_empty() {
        return Err(commerr::invalid_proof(
            "cannot submit an empty error receipt proof unless the sender is authorized to cancel upgrades AND channel is not in FLUSHCOMPLETE",
        ));
    }
    if ch.state == State::Flushcomplete as i32 && receipt.sequence != ch.upgrade_sequence {
        return Err(cherr::invalid_upgrade_sequence(format!(
            "error receipt sequence ({}) must be equal to current upgrade sequence ({}) when the channel is in FLUSHCOMPLETE",
            receipt.sequence, ch.upgrade_sequence
        )));
    }
    if receipt.sequence < ch.upgrade_sequence {
        return Err(cherr::invalid_upgrade_sequence(format!(
            "error receipt sequence ({}) must be greater than or equal to current upgrade sequence ({})",
            receipt.sequence, ch.upgrade_sequence
        )));
    }
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    verify_cp_error_receipt(ctx, &conn, &ch, proof_height, proof, receipt)
        .map_err(|e| e.wrap("failed to verify counterparty error receipt"))
}

/// `WriteUpgradeCancelChannel`.
pub fn write_upgrade_cancel_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state, channelID: {chan}, portID: {port}"
        ))
    })?;
    restore_channel(ctx, port, chan, seq, ch)?;
    let e = UpgradeError {
        sequence: seq,
        err: cherr::invalid_upgrade(""),
    };
    write_error_receipt(ctx, port, chan, &e)
}

/// `ChanUpgradeTimeout`.
pub fn chan_upgrade_timeout(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    cp_channel: &Channel,
    proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| not_found(port, chan))?;
    if ch.state != State::Flushing as i32 && ch.state != State::Flushcomplete as i32 {
        return Err(cherr::invalid_channel_state(format!(
            "expected one of [STATE_FLUSHING, STATE_FLUSHCOMPLETE], got {}",
            state_name(ch.state)
        )));
    }
    let up = get_upgrade(ctx, port, chan)?
        .ok_or_else(|| cherr::upgrade_not_found(format!("port ID ({port}) channel ID ({chan})")))?;
    let hop0 = ch
        .connection_hops
        .first()
        .cloned()
        .ok_or_else(index0_panic)?;
    let conn = crate::connection::get_connection(ctx, &hop0)?
        .ok_or_else(|| conerr::connection_not_found(&hop0))?;
    connection_open(&conn)?;
    let cs = crate::client::client_state(ctx, &conn.client_id)?.ok_or_else(|| {
        crate::errors::client::client_not_found(format!("clientID ({})", conn.client_id))
    })?;
    let proof_ts = crate::client::get_timestamp_at_height(ctx, &conn.client_id, &cs, proof_height)?;
    let t = up.timeout.unwrap_or_default();
    if !elapsed(&t, proof_height, proof_ts) {
        return Err(
            err_timeout_not_reached(&t, proof_height, proof_ts).wrap("upgrade timeout not reached")
        );
    }
    if cp_channel.state != State::Open as i32 && cp_channel.state != State::Flushing as i32 {
        return Err(cherr::invalid_counterparty(format!(
            "expected one of [STATE_OPEN, STATE_FLUSHING], got {}",
            state_name(cp_channel.state)
        )));
    }
    if cp_channel.state == State::Open as i32 {
        let f = fields(&up);
        let up_hop = f
            .connection_hops
            .first()
            .cloned()
            .ok_or_else(index0_panic)?;
        let up_conn = crate::connection::get_connection(ctx, &up_hop)?
            .ok_or_else(|| conerr::connection_not_found(&up_hop))?;
        let cp_hop = cp_conn_id(&up_conn);
        if f.version == cp_channel.version
            && f.ordering == cp_channel.ordering
            && cp_channel.connection_hops.first().map(String::as_str) == Some(cp_hop.as_str())
        {
            return Err(cherr::upgrade_timeout_failed(
                "counterparty channel is already upgraded",
            ));
        }
    }
    if cp_channel.upgrade_sequence < ch.upgrade_sequence {
        return Err(cherr::invalid_upgrade_sequence(format!(
            "counterparty channel upgrade sequence ({}) must be greater than or equal to current upgrade sequence ({})",
            cp_channel.upgrade_sequence, ch.upgrade_sequence
        )));
    }
    verify_cp_channel(ctx, &conn, &ch, proof_height, proof, cp_channel)
        .map_err(|e| e.wrap("failed to verify counterparty channel state"))
}

/// `WriteUpgradeTimeoutChannel`.
pub fn write_upgrade_timeout_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<(Channel, Upgrade)> {
    let ch = get_channel(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing channel when updating channel state in successful ChanUpgradeTimeout step, channelID: {chan}, portID: {port}"
        ))
    })?;
    let up = get_upgrade(ctx, port, chan)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "could not find existing upgrade when cancelling channel upgrade, channelID: {chan}, portID: {port}"
        ))
    })?;
    let seq = ch.upgrade_sequence;
    let ch = restore_channel(ctx, port, chan, seq, ch)?;
    let e = UpgradeError {
        sequence: ch.upgrade_sequence,
        err: cherr::upgrade_timeout(""),
    };
    write_error_receipt(ctx, port, chan, &e)?;
    Ok((ch, up))
}

/// Flushing on acknowledgement (`AcknowledgePacket`, after the packet
/// event): abort on an elapsed counterparty timeout, else complete the
/// flush when no packet is in flight.
pub fn flush_on_ack(ctx: &mut Context<'_>, port: &str, chan: &str, mut ch: Channel) -> Result<()> {
    if ch.state != State::Flushing as i32 {
        return Ok(());
    }
    let Some(cp_up) = channel::get_counterparty_upgrade(ctx, port, chan)? else {
        return Ok(());
    };
    let t = cp_up.timeout.unwrap_or_default();
    let (h, ts) = self_height_time(ctx)?;
    if elapsed(&t, &h, ts) {
        return abort_upgrade(
            ctx,
            port,
            chan,
            Failure::Err(err_timeout_elapsed(&t, &h, ts)),
        );
    }
    if !has_inflight_packets(ctx, port, chan)? {
        ch.state = State::Flushcomplete as i32;
        set_channel(ctx, port, chan, &ch)?;
        ctx.events.emit_all(flush_complete_event(port, chan, &ch));
    }
    Ok(())
}

/// Flushing on timeout (`TimeoutExecuted` of an UNORDERED channel).
pub fn flush_on_timeout(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    ch: &mut Channel,
) -> Result<()> {
    if ch.state != State::Flushing as i32 || ch.ordering != Order::Unordered as i32 {
        return Ok(());
    }
    let Some(cp_up) = channel::get_counterparty_upgrade(ctx, port, chan)? else {
        return Ok(());
    };
    let t = cp_up.timeout.unwrap_or_default();
    let (h, ts) = self_height_time(ctx)?;
    if elapsed(&t, &h, ts) {
        abort_upgrade(
            ctx,
            port,
            chan,
            Failure::Err(err_timeout_elapsed(&t, &h, ts)),
        )?;
    } else if !has_inflight_packets(ctx, port, chan)? {
        ch.state = State::Flushcomplete as i32;
        set_channel(ctx, port, chan, ch)?;
        ctx.events.emit_all(flush_complete_event(port, chan, ch));
    }
    Ok(())
}

/// The ORDERED-channel timeout while flushing aborts the upgrade.
pub fn abort_on_ordered_timeout(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<()> {
    abort_upgrade(
        ctx,
        port,
        chan,
        Failure::Err(cherr::timeout_elapsed(
            "packet timeout elapsed on ORDERED channel",
        )),
    )
}

// ---------------------------------------------------------------------------
// Message events
// ---------------------------------------------------------------------------

/// `EmitChannelUpgradeInitEvent` / Try / Ack / Timeout / Cancel (same
/// attributes, by event type).
#[must_use]
pub fn emit_upgrade(ty: &str, port: &str, chan: &str, ch: &Channel, up: &Upgrade) -> [Event; 2] {
    if ty == "channel_upgrade_timeout" {
        let cp = ch.counterparty.clone().unwrap_or_default();
        let f = fields(up);
        let t = up.timeout.unwrap_or_default();
        return with_message(sdk::event::event(
            ty,
            [
                ("port_id", port.to_owned()),
                ("channel_id", chan.to_owned()),
                ("counterparty_port_id", cp.port_id),
                ("counterparty_channel_id", cp.channel_id),
                ("connection_hops", first_hop(&f)),
                ("version", f.version.clone()),
                ("ordering", order_name(f.ordering).into_owned()),
                (
                    "timeout_height",
                    host::height_string(&t.height.unwrap_or_default()),
                ),
                ("timeout_timestamp", t.timestamp.to_string()),
                ("upgrade_sequence", ch.upgrade_sequence.to_string()),
            ],
        ));
    }
    upgrade_event(ty, port, chan, ch, up)
}

/// `EmitChannelUpgradeConfirmEvent`.
#[must_use]
pub fn emit_confirm(port: &str, chan: &str, ch: &Channel) -> [Event; 2] {
    state_event("channel_upgrade_confirm", port, chan, ch, false)
}

/// `EmitChannelUpgradeOpenEvent`.
#[must_use]
pub fn emit_open(port: &str, chan: &str, ch: &Channel) -> [Event; 2] {
    state_event("channel_upgrade_open", port, chan, ch, true)
}

/// `chpb::ResponseResultType` values.
pub use chpb::ResponseResultType;
