//! ICS-20 packet relay (`keeper/relay.go`, `keeper/msg_server.go`) and the
//! transfer `IBCModule` callbacks (`ibc_module.go`).

use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::applications::transfer::v1::{
    DenomTrace, MsgTransfer, MsgTransferResponse, MsgUpdateParams, MsgUpdateParamsResponse,
};
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet};
use celestia_proto::ibc::core::client::v1::Height;
use prost::Message;
use sdk::coins::{Amount, Coin, Coins};
use sdk::event::event;
use sdk::{Context, Result, SdkError};
use x_ibc::errors::{channel as cherr, ibc as ierr, port as porterr};
use x_ibc::router::{Acknowledgement, IbcModule};

use crate::{
    DENOM_TRACE_KEY, MODULE_NAME, SCOPED_KEEPER, VERSION, denom_metadata, denom_trace_hash,
    escrow_address, full_denom_path, get_denom_trace, get_params, get_port,
    get_total_escrow_for_denom, ibc_denom, set_denom_trace, set_params, set_total_escrow_for_denom,
};

/// `transfer` codespace errors (`types/errors.go`).
pub mod errors {
    use sdk::SdkError;

    fn e(code: u32, desc: &str, log: &str) -> SdkError {
        SdkError::new(
            "transfer",
            code,
            if log.is_empty() {
                desc.to_owned()
            } else {
                format!("{log}: {desc}")
            },
        )
    }
    /// `ErrInvalidPacketTimeout` (2).
    #[must_use]
    pub fn invalid_packet_timeout(log: &str) -> SdkError {
        e(2, "invalid packet timeout", log)
    }
    /// `ErrInvalidDenomForTransfer` (3).
    #[must_use]
    pub fn invalid_denom_for_transfer(log: &str) -> SdkError {
        e(3, "invalid denomination for cross-chain transfer", log)
    }
    /// `ErrInvalidVersion` (4).
    #[must_use]
    pub fn invalid_version(log: &str) -> SdkError {
        e(4, "invalid ICS20 version", log)
    }
    /// `ErrInvalidAmount` (5).
    #[must_use]
    pub fn invalid_amount(log: &str) -> SdkError {
        e(5, "invalid token amount", log)
    }
    /// `ErrTraceNotFound` (6).
    #[must_use]
    pub fn trace_not_found(log: &str) -> SdkError {
        e(6, "denomination trace not found", log)
    }
    /// `ErrSendDisabled` (7).
    #[must_use]
    pub fn send_disabled(log: &str) -> SdkError {
        e(
            7,
            "fungible token transfers from this chain are disabled",
            log,
        )
    }
    /// `ErrReceiveDisabled` (8).
    #[must_use]
    pub fn receive_disabled(log: &str) -> SdkError {
        e(
            8,
            "fungible token transfers to this chain are disabled",
            log,
        )
    }
    /// `ErrMaxTransferChannels` (9).
    #[must_use]
    pub fn max_transfer_channels(log: &str) -> SdkError {
        e(9, "max transfer channels", log)
    }
    /// `ErrInvalidMemo` (11).
    #[must_use]
    pub fn invalid_memo(log: &str) -> SdkError {
        e(11, "invalid memo", log)
    }
}

/// `MaximumReceiverLength`.
pub const MAXIMUM_RECEIVER_LENGTH: usize = 2048;
/// `MaximumMemoLength`.
pub const MAXIMUM_MEMO_LENGTH: usize = 32768;
/// Prefix of IBC denoms.
pub const DENOM_PREFIX: &str = "ibc";

// ---------------------------------------------------------------------------
// Packet data
// ---------------------------------------------------------------------------

/// `FungibleTokenPacketData` (`ibc.applications.transfer.v2`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacketData {
    /// Denom (full trace path).
    pub denom: String,
    /// Amount (decimal string).
    pub amount: String,
    /// Sender.
    pub sender: String,
    /// Receiver.
    pub receiver: String,
    /// Memo.
    pub memo: String,
}

impl PacketData {
    /// `ModuleCdc.UnmarshalJSON(bz, &data)` (jsonpb).
    pub fn decode(bz: &[u8]) -> core::result::Result<Self, String> {
        let f = x_ibc::gojson::decode_string_fields(
            bz,
            &["denom", "amount", "sender", "receiver", "memo"],
        )?;
        let mut it = f.into_iter();
        let mut next = || it.next().unwrap_or_default();
        Ok(Self {
            denom: next(),
            amount: next(),
            sender: next(),
            receiver: next(),
            memo: next(),
        })
    }

    /// `GetBytes`: sorted proto3 JSON without defaults
    /// (`mustProtoMarshalJSON` sets `EmitDefaults: false`).
    #[must_use]
    pub fn get_bytes(&self) -> Vec<u8> {
        use x_ibc::gojson::escape_string as q;
        let parts: Vec<String> = [
            ("amount", &self.amount),
            ("denom", &self.denom),
            ("memo", &self.memo),
            ("receiver", &self.receiver),
            ("sender", &self.sender),
        ]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("\"{k}\":{}", q(v)))
        .collect();
        format!("{{{}}}", parts.join(",")).into_bytes()
    }

    /// `ModuleCdc.MustMarshalJSON(&data)`: field order, defaults emitted.
    #[must_use]
    pub fn marshal_unsorted(&self) -> Vec<u8> {
        use x_ibc::gojson::escape_string as q;
        format!(
            "{{\"denom\":{},\"amount\":{},\"sender\":{},\"receiver\":{},\"memo\":{}}}",
            q(&self.denom),
            q(&self.amount),
            q(&self.sender),
            q(&self.receiver),
            q(&self.memo)
        )
        .into_bytes()
    }

    /// `FungibleTokenPacketData.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        let Ok(amount) = Amount::parse(&self.amount) else {
            return Err(errors::invalid_amount(&format!(
                "unable to parse transfer amount ({}) into math.Int",
                self.amount
            )));
        };
        if !amount.is_positive() {
            return Err(errors::invalid_amount(&format!(
                "amount must be strictly positive: got {amount}"
            )));
        }
        if self.sender.trim().is_empty() {
            return Err(ierr::invalid_address("sender address cannot be blank"));
        }
        if self.receiver.trim().is_empty() {
            return Err(ierr::invalid_address("receiver address cannot be blank"));
        }
        validate_prefixed_denom(&self.denom)
    }
}

// ---------------------------------------------------------------------------
// Denom traces (`types/trace.go`, `types/coin.go`)
// ---------------------------------------------------------------------------

/// `extractPathAndBaseFromFullDenom`.
fn extract_path_and_base(items: &[&str]) -> (String, String) {
    let mut path = Vec::new();
    let mut base: &[&str] = &[];
    let len = items.len();
    let mut i = 0;
    while i < len {
        if i < len - 1 && len > 2 && x_ibc::channel::is_valid_channel_id(items[i + 1]) {
            path.push(items[i]);
            path.push(items[i + 1]);
        } else {
            base = &items[i..];
            break;
        }
        i += 2;
    }
    (path.join("/"), base.join("/"))
}

/// `ParseDenomTrace`.
#[must_use]
pub fn parse_denom_trace(raw: &str) -> DenomTrace {
    let split: Vec<&str> = raw.split('/').collect();
    if split[0] == raw {
        return DenomTrace {
            path: String::new(),
            base_denom: raw.to_owned(),
        };
    }
    let (path, base_denom) = extract_path_and_base(&split);
    DenomTrace { path, base_denom }
}

fn validate_trace_identifiers(ids: &[&str]) -> Result<()> {
    if ids.is_empty() || ids.len() % 2 != 0 {
        return Err(x_ibc::errors::unregistered(format!(
            "trace info must come in pairs of port and channel identifiers '{{portID}}/{{channelID}}', got the identifiers: {ids:?}"
        )));
    }
    for i in (0..ids.len()).step_by(2) {
        x_ibc::host::port_identifier_validator(ids[i])
            .map_err(|e| e.wrap(format!("invalid port ID at position {i}")))?;
        x_ibc::host::channel_identifier_validator(ids[i + 1])
            .map_err(|e| e.wrap(format!("invalid channel ID at position {i}")))?;
    }
    Ok(())
}

/// `ValidatePrefixedDenom`.
pub fn validate_prefixed_denom(denom: &str) -> Result<()> {
    let split: Vec<&str> = denom.split('/').collect();
    if split[0] == denom && !denom.trim().is_empty() {
        return Ok(());
    }
    if split.last().is_none_or(|s| s.trim().is_empty()) {
        return Err(errors::invalid_denom_for_transfer(
            "base denomination cannot be blank",
        ));
    }
    let (path, _) = extract_path_and_base(&split);
    if path.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = path.split('/').collect();
    validate_trace_identifiers(&ids)
}

/// `ParseHexHash`: hex, empty or 32 bytes (`ValidateHash`).
fn parse_hex_hash(s: &str) -> core::result::Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("encoding/hex: odd length hex string".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by(2) {
        let hv = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        };
        let (Some(h), Some(l)) = (hv(b[i]), hv(b[i + 1])) else {
            let bad = if hv(b[i]).is_none() { b[i] } else { b[i + 1] };
            return Err(format!(
                "encoding/hex: invalid byte: U+{:04X} {:?}",
                bad,
                char::from(bad)
            ));
        };
        out.push((h << 4) | l);
    }
    if !out.is_empty() && out.len() != 32 {
        return Err(format!(
            "expected size to be 32 bytes, got {} bytes",
            out.len()
        ));
    }
    Ok(out)
}

/// `ValidateIBCDenom`.
pub fn validate_ibc_denom(denom: &str) -> Result<()> {
    if !sdk::coins::is_valid_denom(denom) {
        return Err(x_ibc::errors::unregistered(format!(
            "invalid denom: {denom}"
        )));
    }
    let split: Vec<&str> = denom.splitn(2, '/').collect();
    if denom == DENOM_PREFIX {
        return Err(errors::invalid_denom_for_transfer(&format!(
            "denomination should be prefixed with the format 'ibc/{{hash(trace + \"/\" + {denom})}}'"
        )));
    }
    if split.len() == 2 && split[0] == DENOM_PREFIX {
        if split[1].trim().is_empty() {
            return Err(errors::invalid_denom_for_transfer(&format!(
                "denomination should be prefixed with the format 'ibc/{{hash(trace + \"/\" + {denom})}}'"
            )));
        }
        parse_hex_hash(split[1]).map_err(|e| {
            x_ibc::errors::unregistered(format!("invalid denom trace hash {}: {e}", split[1]))
        })?;
    }
    Ok(())
}

/// `ReceiverChainIsSource`.
#[must_use]
pub fn receiver_chain_is_source(port: &str, chan: &str, denom: &str) -> bool {
    denom.starts_with(&format!("{port}/{chan}/"))
}

/// `SenderChainIsSource`.
#[must_use]
pub fn sender_chain_is_source(port: &str, chan: &str, denom: &str) -> bool {
    !receiver_chain_is_source(port, chan, denom)
}

/// `sdk.NewCoin`: Go panics on an invalid denom or negative amount.
pub fn new_coin(denom: &str, amount: Amount) -> Result<Coin> {
    let c = Coin::with_amount(denom, amount);
    if !c.is_valid() {
        return Err(sdk::error::go_panic(format!("invalid coin {c}")));
    }
    Ok(c)
}

/// `sdk.NewCoins(c)`: zero coins are dropped.
#[must_use]
pub fn coins(c: &Coin) -> Coins {
    if c.is_zero() {
        Coins::new()
    } else {
        Coins(vec![c.clone()])
    }
}

/// `DenomPathFromHash`.
pub fn denom_path_from_hash(ctx: &mut Context<'_>, denom: &str) -> Result<String> {
    let hex = &denom[DENOM_PREFIX.len() + 1..];
    let hash = parse_hex_hash(hex).map_err(|e| errors::invalid_denom_for_transfer(&e))?;
    let trace = get_denom_trace(ctx, &hash)?.ok_or_else(|| errors::trace_not_found(hex))?;
    Ok(full_denom_path(&trace))
}

/// `HasDenomTrace`.
fn has_denom_trace(ctx: &mut Context<'_>, hash: &[u8]) -> Result<bool> {
    ctx.has(crate::STORE_KEY, &[DENOM_TRACE_KEY, hash].concat())
}

/// bank `HasDenomMetaData`.
fn has_denom_metadata(ctx: &mut Context<'_>, denom: &str) -> Result<bool> {
    ctx.has(
        x_bank::STORE_KEY,
        &sdk::codec::key(&[&[0x01], denom.as_bytes()]),
    )
}

// ---------------------------------------------------------------------------
// Escrow
// ---------------------------------------------------------------------------

/// `currentTotalEscrow.Add(token)` + `SetTotalEscrowForDenom`.
pub fn add_escrow(ctx: &mut Context<'_>, token: &Coin) -> Result<()> {
    let cur = get_total_escrow_for_denom(ctx, &token.denom)?;
    let cur = Amount::parse(&cur.amount).unwrap_or_else(|_| Amount::zero());
    let new = cur
        .checked_add(&token.amount)
        .map_err(sdk::coins::math_err)?;
    set_total_escrow_for_denom(ctx, &Coin::with_amount(token.denom.clone(), new).to_proto())
}

/// `currentTotalEscrow.Sub(token)` + `SetTotalEscrowForDenom` (Go panics
/// when the result is negative).
pub fn sub_escrow(ctx: &mut Context<'_>, token: &Coin) -> Result<()> {
    let cur = get_total_escrow_for_denom(ctx, &token.denom)?;
    let cur = Amount::parse(&cur.amount).unwrap_or_else(|_| Amount::zero());
    let new = cur
        .checked_sub(&token.amount)
        .map_err(sdk::coins::math_err)?;
    if new.is_negative() {
        return Err(sdk::error::go_panic(format!(
            "negative coin amount: {new}{}",
            token.denom
        )));
    }
    set_total_escrow_for_denom(ctx, &Coin::with_amount(token.denom.clone(), new).to_proto())
}

/// `escrowToken`.
fn escrow_token(ctx: &mut Context<'_>, sender: &[u8], escrow: &[u8], token: &Coin) -> Result<()> {
    x_bank::send_coins(ctx, sender, escrow, &coins(token))?;
    add_escrow(ctx, token)
}

/// `unescrowToken`.
fn unescrow_token(
    ctx: &mut Context<'_>,
    escrow: &[u8],
    receiver: &[u8],
    token: &Coin,
) -> Result<()> {
    x_bank::send_coins(ctx, escrow, receiver, &coins(token)).map_err(|e| {
        e.wrap("unable to unescrow tokens, this may be caused by a malicious counterparty module or a bug: please open an issue on counterparty module")
    })?;
    sub_escrow(ctx, token)
}

// ---------------------------------------------------------------------------
// MsgTransfer
// ---------------------------------------------------------------------------

/// `UnboundedSpendLimit`: `2^256 - 1`.
fn is_unbounded(a: &Amount) -> bool {
    a.to_string()
        == "115792089237316195423570985008687907853269984665640564039457584007913129639935"
}

/// `sendTransfer`: returns the packet sequence.
#[allow(clippy::too_many_arguments)]
pub fn send_transfer(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    token: &Coin,
    sender: &[u8],
    receiver: &str,
    timeout_height: &Height,
    timeout_timestamp: u64,
    memo: &str,
) -> Result<u64> {
    if x_ibc::channel::get_channel(ctx, port, chan)?.is_none() {
        return Err(cherr::channel_not_found(format!(
            "port ID ({port}) channel ID ({chan})"
        )));
    }
    let cap = SCOPED_KEEPER
        .get_capability(ctx, &x_ibc::host::channel_capability_path(port, chan))?
        .ok_or_else(|| {
            cherr::channel_capability_not_found("module does not own channel capability")
        })?;
    let mut full_denom_path = token.denom.clone();
    if token.denom.starts_with("ibc/") {
        full_denom_path = denom_path_from_hash(ctx, &token.denom)?;
    }
    let mut token = token.clone();
    if is_unbounded(&token.amount) {
        token.amount = x_bank::get_balance(ctx, sender, &token.denom)?.amount;
    }
    if sender_chain_is_source(port, chan, &full_denom_path) {
        escrow_token(ctx, sender, &escrow_address(port, chan), &token)?;
    } else {
        x_bank::send_from_account_to_module(ctx, sender, MODULE_NAME, &coins(&token))?;
        x_bank::burn_coins(ctx, MODULE_NAME, &coins(&token)).map_err(|e| {
            sdk::error::go_panic(format!(
                "cannot burn coins after a successful send to a module account: {}",
                e.log
            ))
        })?;
    }
    let data = PacketData {
        denom: full_denom_path,
        amount: token.amount.to_string(),
        sender: x_auth::bech32(sender)?,
        receiver: receiver.to_owned(),
        memo: memo.to_owned(),
    };
    x_ibc::channel::send_packet(
        ctx,
        &cap,
        port,
        chan,
        timeout_height,
        timeout_timestamp,
        &data.get_bytes(),
    )
}

/// `sdk.AccAddressFromBech32` (errors are unregistered Go errors, except an
/// empty decoded address).
pub fn acc_address(s: &str) -> Result<Vec<u8>> {
    if s.trim().is_empty() {
        return Err(x_ibc::errors::unregistered(
            "empty address string is not allowed",
        ));
    }
    // `sdk.AccAddressFromBech32`: `GetFromBech32` (plain errors with Go's
    // bech32 messages) then `VerifyAddressFormat`.
    let (hrp, bz) = encoding::bech32::decode_and_convert(s).map_err(x_ibc::errors::unregistered)?;
    if hrp != encoding::bech32::ACCOUNT_PREFIX {
        return Err(x_ibc::errors::unregistered(format!(
            "invalid Bech32 prefix; expected {}, got {hrp}",
            encoding::bech32::ACCOUNT_PREFIX
        )));
    }
    if bz.is_empty() {
        return Err(sdk::error::sdk::unknown_address(
            "addresses cannot be empty",
        ));
    }
    if bz.len() > 255 {
        return Err(sdk::error::sdk::unknown_address(format!(
            "address max length is 255, got {}",
            bz.len()
        )));
    }
    Ok(bz)
}

/// Keeper `Transfer` (the msg server, also called by PFM without
/// `ValidateBasic`).
pub fn transfer(ctx: &mut Context<'_>, msg: &MsgTransfer) -> Result<MsgTransferResponse> {
    if !get_params(ctx)?.send_enabled {
        return Err(errors::send_disabled(""));
    }
    let sender = acc_address(&msg.sender)?;
    let token_pb = msg.token.clone().unwrap_or_default();
    let token = Coin::from_proto(&token_pb)?;
    if x_bank::msgs::is_send_enabled_coins(ctx, &Coins(vec![token.clone()])).is_err() {
        return Err(errors::send_disabled(&format!(
            "{} transfers are currently disabled",
            token.denom
        )));
    }
    if x_bank::is_blocked(&sender) {
        return Err(ierr::unauthorized(format!(
            "{} is not allowed to send funds",
            msg.sender
        )));
    }
    let seq = send_transfer(
        ctx,
        &msg.source_port,
        &msg.source_channel,
        &token,
        &sender,
        &msg.receiver,
        &msg.timeout_height.unwrap_or_default(),
        msg.timeout_timestamp,
        &msg.memo,
    )?;
    ctx.events.emit_all([
        event(
            "ibc_transfer",
            [
                ("sender", msg.sender.clone()),
                ("receiver", msg.receiver.clone()),
                ("amount", token.amount.to_string()),
                ("denom", token.denom.clone()),
                ("memo", msg.memo.clone()),
            ],
        ),
        event("message", [("module", MODULE_NAME)]),
    ]);
    Ok(MsgTransferResponse { sequence: seq })
}

/// `MsgTransfer.ValidateBasic`.
pub fn validate_msg_transfer(msg: &MsgTransfer) -> Result<()> {
    x_ibc::host::port_identifier_validator(&msg.source_port)
        .map_err(|e| e.wrap("invalid source port ID"))?;
    x_ibc::host::channel_identifier_validator(&msg.source_channel)
        .map_err(|e| e.wrap("invalid source channel ID"))?;
    let t = msg.token.clone().unwrap_or_default();
    let token = Coin::from_proto(&t)
        .map_err(|_| ierr::invalid_coins(format!("{}{}", t.amount, t.denom)))?;
    if !token.is_valid() {
        return Err(ierr::invalid_coins(token.to_string()));
    }
    if !token.amount.is_positive() {
        return Err(ierr::insufficient_funds(token.to_string()));
    }
    x_auth::parse_bech32(&msg.sender).map_err(|e| {
        ierr::invalid_address(format!("string could not be parsed as address: {}", e.log))
    })?;
    if msg.receiver.trim().is_empty() {
        return Err(ierr::invalid_address("missing recipient address"));
    }
    if msg.receiver.len() > MAXIMUM_RECEIVER_LENGTH {
        return Err(ierr::invalid_address(format!(
            "recipient address must not exceed {MAXIMUM_RECEIVER_LENGTH} bytes"
        )));
    }
    if msg.memo.len() > MAXIMUM_MEMO_LENGTH {
        return Err(errors::invalid_memo(&format!(
            "memo must not exceed {MAXIMUM_MEMO_LENGTH} bytes"
        )));
    }
    validate_ibc_denom(&token.denom)
}

/// `MsgUpdateParams` (authority: gov).
pub fn update_params(
    ctx: &mut Context<'_>,
    msg: &MsgUpdateParams,
) -> Result<MsgUpdateParamsResponse> {
    let auth = x_ibc::msgs::authority();
    if auth != msg.signer {
        return Err(ierr::unauthorized(format!(
            "expected {auth}, got {}",
            msg.signer
        )));
    }
    set_params(ctx, &msg.params.unwrap_or_default())?;
    Ok(MsgUpdateParamsResponse {})
}

/// `MsgUpdateParams.ValidateBasic`.
pub fn validate_msg_update_params(msg: &MsgUpdateParams) -> Result<()> {
    x_auth::parse_bech32(&msg.signer).map(|_| ()).map_err(|e| {
        ierr::invalid_address(format!("string could not be parsed as address: {}", e.log))
    })
}

// ---------------------------------------------------------------------------
// Receive, acknowledgement, timeout (keeper)
// ---------------------------------------------------------------------------

/// Keeper `OnRecvPacket`.
pub fn on_recv_packet(ctx: &mut Context<'_>, p: &Packet, data: &PacketData) -> Result<()> {
    data.validate_basic()
        .map_err(|e| e.wrap("error validating ICS-20 transfer packet data"))?;
    if !get_params(ctx)?.receive_enabled {
        return Err(errors::receive_disabled(""));
    }
    let receiver = acc_address(&data.receiver).map_err(|e| {
        e.wrap(format!(
            "failed to decode receiver address: {}",
            data.receiver
        ))
    })?;
    let amount = Amount::parse(&data.amount).map_err(|_| {
        errors::invalid_amount(&format!("unable to parse transfer amount: {}", data.amount))
    })?;
    if receiver_chain_is_source(&p.source_port, &p.source_channel, &data.denom) {
        let prefix = format!("{}/{}/", p.source_port, p.source_channel);
        let unprefixed = &data.denom[prefix.len()..];
        let trace = parse_denom_trace(unprefixed);
        let denom = if trace.path.is_empty() {
            unprefixed.to_owned()
        } else {
            ibc_denom(&trace)
        };
        let token = new_coin(&denom, amount)?;
        if x_bank::is_blocked(&receiver) {
            return Err(ierr::unauthorized(format!(
                "{} is not allowed to receive funds",
                x_auth::bech32(&receiver)?
            )));
        }
        let escrow = escrow_address(&p.destination_port, &p.destination_channel);
        return unescrow_token(ctx, &escrow, &receiver, &token);
    }
    let prefixed = format!(
        "{}/{}/{}",
        p.destination_port, p.destination_channel, data.denom
    );
    let trace = parse_denom_trace(&prefixed);
    let hash = denom_trace_hash(&trace);
    if !has_denom_trace(ctx, &hash)? {
        set_denom_trace(ctx, &trace)?;
    }
    let voucher_denom = ibc_denom(&trace);
    if !has_denom_metadata(ctx, &voucher_denom)? {
        x_bank::set_denom_metadata(ctx, &denom_metadata(&trace))?;
    }
    ctx.events.emit(event(
        "denomination_trace",
        [
            ("trace_hash", upper_hex(&hash)),
            ("denom", voucher_denom.clone()),
        ],
    ));
    let voucher = new_coin(&voucher_denom, amount)?;
    x_bank::mint_coins(ctx, MODULE_NAME, &coins(&voucher))
        .map_err(|e| e.wrap("failed to mint IBC tokens"))?;
    x_bank::send_from_module_to_account(ctx, MODULE_NAME, &receiver, &coins(&voucher)).map_err(
        |e| {
            e.wrap(format!(
                "failed to send coins to receiver {}",
                x_auth::bech32(&receiver).unwrap_or_default()
            ))
        },
    )
}

fn upper_hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02X}");
            s
        })
}

/// `refundPacketToken`.
pub fn refund_packet_token(ctx: &mut Context<'_>, p: &Packet, data: &PacketData) -> Result<()> {
    let trace = parse_denom_trace(&data.denom);
    let amount = Amount::parse(&data.amount).map_err(|_| {
        errors::invalid_amount(&format!(
            "unable to parse transfer amount ({}) into math.Int",
            data.amount
        ))
    })?;
    let token = new_coin(&ibc_denom(&trace), amount)?;
    let sender = acc_address(&data.sender)?;
    if sender_chain_is_source(&p.source_port, &p.source_channel, &data.denom) {
        let escrow = escrow_address(&p.source_port, &p.source_channel);
        return unescrow_token(ctx, &escrow, &sender, &token);
    }
    x_bank::mint_coins(ctx, MODULE_NAME, &coins(&token))?;
    x_bank::send_from_module_to_account(ctx, MODULE_NAME, &sender, &coins(&token)).map_err(|e| {
        sdk::error::go_panic(format!(
            "unable to send coins from module to account despite previously minting coins to module account: {}",
            e.log
        ))
    })
}

// ---------------------------------------------------------------------------
// IBCModule
// ---------------------------------------------------------------------------

/// The transfer application (`transfer.IBCModule`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TransferModule;

/// `ValidateTransferChannelParams`.
fn validate_channel_params(
    ctx: &mut Context<'_>,
    order: Order,
    port: &str,
    chan: &str,
) -> Result<()> {
    let seq = chan
        .strip_prefix(x_ibc::channel::CHANNEL_PREFIX)
        .filter(|_| x_ibc::channel::is_valid_channel_id(chan))
        .and_then(x_ibc::height::parse_u64)
        .ok_or_else(|| {
            x_ibc::errors::host::invalid_id(
                "channel identifier is not in the format: `channel-{N}`",
            )
        })?;
    if seq > u64::from(u32::MAX) {
        return Err(errors::max_transfer_channels(&format!(
            "channel sequence {seq} is greater than max allowed transfer channels {}",
            u32::MAX
        )));
    }
    if order != Order::Unordered {
        return Err(cherr::invalid_channel_ordering(format!(
            "expected ORDER_UNORDERED channel, got {} ",
            x_ibc::channel::order_name(order as i32)
        )));
    }
    let bound = get_port(ctx)?;
    if bound != port {
        return Err(porterr::invalid_port(format!(
            "invalid port: {port}, expected {bound}"
        )));
    }
    Ok(())
}

fn packet_event(attrs: Vec<(&str, String)>) -> celestia_proto::celestia_core_proto::abci::Event {
    event("fungible_token_packet", attrs)
}

impl IbcModule for TransferModule {
    fn on_chan_open_init(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        _hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        _cp: &Counterparty,
        version: &str,
    ) -> Result<String> {
        validate_channel_params(ctx, order, port_id, channel_id)?;
        let version = if version.trim().is_empty() {
            VERSION
        } else {
            version
        };
        if version != VERSION {
            return Err(errors::invalid_version(&format!(
                "expected {VERSION}, got {version}"
            )));
        }
        SCOPED_KEEPER.claim_capability(
            ctx,
            cap,
            &x_ibc::host::channel_capability_path(port_id, channel_id),
        )?;
        Ok(version.to_owned())
    }

    fn on_chan_open_try(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        _hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        _cp: &Counterparty,
        _cp_version: &str,
    ) -> Result<String> {
        validate_channel_params(ctx, order, port_id, channel_id)?;
        SCOPED_KEEPER.claim_capability(
            ctx,
            cap,
            &x_ibc::host::channel_capability_path(port_id, channel_id),
        )?;
        Ok(VERSION.to_owned())
    }

    fn on_chan_open_ack(
        &self,
        _ctx: &mut Context<'_>,
        _port_id: &str,
        _channel_id: &str,
        _cp_channel: &str,
        cp_version: &str,
    ) -> Result<()> {
        if cp_version != VERSION {
            return Err(errors::invalid_version(&format!(
                "invalid counterparty version: expected {VERSION}, got {cp_version}"
            )));
        }
        Ok(())
    }

    fn on_chan_open_confirm(&self, _ctx: &mut Context<'_>, _p: &str, _c: &str) -> Result<()> {
        Ok(())
    }

    fn on_chan_close_init(&self, _ctx: &mut Context<'_>, _p: &str, _c: &str) -> Result<()> {
        Err(ierr::invalid_request("user cannot close channel"))
    }

    fn on_chan_close_confirm(&self, _ctx: &mut Context<'_>, _p: &str, _c: &str) -> Result<()> {
        Ok(())
    }

    fn on_recv_packet(
        &self,
        ctx: &mut Context<'_>,
        p: &Packet,
        _relayer: &[u8],
    ) -> Result<Option<Acknowledgement>> {
        let mut ack = Acknowledgement::result(&[1]);
        let mut ack_err: Option<SdkError> = None;
        let data = PacketData::decode(&p.data).unwrap_or_else(|_| {
            let e = ierr::invalid_type("cannot unmarshal ICS-20 transfer packet data");
            ack = Acknowledgement::error(&e);
            ack_err = Some(e);
            PacketData::default()
        });
        if ack.success {
            if let Err(e) = on_recv_packet(ctx, p, &data) {
                // Go panics propagate (the tx fails) instead of becoming an ack.
                if e.codespace == "undefined" && e.code == 111_222 {
                    return Err(e);
                }
                ack = Acknowledgement::error(&e);
                ack_err = Some(e);
            }
        }
        let mut attrs = vec![
            ("module", MODULE_NAME.to_owned()),
            ("sender", data.sender.clone()),
            ("receiver", data.receiver.clone()),
            ("denom", data.denom.clone()),
            ("amount", data.amount.clone()),
            ("memo", data.memo.clone()),
            ("success", ack.success.to_string()),
        ];
        if let Some(e) = &ack_err {
            attrs.push(("error", e.log.clone()));
        }
        ctx.events.emit(packet_event(attrs));
        Ok(Some(ack))
    }

    fn on_acknowledgement_packet(
        &self,
        ctx: &mut Context<'_>,
        p: &Packet,
        acknowledgement: &[u8],
        _relayer: &[u8],
    ) -> Result<()> {
        let ack = x_ibc::channel::decode_acknowledgement(acknowledgement).map_err(|e| {
            ierr::unknown_request(format!(
                "cannot unmarshal ICS-20 transfer packet acknowledgement: {e}"
            ))
        })?;
        let data = PacketData::decode(&p.data).map_err(|e| {
            ierr::unknown_request(format!("cannot unmarshal ICS-20 transfer packet data: {e}"))
        })?;
        // `ModuleCdc.MustMarshalJSON(&ack)` must reproduce the bytes.
        if ack.marshal().as_bytes() != acknowledgement {
            return Err(ierr::invalid_type(
                "acknowledgement did not marshal to expected bytes",
            ));
        }
        if let x_ibc::channel::AckResponse::Error(_) = ack {
            refund_packet_token(ctx, p, &data)?;
        }
        ctx.events.emit(packet_event(vec![
            ("module", MODULE_NAME.to_owned()),
            ("sender", data.sender.clone()),
            ("receiver", data.receiver.clone()),
            ("denom", data.denom.clone()),
            ("amount", data.amount.clone()),
            ("memo", data.memo.clone()),
            ("acknowledgement", ack_text(&ack)),
        ]));
        match ack {
            x_ibc::channel::AckResponse::Result(r) => ctx.events.emit(packet_event(vec![(
                "success",
                String::from_utf8_lossy(&r).into_owned(),
            )])),
            x_ibc::channel::AckResponse::Error(e) => {
                ctx.events.emit(packet_event(vec![("error", e)]));
            }
            x_ibc::channel::AckResponse::Empty => {}
        }
        Ok(())
    }

    fn on_timeout_packet(&self, ctx: &mut Context<'_>, p: &Packet, _relayer: &[u8]) -> Result<()> {
        let data = PacketData::decode(&p.data).map_err(|e| {
            ierr::unknown_request(format!("cannot unmarshal ICS-20 transfer packet data: {e}"))
        })?;
        refund_packet_token(ctx, p, &data)?;
        ctx.events.emit(event(
            "timeout",
            [
                ("module", MODULE_NAME.to_owned()),
                ("refund_receiver", data.sender.clone()),
                ("refund_denom", data.denom.clone()),
                ("refund_amount", data.amount.clone()),
                ("memo", data.memo.clone()),
            ],
        ));
        Ok(())
    }

    fn upgradable(&self) -> bool {
        true
    }
}

/// `Acknowledgement.String()` (gogoproto compact text format).
fn ack_text(ack: &x_ibc::channel::AckResponse) -> String {
    match ack {
        x_ibc::channel::AckResponse::Empty => String::new(),
        x_ibc::channel::AckResponse::Result(r) => format!("result:{} ", proto_text_bytes(r)),
        x_ibc::channel::AckResponse::Error(e) => {
            format!("error:{} ", proto_text_bytes(e.as_bytes()))
        }
    }
}

/// Protobuf text-format quoting of a bytes/string field.
fn proto_text_bytes(b: &[u8]) -> String {
    let mut out = String::from("\"");
    for &c in b {
        match c {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(c)),
            _ => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\{c:03o}"));
            }
        }
    }
    out.push('"');
    out
}

/// Encodes a packed `Any` response (for the router).
#[must_use]
pub fn encode<M: Message>(m: &M) -> Vec<u8> {
    m.encode_to_vec()
}
