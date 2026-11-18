//! The packet forward `IBCMiddleware` (`ibc_middleware.go`) and its keeper
//! logic (`keeper/keeper.go`) wrapped around the ICS-20 transfer app.

use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::applications::transfer::v1::MsgTransfer;
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet};
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::packetforward::v1::InFlightPacket;
use prost::Message;
use sdk::coins::{Amount, Coin};
use sdk::{Context, Result, SdkError};
use x_ibc::channel::AckResponse;
use x_ibc::router::{Acknowledgement, IbcModule};
use x_transfer::relay::{self, PacketData};

use crate::memo::{self, ForwardMetadata, Pfm, Recv};
use crate::{STORE_KEY, refund_packet_key};

/// `DefaultForwardTransferPacketTimeoutTimestamp`: 10 minutes.
pub const DEFAULT_FORWARD_TIMEOUT_NS: i64 = 600_000_000_000;

/// `packetforward.IBCMiddleware`.
#[derive(Debug, Clone)]
pub struct PacketForwardMiddleware<A> {
    /// The wrapped application (transfer).
    pub app: A,
    /// `retriesOnTimeout`.
    pub retries_on_timeout: u8,
    /// `forwardTimeout` (nanoseconds).
    pub forward_timeout: i64,
}

impl<A> PacketForwardMiddleware<A> {
    /// `NewIBCMiddleware(app, k, 0, DefaultForwardTransferPacketTimeoutTimestamp)`,
    /// as celestia-app wires it: no retries and a 10 minute timeout unless
    /// the memo sets them.
    pub const fn new(app: A) -> Self {
        Self {
            app,
            retries_on_timeout: 0,
            forward_timeout: DEFAULT_FORWARD_TIMEOUT_NS,
        }
    }

    /// `OnRecvPacket`'s timeout (nanoseconds) and retries for the forward
    /// metadata `md`: the middleware's own when the memo sets none (or a
    /// timeout that is not positive).
    #[must_use]
    pub fn timeout_and_retries(&self, md: &ForwardMetadata) -> (i64, u8) {
        let timeout = if md.timeout <= 0 {
            self.forward_timeout
        } else {
            md.timeout
        };
        (timeout, md.retries.unwrap_or(self.retries_on_timeout))
    }
}

fn is_panic(e: &SdkError) -> bool {
    e.codespace == "undefined" && e.code == 111_222
}

/// A plain Go `error` (`fmt.Errorf`).
fn plain(log: impl Into<String>) -> SdkError {
    x_ibc::errors::unregistered(log)
}

/// The prefix of every error acknowledgement text PFM creates.
const ERROR_ACK_PREFIX: &str = "packet-forward-middleware error: ";

/// The fork's `ErrorAcknowledgementMessage`: the text of every error
/// acknowledgement it creates (it logs the error instead).
pub const FORK_ERROR_ACK_MESSAGE: &str = "packet-forward-middleware error: forward failed";

/// The fork's `types.EventTypeForwardError`: the event it emits with each
/// error acknowledgement it creates.
pub const EVENT_TYPE_FORWARD_ERROR: &str = "packet_forward_error";

/// The fork's `types.AttributeKeyError`: the error text.
pub const ATTRIBUTE_KEY_ERROR: &str = "error";

/// The event of the fork's `newErrorAcknowledgement(ctx, dstPort,
/// dstChannel, sequence, err)`: `packet_forward_error` with the error text
/// and ibc-go's identifiers of the packet the acknowledgement is for.
/// Upstream v8.2.0 (before app version 12) emits none. Events are not part
/// of any consensus hash and use no gas.
fn emit_forward_error(
    ctx: &mut Context<'_>,
    dst_port: &str,
    dst_channel: &str,
    sequence: u64,
    err: &str,
) {
    if pfm(ctx) != Pfm::Fork {
        return;
    }
    ctx.events.emit(sdk::event::event(
        EVENT_TYPE_FORWARD_ERROR,
        [
            (ATTRIBUTE_KEY_ERROR, err.to_owned()),
            ("packet_sequence", sequence.to_string()),
            ("packet_dst_port", dst_port.to_owned()),
            ("packet_dst_channel", dst_channel.to_owned()),
        ],
    ));
}

/// The text of PFM's `newErrorAcknowledgement(err)`.
fn pfm_error_text(ctx: &Context<'_>, err: &str) -> String {
    match pfm(ctx) {
        Pfm::Upstream => format!("{ERROR_ACK_PREFIX}{err}"),
        Pfm::Fork => FORK_ERROR_ACK_MESSAGE.to_owned(),
    }
}

/// `newErrorAcknowledgement` for the received packet `p` (the fork's
/// `recvErrorAcknowledgement`), then (from app version 12) celestia-app's
/// `pfmIBCMiddleware`, which commits [`fixed_error_ack`] instead.
fn error_ack(ctx: &mut Context<'_>, p: &Packet, err: &str) -> Acknowledgement {
    emit_forward_error(
        ctx,
        &p.destination_port,
        &p.destination_channel,
        p.sequence,
        err,
    );
    if is_v12(ctx) {
        return Acknowledgement::error_string(&fixed_error_ack());
    }
    Acknowledgement::error_string(&pfm_error_text(ctx, err))
}

/// Whether the context executes app version 12 or later.
fn is_v12(ctx: &Context<'_>) -> bool {
    let v = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    sdk::version::is_v12(sdk::version::executing(v))
}

/// The PFM release of the executing app version: the Celestia fork from
/// app version 12, upstream v8.2.0 before.
fn pfm(ctx: &Context<'_>) -> Pfm {
    if is_v12(ctx) {
        Pfm::Fork
    } else {
        Pfm::Upstream
    }
}

/// celestia-app v12 `fixedPFMErrorAck`: the text of
/// `channeltypes.NewErrorAcknowledgement(apperrors.ErrPacketForward)`
/// (codespace `app`, code 11151), which carries only the ABCI code. Go
/// writes the original PFM error text to the node log only, never to an
/// event, so no Go error text is needed from app version 12.
fn fixed_error_ack() -> String {
    "ABCI code: 11151: error handling packet: see events for details".to_owned()
}

/// `GetReceiver`: `bech32(address.Hash(ModuleName, "<channel>/<sender>")[:20])`.
#[must_use]
pub fn get_receiver(channel: &str, original_sender: &str) -> String {
    let typ = hash::sha256(crate::MODULE_NAME.as_bytes());
    let mut pre = typ.to_vec();
    let sender = format!("{channel}/{original_sender}");
    pre.extend_from_slice(&gostr::to_go_bytes(&sender));
    let h = hash::sha256(&pre);
    sdk::address::to_bech32(sdk::address::ACC_PREFIX, &h[..20])
}

/// `getDenomForThisChain`.
#[must_use]
pub fn denom_for_this_chain(
    port: &str,
    channel: &str,
    cp_port: &str,
    cp_channel: &str,
    denom: &str,
) -> String {
    let cp_prefix = format!("{cp_port}/{cp_channel}/");
    if let Some(unwound) = denom.strip_prefix(&cp_prefix) {
        let trace = relay::parse_denom_trace(unwound);
        if trace.path.is_empty() {
            return unwound.to_owned();
        }
        return x_transfer::ibc_denom(&trace);
    }
    x_transfer::ibc_denom(&relay::parse_denom_trace(&format!(
        "{port}/{channel}/{denom}"
    )))
}

fn parse_amount(s: &str) -> Option<Amount> {
    Amount::parse(s).ok()
}

/// The `MsgTransfer` that `ForwardTransferPacket` sends for the forward
/// metadata `md`: `token` from `sender` to `md`'s receiver, port and
/// channel, `timeout` nanoseconds after the block time, with `md`'s next
/// memo (upstream: re-encoded, the fork: as sent).
pub fn forward_msg_transfer(
    ctx: &Context<'_>,
    md: &ForwardMetadata,
    token: &Coin,
    sender: &str,
    timeout: i64,
) -> Result<MsgTransfer> {
    let memo = match &md.next {
        Some(Ok(s)) => s.clone(),
        Some(Err(e)) => return Err(sdk::error::sdk::json_marshal(e.clone())),
        None => String::new(),
    };
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let now = x_ibc::lightclients::tendermint::block_time(ctx) as i64 as u64;
    #[allow(clippy::cast_sign_loss)]
    Ok(MsgTransfer {
        source_port: md.port.clone(),
        source_channel: md.channel.clone(),
        token: Some(token.to_proto()),
        sender: sender.to_owned(),
        receiver: md.receiver.clone(),
        timeout_height: Some(Height::default()),
        timeout_timestamp: now.wrapping_add(timeout as u64),
        memo,
    })
}

#[cfg(test)]
thread_local! {
    /// Every `MsgTransfer` sent to the transfer keeper (tests only).
    static SENT: std::cell::RefCell<Vec<MsgTransfer>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `ForwardTransferPacket`.
#[allow(clippy::too_many_arguments)]
fn forward_transfer_packet(
    ctx: &mut Context<'_>,
    in_flight: Option<InFlightPacket>,
    src: &Packet,
    src_sender: &str,
    receiver: &str,
    md: &ForwardMetadata,
    token: &Coin,
    max_retries: u8,
    timeout: i64,
    nonrefundable: bool,
) -> Result<()> {
    let msg = forward_msg_transfer(ctx, md, token, receiver, timeout)?;
    #[cfg(test)]
    SENT.with(|s| s.borrow_mut().push(msg.clone()));
    let res = relay::transfer(ctx, &msg).map_err(|e| {
        if is_panic(&e) {
            e
        } else {
            sdk::error::sdk::insufficient_funds(e.log)
        }
    })?;
    let ifp = match in_flight {
        Some(mut p) => {
            p.retries_remaining -= 1;
            p
        }
        None => InFlightPacket {
            packet_data: src.data.clone(),
            original_sender_address: src_sender.to_owned(),
            refund_channel_id: src.destination_channel.clone(),
            refund_port_id: src.destination_port.clone(),
            refund_sequence: src.sequence,
            packet_src_port_id: src.source_port.clone(),
            packet_src_channel_id: src.source_channel.clone(),
            packet_timeout_timestamp: src.timeout_timestamp,
            packet_timeout_height: x_ibc::height::to_string(
                &src.timeout_height.unwrap_or_default(),
            ),
            retries_remaining: i32::from(max_retries),
            #[allow(clippy::cast_sign_loss)]
            timeout: timeout as u64,
            nonrefundable,
        },
    };
    let key = refund_packet_key(&md.channel, &md.port, res.sequence);
    ctx.set(STORE_KEY, key, ifp.encode_to_vec())
}

fn decode_in_flight(bz: &[u8]) -> Result<InFlightPacket> {
    InFlightPacket::decode(bz).map_err(|e| sdk::error::go_panic(e.to_string()))
}

/// `GetAndClearInFlightPacket`.
fn get_and_clear_in_flight_packet(
    ctx: &mut Context<'_>,
    channel: &str,
    port: &str,
    sequence: u64,
) -> Result<Option<InFlightPacket>> {
    let key = refund_packet_key(channel, port, sequence);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    ctx.delete(STORE_KEY, &key)?;
    decode_in_flight(&bz).map(Some)
}

/// `userRecoverableAccount`.
fn user_recoverable_account(ifp: &InFlightPacket) -> Result<Vec<u8>> {
    // Go's `sender, err := sdk.AccAddressFromBech32(...)` shadows `err`:
    // only an unmarshal error joins the fallback's.
    let first = match PacketData::decode(&ifp.packet_data) {
        Ok(d) => match sdk::address::acc_from_bech32(&d.receiver) {
            Ok(a) => return Ok(a),
            Err(_) => None,
        },
        Err(e) => Some(e),
    };
    // `bech32.DecodeAndConvert`: any prefix, up to 1023 characters.
    match encoding::bech32::decode_and_convert(&ifp.original_sender_address) {
        Ok((_, bz)) => Ok(bz),
        Err(e) => {
            let joined = match first {
                Some(f) => format!("{f}\n{e}"),
                None => e,
            };
            Err(plain(format!(
                "failed to get user recoverable account: failed to decode bech32 addresses: {joined}"
            )))
        }
    }
}

/// `moveFundsToUserRecoverableAccount`.
fn move_funds_to_user_recoverable_account(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &PacketData,
    ifp: &InFlightPacket,
) -> Result<()> {
    let full = data.denom.clone();
    let amount = parse_amount(&data.amount).ok_or_else(|| {
        plain(format!(
            "failed to parse amount from packet data for forward recovery: {}",
            data.amount
        ))
    })?;
    let token = relay::new_coin(
        &x_transfer::ibc_denom(&relay::parse_denom_trace(&full)),
        amount,
    )?;
    let user = user_recoverable_account(ifp)?;
    if !relay::sender_chain_is_source(&p.source_port, &p.source_channel, &full) {
        x_bank::mint_coins(ctx, x_transfer::MODULE_NAME, &relay::coins(&token))?;
        return x_bank::send_from_module_to_account(
            ctx,
            x_transfer::MODULE_NAME,
            &user,
            &relay::coins(&token),
        )
        .map_err(|e| {
            sdk::error::go_panic(format!(
                "unable to send coins from module to account despite previously minting coins to module account: {}",
                e.log
            ))
        });
    }
    let escrow = x_transfer::escrow_address(&p.source_port, &p.source_channel);
    x_bank::send_coins(ctx, &escrow, &user, &relay::coins(&token)).map_err(|e| {
        plain(format!(
            "failed to send coins from escrow account to user recoverable account: {}",
            e.log
        ))
    })?;
    relay::sub_escrow(ctx, &token)
}

/// `WriteAcknowledgementForForwardedPacket`.
fn write_ack_for_forwarded_packet(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &PacketData,
    ifp: &InFlightPacket,
    ack: &AckResponse,
) -> Result<()> {
    let (_, cap) =
        x_ibc::channel::lookup_module_by_channel(ctx, &ifp.refund_port_id, &ifp.refund_channel_id)
            .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    let mut ack = ack.clone();
    if !ack.success() {
        if ifp.nonrefundable {
            move_funds_to_user_recoverable_account(ctx, p, data, ifp)?;
            let err = match &ack {
                AckResponse::Error(e) => e.clone(),
                _ => String::new(),
            };
            ack = AckResponse::Result(
                format!("packet forward failed after point of no return: {err}").into_bytes(),
            );
            return write_refund_ack(ctx, &cap, ifp, &ack);
        }
        let mut full = data.denom.clone();
        if data.denom.starts_with("ibc/") {
            full = relay::denom_path_from_hash(ctx, &data.denom)?;
        }
        let amount = parse_amount(&data.amount).ok_or_else(|| {
            plain(format!(
                "failed to parse amount from packet data for forward refund: {}",
                data.amount
            ))
        })?;
        let token = relay::new_coin(
            &x_transfer::ibc_denom(&relay::parse_denom_trace(&full)),
            amount,
        )?;
        let escrow = x_transfer::escrow_address(&p.source_port, &p.source_channel);
        let refund_escrow = x_transfer::escrow_address(&ifp.refund_port_id, &ifp.refund_channel_id);
        let new_token = relay::coins(&token);
        if relay::sender_chain_is_source(&p.source_port, &p.source_channel, &full) {
            if relay::sender_chain_is_source(&ifp.refund_port_id, &ifp.refund_channel_id, &full) {
                x_bank::send_coins(ctx, &escrow, &refund_escrow, &new_token).map_err(|e| {
                    plain(format!(
                        "failed to send coins from escrow account to refund escrow account: {}",
                        e.log
                    ))
                })?;
            } else {
                x_bank::send_from_account_to_module(
                    ctx,
                    &escrow,
                    x_transfer::MODULE_NAME,
                    &new_token,
                )
                .map_err(|e| {
                    plain(format!(
                        "failed to send coins from escrow to module account for burn: {}",
                        e.log
                    ))
                })?;
                x_bank::burn_coins(ctx, x_transfer::MODULE_NAME, &new_token).map_err(|e| {
                    sdk::error::go_panic(format!(
                        "cannot burn coins after a successful send from escrow account to module account: {}",
                        e.log
                    ))
                })?;
                relay::sub_escrow(ctx, &token)?;
            }
        } else {
            x_bank::mint_coins(ctx, x_transfer::MODULE_NAME, &new_token).map_err(|e| {
                plain(format!(
                    "cannot mint coins to the {} module account: {}",
                    x_transfer::MODULE_NAME,
                    e.log
                ))
            })?;
            x_bank::send_from_module_to_account(
                ctx,
                x_transfer::MODULE_NAME,
                &refund_escrow,
                &new_token,
            )
            .map_err(|e| {
                plain(format!(
                    "cannot send coins from the {} module to the escrow account {}: {}",
                    x_transfer::MODULE_NAME,
                    sdk::address::to_bech32(sdk::address::ACC_PREFIX, &refund_escrow),
                    e.log
                ))
            })?;
            relay::add_escrow(ctx, &token)?;
        }
    }
    write_refund_ack(ctx, &cap, ifp, &ack)
}

/// celestia-app v12 `pfmICS4Wrapper`: a PFM error acknowledgement of a
/// forwarded packet (made here or relayed from the next chain) is replaced
/// before it is written. `None` keeps `ack`.
fn fixed_forwarded_ack(ctx: &mut Context<'_>, ack: &AckResponse) -> Option<AckResponse> {
    match ack {
        AckResponse::Error(e) if e.starts_with(ERROR_ACK_PREFIX) && is_v12(ctx) => {
            Some(AckResponse::Error(fixed_error_ack()))
        }
        _ => None,
    }
}

/// `ics4Wrapper.WriteAcknowledgement` on the reconstructed inbound packet.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn write_refund_ack(
    ctx: &mut Context<'_>,
    cap: &Capability,
    ifp: &InFlightPacket,
    ack: &AckResponse,
) -> Result<()> {
    let timeout_height = x_ibc::height::parse(&ifp.packet_timeout_height)
        .map_err(|e| sdk::error::go_panic(format!("MustParseHeight: {}", e.log)))?;
    let fixed = fixed_forwarded_ack(ctx, ack);
    let ack = fixed.as_ref().unwrap_or(ack);
    let packet = Packet {
        sequence: ifp.refund_sequence,
        source_port: ifp.packet_src_port_id.clone(),
        source_channel: ifp.packet_src_channel_id.clone(),
        destination_port: ifp.refund_port_id.clone(),
        destination_channel: ifp.refund_channel_id.clone(),
        data: ifp.packet_data.clone(),
        timeout_height: Some(timeout_height),
        timeout_timestamp: ifp.packet_timeout_timestamp,
    };
    x_ibc::channel::write_acknowledgement(ctx, cap, &packet, &gostr::to_go_bytes(&ack.marshal()))
}

/// `TimeoutShouldRetry`: `None` when the packet was not forwarded; the
/// error is set when retries are exhausted.
fn timeout_should_retry(
    ctx: &mut Context<'_>,
    p: &Packet,
) -> Result<Option<(InFlightPacket, Option<String>)>> {
    let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    let ifp = decode_in_flight(&bz)?;
    if ifp.retries_remaining <= 0 {
        let e = format!(
            "giving up on packet on channel ({}) port ({}) after max retries",
            ifp.refund_channel_id, ifp.refund_port_id
        );
        return Ok(Some((ifp, Some(e))));
    }
    Ok(Some((ifp, None)))
}

/// `RemoveInFlightPacket`.
fn remove_in_flight_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<()> {
    let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(());
    }
    ctx.delete(STORE_KEY, &key)
}

/// `RetryTimeout`.
fn retry_timeout(
    ctx: &mut Context<'_>,
    channel: &str,
    port: &str,
    data: &PacketData,
    ifp: InFlightPacket,
) -> Result<()> {
    let mut md = ForwardMetadata {
        receiver: data.receiver.clone(),
        channel: channel.to_owned(),
        port: port.to_owned(),
        ..ForwardMetadata::default()
    };
    // Upstream decodes and re-encodes the memo; the fork sends it as is.
    md.next = memo::retry_next(&data.memo, pfm(ctx))
        .map_err(|e| plain(format!("error unmarshaling memo json: {e}")))?;
    let amount = parse_amount(&data.amount).ok_or_else(|| {
        plain(format!(
            "error parsing amount from string for packetforward retry: {}",
            data.amount
        ))
    })?;
    let denom = x_transfer::ibc_denom(&relay::parse_denom_trace(&data.denom));
    let token = relay::new_coin(&denom, amount)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let retries = ifp.retries_remaining as u8;
    #[allow(clippy::cast_possible_wrap)]
    let timeout = ifp.timeout as i64;
    let nonrefundable = ifp.nonrefundable;
    forward_transfer_packet(
        ctx,
        Some(ifp),
        &Packet::default(),
        "",
        &data.sender,
        &md,
        &token,
        retries,
        timeout,
        nonrefundable,
    )
}

impl<A: IbcModule> PacketForwardMiddleware<A> {
    /// `receiveFunds`.
    fn receive_funds(
        &self,
        ctx: &mut Context<'_>,
        p: &Packet,
        data: &PacketData,
        override_receiver: &str,
        relayer: &[u8],
    ) -> Result<core::result::Result<(), String>> {
        let override_data = PacketData {
            denom: data.denom.clone(),
            amount: data.amount.clone(),
            sender: data.sender.clone(),
            receiver: override_receiver.to_owned(),
            memo: String::new(),
        };
        let mut op = p.clone();
        op.data = override_data.marshal_unsorted();
        Ok(match self.app.on_recv_packet(ctx, &op, relayer)? {
            None => Err("ack is nil".into()),
            Some(a) if !a.success => {
                Err(format!("ack error: {}", String::from_utf8_lossy(&a.bytes)))
            }
            Some(_) => Ok(()),
        })
    }
}

impl<A: IbcModule> IbcModule for PacketForwardMiddleware<A> {
    fn on_chan_open_init(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        version: &str,
    ) -> Result<String> {
        self.app.on_chan_open_init(
            ctx,
            order,
            connection_hops,
            port_id,
            channel_id,
            cap,
            counterparty,
            version,
        )
    }

    fn on_chan_open_try(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        counterparty_version: &str,
    ) -> Result<String> {
        self.app.on_chan_open_try(
            ctx,
            order,
            connection_hops,
            port_id,
            channel_id,
            cap,
            counterparty,
            counterparty_version,
        )
    }

    fn on_chan_open_ack(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        counterparty_channel_id: &str,
        counterparty_version: &str,
    ) -> Result<()> {
        self.app.on_chan_open_ack(
            ctx,
            port_id,
            channel_id,
            counterparty_channel_id,
            counterparty_version,
        )
    }

    fn on_chan_open_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.app.on_chan_open_confirm(ctx, port_id, channel_id)
    }

    fn on_chan_close_init(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.app.on_chan_close_init(ctx, port_id, channel_id)
    }

    fn on_chan_close_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.app.on_chan_close_confirm(ctx, port_id, channel_id)
    }

    fn on_recv_packet(
        &self,
        ctx: &mut Context<'_>,
        p: &Packet,
        relayer: &[u8],
    ) -> Result<Option<Acknowledgement>> {
        let Ok(data) = PacketData::decode(&p.data) else {
            return self.app.on_recv_packet(ctx, p, relayer);
        };
        let md = match memo::on_recv(&data.memo, pfm(ctx)) {
            Recv::NotForward => return self.app.on_recv_packet(ctx, p, relayer),
            Recv::Error(e) => return Ok(Some(error_ack(ctx, p, &e))),
            Recv::NilForwardPanic => {
                return Err(sdk::error::go_panic(
                    "runtime error: invalid memory address or nil pointer dereference",
                ));
            }
            Recv::Forward(m) => m,
        };
        let override_receiver = get_receiver(&p.destination_channel, &data.sender);
        if let Err(e) = self.receive_funds(ctx, p, &data, &override_receiver, relayer)? {
            return Ok(Some(error_ack(
                ctx,
                p,
                &format!("error receiving packet: {e}"),
            )));
        }
        let denom = denom_for_this_chain(
            &p.destination_port,
            &p.destination_channel,
            &p.source_port,
            &p.source_channel,
            &data.denom,
        );
        let Some(amount) = parse_amount(&data.amount) else {
            return Ok(Some(error_ack(
                ctx,
                p,
                &format!("error parsing amount for forward: {}", data.amount),
            )));
        };
        let token = relay::new_coin(&denom, amount)?;
        let (timeout, retries) = self.timeout_and_retries(&md);
        if let Err(e) = forward_transfer_packet(
            ctx,
            None,
            p,
            &data.sender,
            &override_receiver,
            &md,
            &token,
            retries,
            timeout,
            false,
        ) {
            if is_panic(&e) {
                return Err(e);
            }
            return Ok(Some(error_ack(ctx, p, &e.log)));
        }
        Ok(None)
    }

    fn on_acknowledgement_packet(
        &self,
        ctx: &mut Context<'_>,
        p: &Packet,
        acknowledgement: &[u8],
        relayer: &[u8],
    ) -> Result<()> {
        let Ok(data) = PacketData::decode(&p.data) else {
            return self
                .app
                .on_acknowledgement_packet(ctx, p, acknowledgement, relayer);
        };
        let ack = x_ibc::channel::decode_acknowledgement(acknowledgement).map_err(|e| {
            sdk::error::sdk::unknown_request(format!(
                "cannot unmarshal ICS-20 transfer packet acknowledgement: {e}"
            ))
        })?;
        if let Some(ifp) =
            get_and_clear_in_flight_packet(ctx, &p.source_channel, &p.source_port, p.sequence)?
        {
            return write_ack_for_forwarded_packet(ctx, p, &data, &ifp, &ack);
        }
        self.app
            .on_acknowledgement_packet(ctx, p, acknowledgement, relayer)
    }

    fn on_timeout_packet(&self, ctx: &mut Context<'_>, p: &Packet, relayer: &[u8]) -> Result<()> {
        let Ok(data) = PacketData::decode(&p.data) else {
            return self.app.on_timeout_packet(ctx, p, relayer);
        };
        let Some((ifp, err)) = timeout_should_retry(ctx, p)? else {
            return self.app.on_timeout_packet(ctx, p, relayer);
        };
        remove_in_flight_packet(ctx, p)?;
        if let Some(e) = err {
            // The fork emits the event (for the source packet) before it
            // writes the acknowledgement.
            emit_forward_error(
                ctx,
                &ifp.refund_port_id,
                &ifp.refund_channel_id,
                ifp.refund_sequence,
                &e,
            );
            let ack = AckResponse::Error(pfm_error_text(ctx, &e));
            return write_ack_for_forwarded_packet(ctx, p, &data, &ifp, &ack);
        }
        self.app.on_timeout_packet(ctx, p, relayer)?;
        retry_timeout(ctx, &p.source_channel, &p.source_port, &data, ifp)
    }

    fn upgradable(&self) -> bool {
        // `IBCMiddleware` does not implement `UpgradableModule`.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_metadata_acknowledgements_match_pinned_go_bytes() {
        // Invalid metadata returns before transfer keepers. These vectors call
        // the real PFM callback, but do not exercise IBC proof verification.
        let fixtures = [
            (
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../../conformance/fixtures/packetforward/memo-validation-acks.json"
                )),
                false,
            ),
            (
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../../conformance/fixtures/packetforward/memo-parse-error-acks.json"
                )),
                true,
            ),
        ];
        let module = PacketForwardMiddleware {
            app: x_transfer::relay::TransferModule,
            retries_on_timeout: 0,
            forward_timeout: DEFAULT_FORWARD_TIMEOUT_NS,
        };
        for (fixture, parse_errors) in fixtures {
            let cases: serde_json::Value = serde_json::from_str(fixture).unwrap();
            for case in cases.as_array().unwrap() {
                if parse_errors {
                    assert!(
                        matches!(
                            memo::classify(case["memo"].as_str().unwrap()),
                            memo::Memo::ParseError(_)
                        ),
                        "{}",
                        case["name"]
                    );
                }
                if case["valid"] == true {
                    continue;
                }
                let mut store = sdk::MultiStore::new();
                let mut ctx =
                    Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
                let packet = Packet {
                    source_port: "transfer".into(),
                    source_channel: "channel-0".into(),
                    destination_port: "transfer".into(),
                    destination_channel: "channel-1".into(),
                    data: serde_json::to_vec(&serde_json::json!({
                        "denom": "uatom", "amount": "1", "sender": "remote", "receiver": "pfm",
                        "memo": case["memo"].as_str().unwrap(),
                    }))
                    .unwrap(),
                    ..Default::default()
                };
                let ack = module
                    .on_recv_packet(&mut ctx, &packet, &[])
                    .unwrap()
                    .unwrap();
                let expected = encoding::hex::decode(case["ack_hex"].as_str().unwrap()).unwrap();
                assert!(!ack.success, "{}", case["name"]);
                assert_eq!(ack.bytes, expected, "{}", case["name"]);
                assert_eq!(
                    x_ibc::channel::commit_acknowledgement(&ack.bytes),
                    encoding::hex::decode(case["commitment_hex"].as_str().unwrap()).unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }

    /// From app version 12, celestia-app commits a fixed PFM error
    /// acknowledgement that carries only the ABCI code of its
    /// `ErrPacketForward` (PROTOCO-3759), whatever the error text. The
    /// fork emits the error text in a `packet_forward_error` event instead.
    #[test]
    fn error_acknowledgements_are_fixed_from_v12() {
        let module = PacketForwardMiddleware {
            app: x_transfer::relay::TransferModule,
            retries_on_timeout: 0,
            forward_timeout: DEFAULT_FORWARD_TIMEOUT_NS,
        };
        let memos = [
            (
                r#"{"forward":{"receiver":"cosmos1abc","port":"","channel":"channel-0"}}"#,
                "failed to validate metadata: identifier cannot be blank: invalid identifier [celestiaorg/ibc-go/v8@v8.7.3/modules/core/24-host/validate.go:41]",
            ),
            (
                r#"{"forward":{"receiver":"cosmos1abc","port":"transfer","channel":"channel/0"}}"#,
                "failed to validate metadata: identifier channel/0 cannot contain separator '/': invalid identifier [celestiaorg/ibc-go/v8@v8.7.3/modules/core/24-host/validate.go:45]",
            ),
            (
                r#"{"forward":{"receiver":"","port":"transfer","channel":"channel-0"}}"#,
                "failed to validate metadata. receiver cannot be empty",
            ),
            (
                r#"{"forward":{"receiver":1}}"#,
                "error parsing forward metadata: json: cannot unmarshal number into Go struct field ForwardMetadata.forward.receiver of type string",
            ),
            (
                r#"{"forward":{"receiver":"cosmos1abc","port":"transfer","channel":"channel-0","retries":-1}}"#,
                "error parsing forward metadata: json: cannot unmarshal number -1 into Go struct field ForwardMetadata.forward.retries of type uint8",
            ),
        ];
        for (memo, error) in memos {
            let mut store = sdk::MultiStore::new();
            let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
            ctx.consensus_params.version = Some(
                celestia_proto::celestia_core_proto::tendermint::types::VersionParams { app: 12 },
            );
            let packet = Packet {
                sequence: 7,
                source_port: "transfer".into(),
                source_channel: "channel-0".into(),
                destination_port: "transfer".into(),
                destination_channel: "channel-1".into(),
                data: serde_json::to_vec(&serde_json::json!({
                    "denom": "uatom", "amount": "1", "sender": "remote", "receiver": "pfm",
                    "memo": memo,
                }))
                .unwrap(),
                ..Default::default()
            };
            let ack = module
                .on_recv_packet(&mut ctx, &packet, &[])
                .unwrap()
                .unwrap();
            assert!(!ack.success, "{memo}");
            assert_eq!(
                String::from_utf8(ack.bytes).unwrap(),
                r#"{"error":"ABCI code: 11151: error handling packet: see events for details"}"#,
                "{memo}"
            );
            assert_eq!(
                forward_error_events(&ctx),
                vec![vec![
                    ("error".to_owned(), error.to_owned()),
                    ("packet_sequence".to_owned(), "7".to_owned()),
                    ("packet_dst_port".to_owned(), "transfer".to_owned()),
                    ("packet_dst_channel".to_owned(), "channel-1".to_owned()),
                ]],
                "{memo}"
            );
        }
    }

    /// The attributes of each `packet_forward_error` event emitted so far.
    fn forward_error_events(ctx: &Context<'_>) -> ErrorEvents {
        ctx.events
            .events()
            .iter()
            .filter(|e| e.r#type == "packet_forward_error")
            .map(|e| {
                e.attributes
                    .iter()
                    .map(|a| (a.key.clone(), a.value.clone()))
                    .collect()
            })
            .collect()
    }

    /// Upstream v8.2.0 emits no `packet_forward_error` event: before app
    /// version 12 the error text is in the acknowledgement.
    #[test]
    fn no_forward_error_event_before_v12() {
        for app in [10, 11] {
            let (res, _, events) = recv_events(app, r#"{"forward":{"receiver":""}}"#);
            assert!(!res.unwrap().unwrap().success);
            assert!(events.is_empty(), "{app}");
            let (_, events) = give_up(app);
            assert!(events.is_empty(), "{app}");
        }
    }

    /// From app version 12, each error acknowledgement of `OnRecvPacket`
    /// emits one `packet_forward_error` event, also after funds are
    /// received; a forward emits none.
    #[test]
    fn forward_error_event_from_v12() {
        // The forward fails at the transfer (no IBC state here), after the
        // funds were received.
        let (res, received, events) = recv_events(12, &forward(""));
        assert!(!res.unwrap().unwrap().success);
        assert_eq!(received, 1);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0][0].0, "error");
        assert!(
            events[0][0].1.ends_with(": insufficient funds"),
            "{events:?}"
        );
        assert_eq!(events[0][1], ("packet_sequence".into(), "1".into()));
        // Not a forward: the transfer app answers, no event.
        let (res, _, events) = recv_events(12, "hello");
        assert!(res.unwrap().unwrap().success);
        assert!(events.is_empty());
    }

    /// From app version 12, the error acknowledgement written when a
    /// forwarded packet times out with no retries left emits one
    /// `packet_forward_error` event for the source packet (refund port,
    /// channel and sequence), before the acknowledgement is written.
    #[test]
    fn give_up_emits_forward_error_event_from_v12() {
        let (_, events) = give_up(12);
        assert_eq!(
            events,
            vec![vec![
                (
                    "error".to_owned(),
                    "giving up on packet on channel (channel-99) port (transfer) after max retries"
                        .to_owned()
                ),
                ("packet_sequence".to_owned(), "9".to_owned()),
                ("packet_dst_port".to_owned(), "transfer".to_owned()),
                ("packet_dst_channel".to_owned(), "channel-99".to_owned()),
            ]]
        );
    }

    #[test]
    fn forwarded_error_acknowledgements_are_fixed_from_v12() {
        let timeout = AckResponse::Error(
            "packet-forward-middleware error: giving up on packet on channel (channel-1) port (transfer) after max retries".into(),
        );
        let ibc_go = AckResponse::Error(
            "ABCI code: 5: error handling packet: see events for details".into(),
        );
        let result = AckResponse::Result(b"packet-forward-middleware error: x".to_vec());
        for app in [10, 11, 12] {
            let mut store = sdk::MultiStore::new();
            let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
            ctx.consensus_params.version =
                Some(celestia_proto::celestia_core_proto::tendermint::types::VersionParams { app });
            let fixed = fixed_forwarded_ack(&mut ctx, &timeout);
            if app == 12 {
                assert_eq!(
                    fixed.map(|a| a.marshal()).as_deref(),
                    Some(
                        r#"{"error":"ABCI code: 11151: error handling packet: see events for details"}"#
                    )
                );
            } else {
                assert!(fixed.is_none());
            }
            assert!(fixed_forwarded_ack(&mut ctx, &ibc_go).is_none());
            assert!(fixed_forwarded_ack(&mut ctx, &result).is_none());
        }
    }

    #[test]
    fn receiver_and_denoms() {
        // The override receiver is a module-style derived address.
        assert!(get_receiver("channel-0", "celestia1abc").starts_with("celestia1"));
        assert_eq!(
            denom_for_this_chain(
                "transfer",
                "channel-0",
                "transfer",
                "channel-1",
                "transfer/channel-1/utia"
            ),
            "utia"
        );
        assert!(
            denom_for_this_chain("transfer", "channel-0", "transfer", "channel-1", "stake")
                .starts_with("ibc/")
        );
    }

    #[test]
    fn receiver_hashes_go_string_bytes() {
        // Go v1.26.9 / packet-forward-middleware v8.2.0 GetReceiver vector.
        // Exercise the JSON decoder used by on_recv_packet: U+10FFFF becomes
        // four reserved gostr chars internally, then GetReceiver hashes its
        // original four UTF-8 bytes.
        let json = "{\"denom\":\"utia\",\"amount\":\"1\",\"sender\":\"\u{10ffff}\",\"receiver\":\"pfm\",\"memo\":\"\"}"
            .to_string();
        let data = PacketData::decode(json.as_bytes()).unwrap();
        assert_eq!(
            gostr::to_go_bytes(&data.sender).as_ref(),
            "\u{10ffff}".as_bytes()
        );
        assert_eq!(
            get_receiver("channel-0", &data.sender),
            "celestia1v67n437r7naz4p0zrpry7grxes753qwyng7vla"
        );
        assert!(
            data.marshal_unsorted()
                .windows("\u{10ffff}".len())
                .any(|window| window == "\u{10ffff}".as_bytes())
        );
    }

    /// The fallback is `bech32.DecodeAndConvert`: any prefix, up to 1023
    /// characters, so a 90-character prefix (beyond BIP-173's 83) decodes
    /// (security.md finding 74); the shadowed `err` leaves the receiver's
    /// error out of the joined text.
    #[test]
    fn user_recoverable_account_falls_back_like_go() {
        let long_hrp = format!("{}1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc53m8tvl", "a".repeat(90));
        let data = br#"{"amount":"1","denom":"utia","receiver":"pfm","sender":"s"}"#.to_vec();
        let ifp = |sender: &str| InFlightPacket {
            original_sender_address: sender.to_owned(),
            packet_data: data.clone(),
            ..Default::default()
        };
        assert_eq!(
            user_recoverable_account(&ifp(&long_hrp)).unwrap(),
            (1..=20).collect::<Vec<u8>>()
        );
        assert_eq!(
            user_recoverable_account(&ifp("x")).unwrap_err().log,
            "failed to get user recoverable account: failed to decode bech32 addresses: \
             decoding bech32 failed: invalid bech32 string length 1"
        );
    }

    /// The `MsgTransfer`s sent to the transfer keeper on this thread since
    /// the last call.
    fn take_sent() -> Vec<MsgTransfer> {
        SENT.with(|s| std::mem::take(&mut *s.borrow_mut()))
    }

    const FIXED_ACK: &str =
        r#"{"error":"ABCI code: 11151: error handling packet: see events for details"}"#;

    fn transfer_packet(memo: &str) -> Packet {
        Packet {
            sequence: 1,
            source_port: "transfer".into(),
            source_channel: "channel-1".into(),
            destination_port: "transfer".into(),
            destination_channel: "channel-0".into(),
            data: serde_json::to_vec(&serde_json::json!({
                "denom": "uatom", "amount": "1", "sender": "remote", "receiver": "pfm",
                "memo": memo,
            }))
            .unwrap(),
            ..Default::default()
        }
    }

    fn set_app_version(ctx: &mut Context<'_>, app: u64) {
        ctx.consensus_params.version =
            Some(celestia_proto::celestia_core_proto::tendermint::types::VersionParams { app });
    }

    /// PFM's own error text: upstream's carries the Go error; the fork's
    /// is fixed (celestia-app then commits [`fixed_error_ack`] from 12).
    #[test]
    fn pfm_error_text_is_fixed_in_the_fork() {
        let give_up =
            "giving up on packet on channel (channel-0) port (transfer) after max retries";
        for (app, want) in [
            (10, format!("{ERROR_ACK_PREFIX}{give_up}")),
            (11, format!("{ERROR_ACK_PREFIX}{give_up}")),
            (12, FORK_ERROR_ACK_MESSAGE.to_owned()),
        ] {
            let mut store = sdk::MultiStore::new();
            let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
            set_app_version(&mut ctx, app);
            assert_eq!(pfm_error_text(&ctx, give_up), want);
            let ack = error_ack(&mut ctx, &Packet::default(), give_up);
            let committed = if app == 12 {
                FIXED_ACK.to_owned()
            } else {
                format!(r#"{{"error":"{want}"}}"#)
            };
            assert_eq!(String::from_utf8(ack.bytes).unwrap(), committed);
            // The fork's text keeps the prefix that celestia-app replaces.
            assert!(FORK_ERROR_ACK_MESSAGE.starts_with(ERROR_ACK_PREFIX));
        }
    }

    /// An application that accepts every packet and counts the packets it
    /// receives (PFM's `receiveFunds` and pass-through).
    #[derive(Default)]
    struct OkApp {
        received: std::cell::Cell<u32>,
    }

    impl IbcModule for OkApp {
        fn on_chan_open_init(
            &self,
            _: &mut Context<'_>,
            _: Order,
            _: &[String],
            _: &str,
            _: &str,
            _: &Capability,
            _: &Counterparty,
            _: &str,
        ) -> Result<String> {
            unreachable!()
        }
        fn on_chan_open_try(
            &self,
            _: &mut Context<'_>,
            _: Order,
            _: &[String],
            _: &str,
            _: &str,
            _: &Capability,
            _: &Counterparty,
            _: &str,
        ) -> Result<String> {
            unreachable!()
        }
        fn on_chan_open_ack(
            &self,
            _: &mut Context<'_>,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<()> {
            unreachable!()
        }
        fn on_chan_open_confirm(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            unreachable!()
        }
        fn on_chan_close_init(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            unreachable!()
        }
        fn on_chan_close_confirm(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            unreachable!()
        }
        fn on_recv_packet(
            &self,
            _: &mut Context<'_>,
            _: &Packet,
            _: &[u8],
        ) -> Result<Option<Acknowledgement>> {
            self.received.set(self.received.get() + 1);
            Ok(Some(Acknowledgement::result(&[1])))
        }
        fn on_acknowledgement_packet(
            &self,
            _: &mut Context<'_>,
            _: &Packet,
            _: &[u8],
            _: &[u8],
        ) -> Result<()> {
            Ok(())
        }
        fn on_timeout_packet(&self, _: &mut Context<'_>, _: &Packet, _: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    fn ok_module() -> PacketForwardMiddleware<OkApp> {
        PacketForwardMiddleware {
            app: OkApp::default(),
            retries_on_timeout: 0,
            forward_timeout: DEFAULT_FORWARD_TIMEOUT_NS,
        }
    }

    /// `OnRecvPacket` at app version `app`: the result and how many packets
    /// reached the wrapped app.
    fn recv(app: u64, memo: &str) -> (Result<Option<Acknowledgement>>, u32) {
        let mut store = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
        set_app_version(&mut ctx, app);
        let m = ok_module();
        let res = m.on_recv_packet(&mut ctx, &transfer_packet(memo), &[]);
        (res, m.app.received.get())
    }

    /// The attributes of `packet_forward_error` events.
    type ErrorEvents = Vec<Vec<(String, String)>>;

    /// [`recv`] and the `packet_forward_error` events it emitted.
    fn recv_events(app: u64, memo: &str) -> (Result<Option<Acknowledgement>>, u32, ErrorEvents) {
        let mut store = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
        set_app_version(&mut ctx, app);
        let m = ok_module();
        let res = m.on_recv_packet(&mut ctx, &transfer_packet(memo), &[]);
        (res, m.app.received.get(), forward_error_events(&ctx))
    }

    /// Times out a forwarded packet with no retries left (source packet:
    /// sequence 9 on transfer/channel-99). Writing the acknowledgement
    /// fails here (no IBC state); returns that result and the
    /// `packet_forward_error` events.
    fn give_up(app: u64) -> (Result<()>, ErrorEvents) {
        let mut store = sdk::MultiStore::new();
        store.mount(STORE_KEY, sdk::Kind::Iavl).unwrap();
        let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
        set_app_version(&mut ctx, app);
        let p = transfer_packet("");
        let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
        crate::set_in_flight_packet(
            &mut ctx,
            &key,
            &InFlightPacket {
                refund_channel_id: "channel-99".into(),
                refund_port_id: "transfer".into(),
                refund_sequence: 9,
                retries_remaining: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let res = ok_module().on_timeout_packet(&mut ctx, &p, &[]);
        (res, forward_error_events(&ctx))
    }

    fn forward(fields: &str) -> String {
        format!(
            r#"{{"forward":{{"receiver":"r","port":"transfer","channel":"channel-0"{fields}}}}}"#
        )
    }

    /// From app version 12 the fork rejects these memos with an error
    /// acknowledgement (celestia-app commits its fixed one) before any
    /// funds are received. Before 12, upstream v8.2.0 receives the funds
    /// first (or panics on a forward that decodes to nil).
    #[test]
    fn recv_uses_the_fork_from_v12() {
        let over = forward(&format!(
            r#","pad":"{}""#,
            "x".repeat(crate::memo::MAX_FORWARD_MEMO_LENGTH)
        ));
        let nil = format!(
            r#"{},"Forward":null}}"#,
            &forward("")[..forward("").len() - 1]
        );
        for (memo, upstream_panics) in [
            (nil.as_str(), true),
            (&forward(r#","timeout":1e19"#), false),
            (&forward(r#","next":"hello""#), false),
            (&over, false),
        ] {
            for app in [10, 11] {
                let (res, received) = recv(app, memo);
                if upstream_panics {
                    assert!(res.is_err_and(|e| is_panic(&e)), "{app} {memo:.80}");
                } else {
                    assert_eq!(received, 1, "{app} {memo:.80}: {res:?}");
                }
            }
            let (res, received) = recv(12, memo);
            assert_eq!(received, 0, "{memo:.80}");
            let ack = res.unwrap().unwrap();
            assert!(!ack.success);
            assert_eq!(ack.bytes, FIXED_ACK.as_bytes(), "{memo:.80}");
        }
        // Memos the fork accepts, and memos that are no forward (of any
        // length), behave alike in every version.
        let big = format!(r#"{{"pad":"{}"}}"#, "x".repeat(40 << 10));
        for memo in [
            &forward(r#","next":{"a":"<"}"#),
            &forward(r#","timeout":"1s""#),
        ] {
            for app in [10, 11, 12] {
                assert_eq!(recv(app, memo).1, 1, "{app} {memo}");
            }
        }
        for app in [10, 11, 12] {
            let (res, received) = recv(app, &big);
            assert_eq!(received, 1);
            assert!(res.unwrap().unwrap().success);
        }
    }

    /// Runs `OnTimeoutPacket` (wrapped app: [`OkApp`]) for a forwarded
    /// packet with `memo` and one retry left; returns the error text.
    fn retry(app: u64, memo: &str) -> String {
        retry_sent(app, memo).0
    }

    /// [`retry`], and the `MsgTransfer` that the retry sent to the transfer
    /// keeper (if any).
    fn retry_sent(app: u64, memo: &str) -> (String, Option<MsgTransfer>) {
        take_sent();
        let mut store = sdk::MultiStore::new();
        store.mount(STORE_KEY, sdk::Kind::Iavl).unwrap();
        let mut ctx = Context::new(&mut store, sdk::Header::default(), sdk::ExecMode::Finalize);
        set_app_version(&mut ctx, app);
        let p = transfer_packet(memo);
        let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
        crate::set_in_flight_packet(
            &mut ctx,
            &key,
            &InFlightPacket {
                refund_channel_id: "channel-0".into(),
                refund_port_id: "transfer".into(),
                retries_remaining: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let err = ok_module()
            .on_timeout_packet(&mut ctx, &p, &[])
            .map_or_else(|e| e.log, |()| String::new());
        let mut sent = take_sent();
        assert!(sent.len() <= 1);
        (err, sent.pop())
    }

    /// From app version 12 the forwarded `MsgTransfer` carries `next` as
    /// the sender's bytes (a JSON string: its unquoted content); upstream
    /// re-encodes it. Both send the memo of the forward metadata to its
    /// receiver, port and channel, with the middleware's default timeout.
    #[test]
    fn forwarded_transfer_memo_from_v12() {
        let q = |s: &str| serde_json::to_string(s).unwrap();
        for (next, fork, upstream) in [
            (
                r#"{ "b" : 1, "a" : "<" }"#.to_owned(),
                r#"{ "b" : 1, "a" : "<" }"#,
                r#"{"b":1,"a":"\u003c"}"#,
            ),
            (
                q(r#"{ "a" : [ 1 ] }"#),
                r#"{ "a" : [ 1 ] }"#,
                r#"{"a":[1]}"#,
            ),
            (
                r#"{"a":1,"a":2}"#.to_owned(),
                r#"{"a":1,"a":2}"#,
                r#"{"a":2}"#,
            ),
            ("null".to_owned(), "", ""),
        ] {
            let memo = forward(&format!(r#","next":{next}"#));
            for (app, want) in [(10, upstream), (11, upstream), (12, fork)] {
                take_sent();
                let (res, received) = recv(app, &memo);
                // The transfer fails on missing stores, after it was sent.
                assert!(!res.unwrap().unwrap().success);
                assert_eq!(received, 1);
                let sent = take_sent();
                assert_eq!(sent.len(), 1, "{app} {memo}");
                let msg = &sent[0];
                assert_eq!(msg.memo, want, "{app} {memo}");
                assert_eq!(msg.receiver, "r");
                assert_eq!(msg.source_port, "transfer");
                assert_eq!(msg.source_channel, "channel-0");
                assert_eq!(msg.timeout_timestamp, DEFAULT_FORWARD_TIMEOUT_NS as u64);
            }
        }
    }

    /// From app version 12 a timed-out packet is retried with its memo as
    /// is, whatever the memo; upstream fails on a memo it cannot decode.
    #[test]
    fn retry_keeps_the_memo_from_v12() {
        let over = forward(&format!(r#","pad":"{}""#, "x".repeat(40 << 10)));
        let deep = format!("{}{}", "[".repeat(10_001), "]".repeat(10_001));
        for memo in ["hello", deep.as_str()] {
            for app in [10, 11] {
                let (err, sent) = retry_sent(app, memo);
                assert!(
                    err.starts_with("error unmarshaling memo json"),
                    "{app} {memo:.80}"
                );
                assert!(sent.is_none());
            }
        }
        // Upstream re-encodes the memo it sends again.
        let (_, sent) = retry_sent(10, r#"{ "a" : "<" }"#);
        assert_eq!(sent.unwrap().memo, r#"{"a":"\u003c"}"#);
        for memo in [
            "hello",
            deep.as_str(),
            over.as_str(),
            "null",
            r#"{"a":1,"a":2}"#,
        ] {
            let (err, sent) = retry_sent(12, memo);
            // The retry reaches the transfer, which fails on missing stores.
            assert!(
                !err.is_empty() && !err.contains("memo"),
                "{memo:.80}: {err}"
            );
            // It sent the memo exactly as the timed-out packet carried it.
            let sent = sent.unwrap_or_else(|| panic!("{memo:.80}: not sent"));
            assert_eq!(sent.memo, memo, "{memo:.80}");
            assert_eq!(sent.receiver, "pfm");
            if memo != "hello" && memo != deep {
                assert_eq!(err, retry(10, r#"{"a":1}"#), "{memo:.80}");
            }
        }
    }
}
