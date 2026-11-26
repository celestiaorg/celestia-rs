//! The ibc-go v6 (celestiaorg `v6.3.0`) interchain accounts host of app
//! versions 2 and 3 (`27-interchain-accounts/host`): the channel
//! handshake callbacks, which create the interchain account, and
//! `OnRecvPacket`, which executes the controller's messages as that
//! account through the v0.46 msg router.
//!
//! The store layout, the metadata JSON and the error codes are the ones
//! of [`x_icahost`] (unchanged since v6). The rest differs from the v8
//! host and is ported here: the params live in the `icahost` `x/params`
//! subspace, the metadata allows only the `proto3` encoding, an active
//! channel may be reopened only while it is not `OPEN` and with the same
//! metadata, and the account address hashes the block header's app hash
//! and data hash. A message whose signers, `ValidateBasic` or handler is
//! not ported aborts the block ([`super::not_ported`]), as does a
//! `CosmosTx` that does not decode or holds an empty type URL.

use celestia_proto::Any;
use celestia_proto::capability::v1::Capability;
use celestia_proto::cosmos::auth::v1beta1::BaseAccount;
use celestia_proto::cosmos::base::abci::v1beta1::TxMsgData;
use celestia_proto::ibc::applications::interchain_accounts::v1::{
    CosmosTx, InterchainAccount, Metadata,
};
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet, State};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result, SdkError};
use x_ibc::channel::{self, AckResponse};
use x_ibc::errors::{channel as cherr, connection as conerr};
use x_icahost::host::{self as ica, errors};
use x_icahost::{HOST_PORT_ID, SCOPED_KEEPER, VERSION};

/// The module a host port or channel capability routes to.
pub(super) const MODULE: &str = x_icahost::SUB_MODULE_NAME;

std::thread_local! {
    /// The data hash of the block header being executed (Go's
    /// `ctx.BlockHeader().DataHash`, which the account address hashes).
    static DATA_HASH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets the data hash of the block header being executed.
pub(super) fn set_data_hash(h: &[u8]) {
    DATA_HASH.with(|d| h.clone_into(&mut d.borrow_mut()));
}

fn not_ported(what: &str) -> SdkError {
    super::not_ported(errs::not_supported(format!(
        "the ibc-go v6 interchain accounts host: {what}"
    )))
}

/// A param of the `icahost` subspace (`Subspace.Get`: Go panics when it
/// is unset or does not decode).
fn param(ctx: &mut Context<'_>, key: &str) -> Result<serde_json::Value> {
    let raw = x_params::get_raw(ctx, MODULE, key.as_bytes())?
        .ok_or_else(|| sdk::error::go_panic(format!("UnmarshalJSON: icahost {key} is not set")))?;
    serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace icahost: {key}: {e}")))
}

/// `IsHostEnabled`.
fn host_enabled(ctx: &mut Context<'_>) -> Result<bool> {
    let v = param(ctx, "HostEnabled")?;
    v.as_bool()
        .ok_or_else(|| sdk::error::go_panic(format!("subspace icahost: HostEnabled: {v}")))
}

/// `GetAllowMessages`.
fn allow_messages(ctx: &mut Context<'_>) -> Result<Vec<String>> {
    let v = param(ctx, "AllowMessages")?;
    if v.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(v)
        .map_err(|e| sdk::error::go_panic(format!("subspace icahost: AllowMessages: {e}")))
}

fn flow(log: &str) -> SdkError {
    errors::invalid_channel_flow(log)
}

/// `OnChanOpenInit`: the host never initiates.
pub(super) fn on_chan_open_init() -> SdkError {
    flow("channel handshake must be initiated by controller chain")
}

/// `OnChanOpenAck`: the host never initiates.
pub(super) fn on_chan_open_ack() -> SdkError {
    flow("channel handshake must be initiated by controller chain")
}

/// v6 `ValidateAccountAddress`.
fn validate_account_address(addr: &str) -> Result<()> {
    if addr.is_empty() || !addr.bytes().all(|b| b.is_ascii_alphanumeric()) || addr.len() > 128 {
        return Err(errors::invalid_account_address(
            "address must contain strictly alphanumeric characters, not exceeding 128 characters in length",
        ));
    }
    Ok(())
}

/// v6 `ValidateHostMetadata`: only the `proto3` encoding.
fn validate_host_metadata(ctx: &mut Context<'_>, hop: &str, m: &Metadata) -> Result<()> {
    if m.encoding != ica::ENCODING_PROTOBUF {
        return Err(errors::invalid_codec(&format!(
            "unsupported encoding format {}",
            m.encoding
        )));
    }
    if m.tx_type != ica::TX_TYPE_SDK_MULTI_MSG {
        return Err(errors::unknown_data_type(&format!(
            "unsupported transaction type {}",
            m.tx_type
        )));
    }
    let conn = x_ibc::connection::get_connection(ctx, hop)?
        .ok_or_else(|| conerr::connection_not_found(hop))?;
    let cp_conn = conn.counterparty.unwrap_or_default().connection_id;
    if m.controller_connection_id != cp_conn {
        return Err(conerr::invalid_connection(format!(
            "expected {cp_conn}, got {}",
            m.controller_connection_id
        )));
    }
    if m.host_connection_id != hop {
        return Err(conerr::invalid_connection(format!(
            "expected {hop}, got {}",
            m.host_connection_id
        )));
    }
    if !m.address.is_empty() {
        validate_account_address(&m.address)?;
    }
    if m.version != VERSION {
        return Err(errors::invalid_version(&format!(
            "expected {VERSION}, got {}",
            m.version
        )));
    }
    Ok(())
}

/// `IsPreviousMetadataEqual`.
fn is_previous_metadata_equal(previous: &str, m: &Metadata) -> bool {
    ica::metadata_from_version(previous).is_ok_and(|p| {
        p.version == m.version
            && p.controller_connection_id == m.controller_connection_id
            && p.host_connection_id == m.host_connection_id
            && p.encoding == m.encoding
            && p.tx_type == m.tx_type
    })
}

/// `address.Hash(typ, key)`.
fn address_hash(typ: &[u8], key: &[u8]) -> [u8; 32] {
    let mut pre = hash::sha256(typ).to_vec();
    pre.extend_from_slice(key);
    hash::sha256(&pre)
}

/// v6 `GenerateAddress`: `Derive(Module("interchainaccounts",
/// "icahost-accounts"), connectionID + portID + AppHash + DataHash)`.
fn generate_address(ctx: &Context<'_>, connection_id: &str, port_id: &str) -> Vec<u8> {
    let mut mkey = x_icahost::MODULE_NAME.as_bytes().to_vec();
    mkey.push(0);
    mkey.extend_from_slice(b"icahost-accounts");
    let module = address_hash(b"module", &mkey);
    let mut buf = format!("{connection_id}{port_id}").into_bytes();
    buf.extend_from_slice(&ctx.header.app_hash);
    DATA_HASH.with(|d| buf.extend_from_slice(&d.borrow()));
    address_hash(&module, &buf).to_vec()
}

/// `createInterchainAccount` (v0.46 `NewAccount` takes the next account
/// number).
fn create_interchain_account(
    ctx: &mut Context<'_>,
    connection_id: &str,
    controller_port: &str,
) -> Result<Vec<u8>> {
    let addr = generate_address(ctx, connection_id, controller_port);
    if x_auth::get_account(ctx, &addr)?.is_some() {
        return Err(errors::account_already_exist(&format!(
            "existing account for newly generated interchain account address {}",
            x_auth::bech32(&addr)?
        )));
    }
    let number = x_auth::next_account_number(ctx)?;
    let acc = x_auth::Account::Interchain(InterchainAccount {
        base_account: Some(BaseAccount {
            address: x_auth::bech32(&addr)?,
            pub_key: None,
            account_number: number,
            sequence: 0,
        }),
        account_owner: controller_port.to_owned(),
    });
    x_auth::set_account(ctx, &acc)?;
    x_icahost::set_interchain_account_address(
        ctx,
        connection_id,
        controller_port,
        acc.address_str(),
    )?;
    Ok(addr)
}

/// `IBCModule.OnChanOpenTry` and the keeper's: the version with the
/// interchain account address.
#[allow(clippy::trivially_copy_pass_by_ref, clippy::too_many_arguments)]
pub(super) fn on_chan_open_try(
    ctx: &mut Context<'_>,
    order: i32,
    hops: &[String],
    port: &str,
    chan: &str,
    cap: &Capability,
    counterparty: &Counterparty,
    counterparty_version: &str,
) -> Result<String> {
    if !host_enabled(ctx)? {
        return Err(errors::host_sub_module_disabled());
    }
    if order != Order::Ordered as i32 {
        return Err(cherr::invalid_channel_ordering(format!(
            "expected {}, got {}",
            channel::order_name(Order::Ordered as i32),
            channel::order_name(order)
        )));
    }
    if port != HOST_PORT_ID {
        return Err(errors::invalid_host_port(&format!(
            "expected {HOST_PORT_ID}, got {port}"
        )));
    }
    let mut metadata = ica::metadata_from_version(counterparty_version)?;
    let hop = hops.first().cloned().unwrap_or_default();
    validate_host_metadata(ctx, &hop, &metadata)?;
    if let Some(active) = ica::active_channel_id(ctx, &hop, &counterparty.port_id)? {
        let Some(ch) = channel::get_channel(ctx, port, &active)? else {
            return Err(sdk::error::go_panic(format!(
                "active channel mapping set for {active} but channel does not exist in channel store"
            )));
        };
        if ch.state == State::Open as i32 {
            return Err(errors::active_channel_already_set(&format!(
                "existing active channel {active} for portID {port} is already OPEN"
            )));
        }
        // `GetAppVersion` reads the channel again.
        let Some(ch) = channel::get_channel(ctx, port, &active)? else {
            return Err(sdk::error::go_panic(format!(
                "active channel mapping set for {active}, but channel does not exist in channel store"
            )));
        };
        if !is_previous_metadata_equal(&ch.version, &metadata) {
            return Err(errors::invalid_version(
                "previous active channel metadata does not match provided version",
            ));
        }
    }
    SCOPED_KEEPER
        .claim_capability(ctx, cap, &x_ibc::host::channel_capability_path(port, chan))
        .map_err(|e| {
            e.wrap(format!(
                "failed to claim capability for channel {chan} on port {port}"
            ))
        })?;
    let addr = if let Some(existing) =
        ica::interchain_account_address(ctx, &metadata.host_connection_id, &counterparty.port_id)?
    {
        let addr = x_auth::parse_bech32(&existing).map_err(|e| sdk::error::go_panic(e.log))?;
        if !matches!(
            x_auth::get_account(ctx, &addr)?,
            Some(x_auth::Account::Interchain(_))
        ) {
            return Err(errors::invalid_account_reopening(&format!(
                "existing account address {existing}, does not have interchain account type"
            )));
        }
        addr
    } else {
        create_interchain_account(ctx, &metadata.host_connection_id, &counterparty.port_id)?
    };
    metadata.address = x_auth::bech32(&addr)?;
    Ok(ica::metadata_json(&metadata))
}

/// `IBCModule.OnChanOpenConfirm` and the keeper's: the channel becomes
/// the active channel of its controller port.
pub(super) fn on_chan_open_confirm(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<()> {
    if !host_enabled(ctx)? {
        return Err(errors::host_sub_module_disabled());
    }
    let ch = channel::get_channel(ctx, port, chan)?.ok_or_else(|| {
        cherr::channel_not_found(format!("failed to retrieve channel {chan} on port {port}"))
    })?;
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    let cp_port = ch.counterparty.unwrap_or_default().port_id;
    x_icahost::set_active_channel_id(ctx, &hop, &cp_port, chan)
}

/// `IBCModule.OnRecvPacket`: a result acknowledgement with the
/// `TxMsgData` of the executed messages, or an error acknowledgement
/// (its ABCI code only).
pub(super) fn on_recv_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<AckResponse> {
    if !host_enabled(ctx)? {
        let e = errors::host_sub_module_disabled();
        return Ok(super::ibc_transfer::error_ack(&e));
    }
    let res = keeper_on_recv_packet(ctx, p);
    let (ack, err) = match res {
        Ok(bz) => (AckResponse::Result(bz), None),
        Err(e) if e.codespace == "undefined" && e.code == 111_222 => return Err(e),
        Err(e) => (super::ibc_transfer::error_ack(&e), Some(e)),
    };
    let mut attrs = vec![
        ("module", x_icahost::MODULE_NAME.to_owned()),
        ("host_channel_id", p.destination_channel.clone()),
        ("success", ack.success().to_string()),
    ];
    if let Some(e) = &err {
        attrs.push(("error", e.log.clone()));
    }
    ctx.events.emit(event("ics27_packet", attrs));
    Ok(ack)
}

/// Keeper `OnRecvPacket`.
fn keeper_on_recv_packet(ctx: &mut Context<'_>, p: &Packet) -> Result<Vec<u8>> {
    let (ty, data) = ica::packet_data(&p.data).ok_or_else(|| {
        errors::unknown_data_type("cannot unmarshal ICS-27 interchain account packet data")
    })?;
    if ty != 1 {
        return Err(errors::unknown_data_type(""));
    }
    let msgs = deserialize_cosmos_tx(&data)?;
    execute_tx(
        ctx,
        &p.source_port,
        &p.destination_port,
        &p.destination_channel,
        &msgs,
    )
}

/// `DeserializeCosmosTx` for the messages the legacy driver knows. A
/// `CosmosTx` that prost does not decode, or an empty type URL (a nil
/// `sdk.Msg` in Go), is not ported; a type the v0.46 registry does not
/// hold is not ported either (its signers are unknown, see
/// [`authenticate_tx`]).
fn deserialize_cosmos_tx(data: &[u8]) -> Result<Vec<Any>> {
    let tx = CosmosTx::decode(data)
        .map_err(|_| not_ported("a CosmosTx that does not decode with prost"))?;
    if tx.messages.iter().any(|m| m.type_url.is_empty()) {
        return Err(not_ported("a CosmosTx message with an empty type URL"));
    }
    Ok(tx.messages)
}

/// `authenticateTx`.
fn authenticate_tx(
    ctx: &mut Context<'_>,
    msgs: &[Any],
    connection_id: &str,
    port_id: &str,
) -> Result<()> {
    let Some(ica_addr) = ica::interchain_account_address(ctx, connection_id, port_id)? else {
        return Err(errors::interchain_account_not_found(&format!(
            "failed to retrieve interchain account on port {port_id}"
        )));
    };
    let allow = allow_messages(ctx)?;
    for m in msgs {
        if !x_icahost::contains_msg_type(&allow, &m.type_url) {
            return Err(errs::unauthorized(format!(
                "message type not allowed: {}",
                m.type_url
            )));
        }
        let signer = super::authz::exec_signer(m)?.ok_or_else(|| {
            not_ported(&format!("the cosmos-sdk v0.46 signers of {}", m.type_url))
        })?;
        let got = if signer.is_empty() {
            String::new()
        } else {
            x_auth::bech32(&signer)?
        };
        if got != ica_addr {
            return Err(errs::unauthorized(format!(
                "unexpected signer address: expected {ica_addr}, got {got}"
            )));
        }
    }
    Ok(())
}

/// `executeTx`: every message, in one cache context kept only when all
/// succeed.
fn execute_tx(
    ctx: &mut Context<'_>,
    source_port: &str,
    dest_port: &str,
    dest_channel: &str,
    msgs: &[Any],
) -> Result<Vec<u8>> {
    let Some(ch) = channel::get_channel(ctx, dest_port, dest_channel)? else {
        return Err(cherr::channel_not_found(""));
    };
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    authenticate_tx(ctx, msgs, &hop, source_port)?;
    let base = ctx.multistore().depth();
    let cp = ctx.branch();
    let mut responses = Vec::with_capacity(msgs.len());
    for m in msgs {
        match execute_msg(ctx, m) {
            Ok(r) => responses.push(r),
            Err(e) => {
                while ctx.multistore().depth() > base {
                    ctx.multistore().discard_branch();
                }
                return Err(e);
            }
        }
    }
    ctx.write(cp);
    #[allow(deprecated)]
    let data = TxMsgData {
        data: Vec::new(),
        msg_responses: responses,
    };
    Ok(data.encode_to_vec())
}

/// `msg.ValidateBasic()`, then `executeMsg`: the router's handler (a
/// fresh event manager, `ValidateBasic` again, the gatekeeper's circuit
/// breaker, the msg server); its events go to the context.
fn execute_msg(ctx: &mut Context<'_>, m: &Any) -> Result<Any> {
    super::msgs::validate_basic(m)?;
    let v = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    match super::tx::gatekeeper_accepts(v, &m.type_url) {
        None => {
            return Err(errs::not_supported(format!(
                "app version {v} is not supported"
            )));
        }
        Some(false) => {
            return Err(x_ibc::errors::unregistered(format!(
                "circuit breaker disables execution of this message: {}",
                m.type_url
            )));
        }
        Some(true) => {}
    }
    let outer = std::mem::take(&mut ctx.events);
    let res = super::msgs::route(ctx, m);
    let nested = std::mem::replace(&mut ctx.events, outer).take();
    let res = res?;
    for e in nested {
        ctx.events.emit(e);
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address hashes the connection, the port, the app hash and the
    /// data hash (Go `GenerateAddress`).
    #[test]
    fn address_hashes_the_app_and_data_hashes() {
        let mut ms = sdk::MultiStore::new();
        let header = sdk::Header {
            app_hash: vec![1; 32],
            ..sdk::Header::default()
        };
        let ctx = Context::new(&mut ms, header, sdk::ExecMode::Finalize);
        set_data_hash(&[]);
        let without = generate_address(&ctx, "connection-0", "icacontroller-x");
        set_data_hash(&[2; 32]);
        let with = generate_address(&ctx, "connection-0", "icacontroller-x");
        assert_ne!(without, with);
        assert_eq!(with.len(), 32);
        // Without a data hash it is the v8 address of the same header.
        assert_eq!(
            without,
            x_icahost::host::generate_address(&ctx, "connection-0", "icacontroller-x")
        );
        set_data_hash(&[]);
    }

    /// Previous metadata must agree on every field but the address.
    #[test]
    fn previous_metadata_ignores_the_address() {
        let m = Metadata {
            version: VERSION.into(),
            controller_connection_id: "connection-1".into(),
            host_connection_id: "connection-0".into(),
            address: "a".into(),
            encoding: ica::ENCODING_PROTOBUF.into(),
            tx_type: ica::TX_TYPE_SDK_MULTI_MSG.into(),
        };
        let mut prev = m.clone();
        prev.address = "b".into();
        assert!(is_previous_metadata_equal(&ica::metadata_json(&prev), &m));
        prev.host_connection_id = "connection-2".into();
        assert!(!is_previous_metadata_equal(&ica::metadata_json(&prev), &m));
        assert!(!is_previous_metadata_equal("not json", &m));
    }
}
