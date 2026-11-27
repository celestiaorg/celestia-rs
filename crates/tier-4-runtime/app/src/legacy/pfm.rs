//! packet-forward-middleware v6.1.2 in the transfer stack of app versions
//! 2 and 3 (celestia-app v3.13.0 `app.go`: token filter, then PFM, then
//! transfer; PFM is skipped at app version 1). PFM is wired with no
//! retries on timeout, a 10 minute forward timeout and the token filter
//! keeper (which passes calls to the channel keeper) as its ICS-4 wrapper.
//!
//! The memo decoding, the override receiver, the denom on this chain and
//! the forwarded `MsgTransfer` are the same in v6.1.2 as in upstream
//! v8.2.0, so they come from [`x_packetforward`] ([`Pfm::Upstream`]). The
//! keeper steps run here on the ibc-go v6 transfer of
//! [`super::ibc_transfer`] (no total-escrow tracking). Not ported, and so
//! aborting the block: a memo whose `forward` decodes to nil (a Go panic), and the refund of a
//! forwarded voucher (burn or mint).
//!
//! The voucher refund cannot happen on a celestia-app v1 to v3 chain:
//! the token filter in front of PFM rejects every received token that is
//! not returning to this chain, so no voucher is ever minted here
//! (mainnet's genesis holds only `utia` and no denom traces). A forward
//! therefore always sends a native token, which the refund moves from
//! escrow to escrow.

use celestia_proto::ibc::core::channel::v1::Packet;
use celestia_proto::packetforward::v1::InFlightPacket;
use prost::Message;
use sdk::coins::{Amount, Coin};
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};
use x_ibc::channel::{self, AckResponse};
use x_packetforward::memo::{self, ForwardMetadata, Pfm, Recv};
use x_packetforward::middleware::{
    PacketForwardMiddleware, denom_for_this_chain, forward_msg_transfer, get_receiver,
};
use x_packetforward::{STORE_KEY, refund_packet_key};
use x_transfer::relay as tr;

use super::ibc_transfer;

/// Whether PFM is in the transfer stack: app versions 2 and 3.
pub(super) fn runs(ctx: &Context<'_>) -> bool {
    ctx.consensus_params
        .version
        .as_ref()
        .is_some_and(|v| v.app >= 2)
}

fn not_ported(what: &str) -> SdkError {
    super::not_ported(errs::not_supported(format!(
        "packet-forward-middleware v6: {what}"
    )))
}

/// A plain Go `error` (`fmt.Errorf`).
fn plain(log: impl Into<String>) -> SdkError {
    x_ibc::errors::unregistered(log)
}

/// v6 `newErrorAcknowledgement`: the full error text, committed in the
/// acknowledgement.
fn error_ack(err: &str) -> AckResponse {
    AckResponse::Error(format!("packet-forward-middleware error: {err}"))
}

/// `IBCMiddleware.OnRecvPacket`. `None` is the asynchronous
/// acknowledgement of a forwarded packet: the core keeps the state and
/// writes no acknowledgement.
pub(super) fn on_recv_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<Option<AckResponse>> {
    let Ok(data) = tr::PacketData::decode(&p.data) else {
        return ibc_transfer::transfer_on_recv_packet(ctx, p).map(Some);
    };
    let md = match memo::on_recv(&data.memo, Pfm::Upstream) {
        Recv::NotForward => return ibc_transfer::transfer_on_recv_packet(ctx, p).map(Some),
        Recv::Error(e) => return Ok(Some(error_ack(&e))),
        Recv::NilForwardPanic => return Err(not_ported("forward metadata that decodes to nil")),
        Recv::Forward(m) => m,
    };
    let override_receiver = get_receiver(&p.destination_channel, &data.sender);
    if let Err(e) = receive_funds(ctx, p, &data, &override_receiver)? {
        return Ok(Some(error_ack(&format!("error receiving packet: {e}"))));
    }
    let denom = denom_for_this_chain(
        &p.destination_port,
        &p.destination_channel,
        &p.source_port,
        &p.source_channel,
        &data.denom,
    );
    let Ok(amount) = Amount::parse(&data.amount) else {
        return Ok(Some(error_ack(&format!(
            "error parsing amount for forward: {}",
            data.amount
        ))));
    };
    let token = tr::new_coin(&denom, amount)?;
    let (timeout, retries) = PacketForwardMiddleware::new(()).timeout_and_retries(&md);
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
    ) {
        return Ok(Some(error_ack(&e.log)));
    }
    Ok(None)
}

/// `receiveFunds`: the transfer app receives the packet with the override
/// receiver and no memo.
fn receive_funds(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &tr::PacketData,
    override_receiver: &str,
) -> Result<core::result::Result<(), String>> {
    let override_data = tr::PacketData {
        denom: data.denom.clone(),
        amount: data.amount.clone(),
        sender: data.sender.clone(),
        receiver: override_receiver.to_owned(),
        memo: String::new(),
    };
    let mut op = p.clone();
    op.data = override_data.marshal_unsorted();
    let ack = ibc_transfer::transfer_on_recv_packet(ctx, &op)?;
    if ack.success() {
        return Ok(Ok(()));
    }
    Ok(Err(format!("ack error: {}", ack.marshal())))
}

/// The `FeePercentage` param (`GetFeePercentage`, a read of the
/// `packetfowardmiddleware` subspace).
fn fee_percentage(ctx: &mut Context<'_>) -> Result<math::LegacyDec> {
    let raw = x_params::get_raw(ctx, x_packetforward::MODULE_NAME, b"FeePercentage")?.ok_or_else(
        || sdk::error::go_panic("UnmarshalJSON: packetfowardmiddleware FeePercentage is not set"),
    )?;
    let s: String = serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("UnmarshalJSON: FeePercentage: {e}")))?;
    math::LegacyDec::from_str_go(&s)
        .map_err(|e| sdk::error::go_panic(format!("UnmarshalJSON: FeePercentage: {e}")))
}

/// The fee part of `ForwardTransferPacket`: `RoundInt(amount * fee)` goes
/// from `receiver` to the community pool (distribution
/// `FundCommunityPool`); the rest is forwarded.
fn pay_fee(ctx: &mut Context<'_>, token: &Coin, receiver: &str) -> Result<Coin> {
    let go_math = |e: math::MathError| sdk::error::go_panic(e.to_string());
    let fee = math::LegacyDec::from_int(&token.amount)
        .checked_mul(&fee_percentage(ctx)?)
        .and_then(|d| d.round_int())
        .map_err(go_math)?;
    let packet = Coin {
        denom: token.denom.clone(),
        amount: token.amount.checked_sub(&fee).map_err(go_math)?,
    };
    if fee.is_positive() {
        let host = sdk::address::acc_from_bech32_go(receiver).map_err(errs::invalid_address)?;
        let fee_coins = sdk::coins::Coins::sanitized(vec![Coin {
            denom: token.denom.clone(),
            amount: fee,
        }])?;
        x_distribution::keeper::fund_community_pool(ctx, &fee_coins, &host)
            .map_err(|e| errs::insufficient_funds(e.log))?;
    }
    Ok(packet)
}

/// `ForwardTransferPacket`: the transfer msg server sends the token on,
/// and the in-flight packet is stored under the new packet's key.
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
) -> Result<()> {
    let packet_coin = pay_fee(ctx, token, receiver)?;
    let msg = forward_msg_transfer(ctx, md, &packet_coin, receiver, timeout)?;
    let sequence =
        ibc_transfer::transfer_sequence(ctx, &msg).map_err(|e| errs::insufficient_funds(e.log))?;
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
            nonrefundable: false,
        },
    };
    let key = refund_packet_key(&md.channel, &md.port, sequence);
    ctx.set(STORE_KEY, key, ifp.encode_to_vec())
}

fn decode_in_flight(bz: &[u8]) -> Result<InFlightPacket> {
    InFlightPacket::decode(bz).map_err(|e| sdk::error::go_panic(e.to_string()))
}

/// `GetAndClearInFlightPacket` for the acknowledged packet `p`.
pub(super) fn get_and_clear_in_flight(
    ctx: &mut Context<'_>,
    p: &Packet,
) -> Result<Option<InFlightPacket>> {
    if !runs(ctx) {
        return Ok(None);
    }
    let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    ctx.delete(STORE_KEY, &key)?;
    decode_in_flight(&bz).map(Some)
}

/// `IBCMiddleware.OnTimeoutPacket` after the packet data decoded: `false`
/// when `p` was not forwarded (the transfer app handles the timeout).
/// With retries left, the transfer app refunds and the token is sent
/// again; otherwise the in-flight packet is removed and the error
/// acknowledgement written for the source packet.
pub(super) fn on_timeout_packet(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &tr::PacketData,
    transfer_on_timeout: impl FnOnce(&mut Context<'_>) -> Result<()>,
) -> Result<bool> {
    if !runs(ctx) {
        return Ok(false);
    }
    // `TimeoutShouldRetry`.
    let key = refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(false);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    let ifp = decode_in_flight(&bz)?;
    if ifp.retries_remaining <= 0 {
        // `RemoveInFlightPacket`.
        if ctx.has(STORE_KEY, &key)? {
            ctx.delete(STORE_KEY, &key)?;
        }
        let e = format!(
            "giving up on packet on channel ({}) port ({}) after max retries",
            ifp.refund_channel_id, ifp.refund_port_id
        );
        write_ack_for_forwarded_packet(ctx, p, data, &ifp, &error_ack(&e))?;
        return Ok(true);
    }
    transfer_on_timeout(ctx)?;
    retry_timeout(ctx, &p.source_channel, &p.source_port, data, ifp)?;
    Ok(true)
}

/// `RetryTimeout` (v6: the old in-flight entry stays).
fn retry_timeout(
    ctx: &mut Context<'_>,
    channel: &str,
    port: &str,
    data: &tr::PacketData,
    ifp: InFlightPacket,
) -> Result<()> {
    let mut md = ForwardMetadata {
        receiver: data.receiver.clone(),
        channel: channel.to_owned(),
        port: port.to_owned(),
        ..ForwardMetadata::default()
    };
    md.next = memo::retry_next(&data.memo, Pfm::Upstream)
        .map_err(|e| plain(format!("error unmarshaling memo json: {e}")))?;
    let amount = Amount::parse(&data.amount).map_err(|_| {
        plain(format!(
            "error parsing amount from string for packetforward retry: {}",
            data.amount
        ))
    })?;
    let denom = x_transfer::ibc_denom(&tr::parse_denom_trace(&data.denom));
    let token = tr::new_coin(&denom, amount)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let retries = ifp.retries_remaining as u8;
    #[allow(clippy::cast_possible_wrap)]
    let timeout = ifp.timeout as i64;
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
    )
}

/// `WriteAcknowledgementForForwardedPacket`: on an error, the forwarded
/// native token moves from the forward channel's escrow back to the
/// source channel's escrow; then the acknowledgement is written for the
/// source packet.
pub(super) fn write_ack_for_forwarded_packet(
    ctx: &mut Context<'_>,
    p: &Packet,
    data: &tr::PacketData,
    ifp: &InFlightPacket,
    ack: &AckResponse,
) -> Result<()> {
    let (_, cap) =
        channel::lookup_module_by_channel(ctx, &ifp.refund_port_id, &ifp.refund_channel_id)
            .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    if !ack.success() {
        let mut full = data.denom.clone();
        if data.denom.starts_with("ibc/") {
            full = tr::denom_path_from_hash(ctx, &data.denom)?;
        }
        let amount = Amount::parse(&data.amount).map_err(|_| {
            plain(format!(
                "failed to parse amount from packet data for forward refund: {}",
                data.amount
            ))
        })?;
        let token = tr::new_coin(
            &x_transfer::ibc_denom(&tr::parse_denom_trace(&full)),
            amount,
        )?;
        if !tr::sender_chain_is_source(&p.source_port, &p.source_channel, &full)
            || !tr::sender_chain_is_source(&ifp.refund_port_id, &ifp.refund_channel_id, &full)
        {
            return Err(not_ported(
                "the refund of a forwarded voucher (burn or mint)",
            ));
        }
        let escrow = x_transfer::escrow_address(&p.source_port, &p.source_channel);
        let refund_escrow = x_transfer::escrow_address(&ifp.refund_port_id, &ifp.refund_channel_id);
        x_bank::send_coins(ctx, &escrow, &refund_escrow, &tr::coins(&token)).map_err(|e| {
            plain(format!(
                "failed to send coins from escrow account to refund escrow account: {}",
                e.log
            ))
        })?;
    }
    let timeout_height = x_ibc::height::parse(&ifp.packet_timeout_height)
        .map_err(|e| sdk::error::go_panic(format!("MustParseHeight: {}", e.log)))?;
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
    ibc_transfer::write_acknowledgement(ctx, &cap, &packet, ack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
    use sdk::{ExecMode, Header, MultiStore};

    fn v2_store() -> MultiStore {
        let mut ms = MultiStore::new();
        let v2 = sdk::version::Release::for_app_version(2).unwrap();
        crate::stores::mount_release(&mut ms, v2).unwrap();
        ms
    }

    fn at(ctx: &mut Context<'_>, app: u64) {
        ctx.consensus_params = ConsensusParams {
            version: Some(VersionParams { app }),
            ..ConsensusParams::default()
        };
    }

    fn packet() -> Packet {
        Packet {
            source_port: "transfer".into(),
            source_channel: "channel-0".into(),
            sequence: 7,
            ..Packet::default()
        }
    }

    /// The in-flight lookup is a `Has` on the PFM store from app version 2
    /// (gas), nothing at app version 1.
    #[test]
    fn in_flight_lookup_costs_a_has_from_app_version_2() -> Result<()> {
        let mut ms = v2_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let p = packet();
        at(&mut ctx, 1);
        let before = ctx.gas.consumed();
        assert!(get_and_clear_in_flight(&mut ctx, &p)?.is_none());
        assert_eq!(ctx.gas.consumed(), before);
        at(&mut ctx, 2);
        assert!(get_and_clear_in_flight(&mut ctx, &p)?.is_none());
        assert_eq!(ctx.gas.consumed(), before + 1000);
        Ok(())
    }

    /// A stored in-flight packet is returned once and removed.
    #[test]
    fn in_flight_packet_is_cleared() -> Result<()> {
        let mut ms = v2_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        at(&mut ctx, 2);
        let ifp = InFlightPacket {
            refund_channel_id: "channel-1".into(),
            refund_sequence: 3,
            ..InFlightPacket::default()
        };
        ctx.set(
            STORE_KEY,
            refund_packet_key("channel-0", "transfer", 7),
            ifp.encode_to_vec(),
        )?;
        assert_eq!(get_and_clear_in_flight(&mut ctx, &packet())?, Some(ifp));
        assert!(get_and_clear_in_flight(&mut ctx, &packet())?.is_none());
        Ok(())
    }

    /// The fee param decodes as the amino JSON string of a `Dec`.
    #[test]
    fn fee_percentage_decodes_the_param() -> Result<()> {
        let mut ms = v2_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let set = |ctx: &mut Context<'_>, v: &str| {
            x_params::set_raw(
                ctx,
                x_packetforward::MODULE_NAME,
                b"FeePercentage",
                v.as_bytes().to_vec(),
            )
        };
        set(&mut ctx, r#""0.000000000000000000""#)?;
        assert!(fee_percentage(&mut ctx)?.is_zero());
        set(&mut ctx, r#""0.100000000000000000""#)?;
        assert_eq!(
            fee_percentage(&mut ctx)?.to_string(),
            "0.100000000000000000"
        );
        Ok(())
    }

    /// v6 error acknowledgements carry the full error text.
    #[test]
    fn error_acknowledgements_carry_the_error_text() {
        assert_eq!(
            error_ack(
                "giving up on packet on channel (channel-1) port (transfer) after max retries"
            )
            .marshal(),
            r#"{"error":"packet-forward-middleware error: giving up on packet on channel (channel-1) port (transfer) after max retries"}"#
        );
    }
}
