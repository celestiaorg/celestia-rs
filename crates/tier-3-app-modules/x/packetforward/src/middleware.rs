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

use crate::memo::{self, ForwardMetadata, Memo};
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

fn is_panic(e: &SdkError) -> bool {
    e.codespace == "undefined" && e.code == 111_222
}

/// A plain Go `error` (`fmt.Errorf`).
fn plain(log: impl Into<String>) -> SdkError {
    x_ibc::errors::unregistered(log)
}

/// `newErrorAcknowledgement`.
fn error_ack(err: &str) -> Acknowledgement {
    Acknowledgement::error_string(&format!("packet-forward-middleware error: {err}"))
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
    let memo = match &md.next {
        Some(Ok(s)) => s.clone(),
        Some(Err(e)) => return Err(sdk::error::sdk::json_marshal(e.clone())),
        None => String::new(),
    };
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let now = x_ibc::lightclients::tendermint::block_time(ctx) as i64 as u64;
    #[allow(clippy::cast_sign_loss)]
    let msg = MsgTransfer {
        source_port: md.port.clone(),
        source_channel: md.channel.clone(),
        token: Some(token.to_proto()),
        sender: receiver.to_owned(),
        receiver: md.receiver.clone(),
        timeout_height: Some(Height::default()),
        timeout_timestamp: now.wrapping_add(timeout as u64),
        memo,
    };
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
    if !data.memo.is_empty() {
        md.next = Some(
            memo::unmarshal_next(&data.memo)
                .map_err(|e| plain(format!("error unmarshaling memo json: {e}")))?,
        );
    }
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
        let md = match memo::classify(&data.memo) {
            Memo::NotForward => return self.app.on_recv_packet(ctx, p, relayer),
            Memo::ParseError(e) => {
                return Ok(Some(error_ack(&format!(
                    "error parsing forward metadata: {e}"
                ))));
            }
            Memo::NilForward => {
                return Err(sdk::error::go_panic(
                    "runtime error: invalid memory address or nil pointer dereference",
                ));
            }
            Memo::Forward(m) => m,
        };
        if let Err(e) = memo::validate(&md) {
            return Ok(Some(error_ack(&e)));
        }
        let override_receiver = get_receiver(&p.destination_channel, &data.sender);
        if let Err(e) = self.receive_funds(ctx, p, &data, &override_receiver, relayer)? {
            return Ok(Some(error_ack(&format!("error receiving packet: {e}"))));
        }
        let denom = denom_for_this_chain(
            &p.destination_port,
            &p.destination_channel,
            &p.source_port,
            &p.source_channel,
            &data.denom,
        );
        let Some(amount) = parse_amount(&data.amount) else {
            return Ok(Some(error_ack(&format!(
                "error parsing amount for forward: {}",
                data.amount
            ))));
        };
        let token = relay::new_coin(&denom, amount)?;
        let timeout = if md.timeout <= 0 {
            self.forward_timeout
        } else {
            md.timeout
        };
        let retries = md.retries.unwrap_or(self.retries_on_timeout);
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
            return Ok(Some(error_ack(&e.log)));
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
            let ack = AckResponse::Error(format!("packet-forward-middleware error: {e}"));
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
        let json = format!(
            "{{\"denom\":\"utia\",\"amount\":\"1\",\"sender\":\"\u{10ffff}\",\"receiver\":\"pfm\",\"memo\":\"\"}}"
        );
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
}
