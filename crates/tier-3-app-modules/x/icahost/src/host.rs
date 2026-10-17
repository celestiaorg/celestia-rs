//! The ICS-27 host `IBCModule` (`host/ibc_module.go`), its keeper logic
//! (`host/keeper/handshake.go`, `relay.go`, `account.go`) and message
//! server (`MsgUpdateParams`).
//!
//! `MsgModuleQuerySafe` runs the app's gRPC query router (supplied by the
//! app, see [`module_query_safe`]).

use celestia_proto::Any;
use celestia_proto::capability::v1::Capability;
use celestia_proto::cosmos::auth::v1beta1::BaseAccount;
use celestia_proto::cosmos::base::abci::v1beta1::TxMsgData;
use celestia_proto::ibc::applications::interchain_accounts::host::v1::{
    MsgModuleQuerySafe, MsgModuleQuerySafeResponse, MsgUpdateParams, MsgUpdateParamsResponse,
};
use celestia_proto::ibc::applications::interchain_accounts::v1::{
    CosmosTx, InterchainAccount, Metadata,
};
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet, State};
use prost::{Message, Name};
use sdk::event::event;
use sdk::router::MsgRouter;
use sdk::{Context, Result, SdkError};
use x_ibc::errors::{channel as cherr, connection as conerr, ibc as ierr};
use x_ibc::gojson::escape_string;
use x_ibc::router::{Acknowledgement, IbcModule};

use crate::{HOST_PORT_ID, MODULE_NAME, SCOPED_KEEPER, STORE_KEY, VERSION};

/// `interchainaccounts` codespace errors (`types/errors.go`).
pub mod errors {
    use sdk::SdkError;

    fn e(code: u32, desc: &str, log: &str) -> SdkError {
        SdkError::new(
            super::MODULE_NAME,
            code,
            if log.is_empty() {
                desc.to_owned()
            } else {
                format!("{log}: {desc}")
            },
        )
    }

    macro_rules! ica_errors {
        ($($name:ident = $code:expr, $desc:expr;)*) => {
            $(
                #[doc = $desc]
                #[must_use]
                pub fn $name(log: &str) -> SdkError {
                    e($code, $desc, log)
                }
            )*
        };
    }

    ica_errors! {
        unknown_data_type = 2, "unknown data type";
        account_already_exist = 3, "account already exist";
        invalid_channel_flow = 5, "invalid message sent to channel end";
        invalid_route = 7, "invalid route";
        interchain_account_not_found = 8, "interchain account not found";
        active_channel_already_set = 10, "active channel already set for this owner";
        invalid_version = 12, "invalid interchain accounts version";
        invalid_account_address = 13, "invalid account address";
        invalid_host_port = 16, "invalid host port";
        invalid_codec = 18, "codec is not supported";
        invalid_account_reopening = 19, "invalid account reopening";
    }

    /// `ErrHostSubModuleDisabled` (`icahost` codespace, 2).
    #[must_use]
    pub fn host_sub_module_disabled() -> SdkError {
        SdkError::new("icahost", 2, "host submodule is disabled")
    }
}

/// `EncodingProtobuf`.
pub const ENCODING_PROTOBUF: &str = "proto3";
/// `EncodingProto3JSON`.
pub const ENCODING_PROTO3_JSON: &str = "proto3json";
/// `TxTypeSDKMultiMsg`.
pub const TX_TYPE_SDK_MULTI_MSG: &str = "sdk_multi_msg";
/// `DefaultMaxAddrLength`.
const MAX_ADDR_LENGTH: usize = 128;

// ---------------------------------------------------------------------------
// Metadata (`types/metadata.go`)
// ---------------------------------------------------------------------------

const METADATA_FIELDS: [(&str, &str); 6] = [
    ("version", "version"),
    ("controller_connection_id", "controllerConnectionId"),
    ("host_connection_id", "hostConnectionId"),
    ("address", "address"),
    ("encoding", "encoding"),
    ("tx_type", "txType"),
];

/// `MetadataFromVersion` (`ModuleCdc.UnmarshalJSON`, jsonpb).
pub fn metadata_from_version(version: &str) -> Result<Metadata> {
    let bad = || errors::unknown_data_type("cannot unmarshal ICS-27 interchain accounts metadata");
    let v = x_ibc::gojson::first_value(&gostr::to_go_bytes(version)).map_err(|_| bad())?;
    let mut f: [String; 6] = Default::default();
    match v {
        serde_json::Value::Null => {}
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                let Some(i) = METADATA_FIELDS
                    .iter()
                    .position(|(orig, camel)| *orig == k || *camel == k)
                else {
                    return Err(bad());
                };
                match val {
                    serde_json::Value::Null => {}
                    serde_json::Value::String(s) => f[i] = s,
                    _ => return Err(bad()),
                }
            }
        }
        _ => return Err(bad()),
    }
    let [
        version,
        controller_connection_id,
        host_connection_id,
        address,
        encoding,
        tx_type,
    ] = f;
    Ok(Metadata {
        version,
        controller_connection_id,
        host_connection_id,
        address,
        encoding,
        tx_type,
    })
}

/// `ModuleCdc.MarshalJSON(&metadata)` (jsonpb, defaults emitted).
#[must_use]
pub fn metadata_json(m: &Metadata) -> String {
    let values = [
        &m.version,
        &m.controller_connection_id,
        &m.host_connection_id,
        &m.address,
        &m.encoding,
        &m.tx_type,
    ];
    let parts: Vec<String> = METADATA_FIELDS
        .iter()
        .zip(values)
        .map(|((k, _), v)| format!("\"{k}\":{}", escape_string(v)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// `ValidateAccountAddress`.
fn validate_account_address(addr: &str) -> Result<()> {
    if addr.is_empty()
        || !addr.bytes().all(|b| b.is_ascii_alphanumeric())
        || addr.len() > MAX_ADDR_LENGTH
    {
        return Err(errors::invalid_account_address(&format!(
            "address must contain strictly alphanumeric characters, not exceeding {MAX_ADDR_LENGTH} characters in length"
        )));
    }
    Ok(())
}

/// `channelKeeper.GetConnection` (an error when missing).
fn get_connection(
    ctx: &mut Context<'_>,
    id: &str,
) -> Result<celestia_proto::ibc::core::connection::v1::ConnectionEnd> {
    x_ibc::connection::get_connection(ctx, id)?.ok_or_else(|| conerr::connection_not_found(id))
}

/// `ValidateHostMetadata`.
fn validate_host_metadata(ctx: &mut Context<'_>, hops: &[String], m: &Metadata) -> Result<()> {
    if m.encoding != ENCODING_PROTOBUF && m.encoding != ENCODING_PROTO3_JSON {
        return Err(errors::invalid_codec(&format!(
            "unsupported encoding format {}",
            m.encoding
        )));
    }
    if m.tx_type != TX_TYPE_SDK_MULTI_MSG {
        return Err(errors::unknown_data_type(&format!(
            "unsupported transaction type {}",
            m.tx_type
        )));
    }
    let hop = hops.first().cloned().unwrap_or_default();
    let conn = get_connection(ctx, &hop)?;
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

// ---------------------------------------------------------------------------
// Keeper
// ---------------------------------------------------------------------------

/// `GetParams` (Go panics when unset).
fn params(
    ctx: &mut Context<'_>,
) -> Result<celestia_proto::ibc::applications::interchain_accounts::host::v1::Params> {
    crate::get_params(ctx).map_err(|e| sdk::error::go_panic(e.log))
}

/// `GetActiveChannelID` (`Has` then `Get`).
pub fn active_channel_id(
    ctx: &mut Context<'_>,
    connection_id: &str,
    port_id: &str,
) -> Result<Option<String>> {
    let key = crate::key_active_channel(port_id, connection_id);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    Ok(Some(String::from_utf8_lossy(&bz).into_owned()))
}

/// `GetInterchainAccountAddress` (`Has` then `Get`).
pub fn interchain_account_address(
    ctx: &mut Context<'_>,
    connection_id: &str,
    port_id: &str,
) -> Result<Option<String>> {
    let key = crate::key_owner_account(port_id, connection_id);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    let bz = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
    Ok(Some(String::from_utf8_lossy(&bz).into_owned()))
}

/// `address.Hash(typ, key)`.
fn address_hash(typ: &[u8], key: &[u8]) -> [u8; 32] {
    let th = hash::sha256(typ);
    let mut pre = th.to_vec();
    pre.extend_from_slice(key);
    hash::sha256(&pre)
}

/// `GenerateAddress`: `Derive(Module("interchainaccounts",
/// "icahost-accounts"), connectionID + portID + header.AppHash +
/// header.DataHash)` (the SDK sets no data hash in the block header).
#[must_use]
pub fn generate_address(ctx: &Context<'_>, connection_id: &str, port_id: &str) -> Vec<u8> {
    let mut mkey = MODULE_NAME.as_bytes().to_vec();
    mkey.push(0);
    mkey.extend_from_slice(b"icahost-accounts");
    let module = address_hash(b"module", &mkey);
    let mut buf = format!("{connection_id}{port_id}").into_bytes();
    buf.extend_from_slice(&ctx.header.app_hash);
    address_hash(&module, &buf).to_vec()
}

/// `createInterchainAccount`.
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
    crate::set_interchain_account_address(ctx, connection_id, controller_port, acc.address_str())?;
    Ok(addr)
}

/// Keeper `OnChanOpenTry`.
#[allow(clippy::trivially_copy_pass_by_ref, clippy::single_match_else)]
fn keeper_on_chan_open_try(
    ctx: &mut Context<'_>,
    hops: &[String],
    port_id: &str,
    channel_id: &str,
    cap: &Capability,
    counterparty: &Counterparty,
    counterparty_version: &str,
) -> Result<String> {
    if port_id != HOST_PORT_ID {
        return Err(errors::invalid_host_port(&format!(
            "expected {HOST_PORT_ID}, got {port_id}"
        )));
    }
    let hop = hops.first().cloned().unwrap_or_default();
    let mut metadata = match metadata_from_version(counterparty_version) {
        Ok(m) => m,
        Err(_) => {
            let conn = get_connection(ctx, &hop)
                .map_err(|e| e.wrap(format!("failed to retrieve connection {hop}")))?;
            Metadata {
                version: VERSION.to_owned(),
                controller_connection_id: conn.counterparty.unwrap_or_default().connection_id,
                host_connection_id: hop.clone(),
                address: String::new(),
                encoding: ENCODING_PROTOBUF.to_owned(),
                tx_type: TX_TYPE_SDK_MULTI_MSG.to_owned(),
            }
        }
    };
    validate_host_metadata(ctx, hops, &metadata)?;
    if let Some(active) = active_channel_id(ctx, &hop, &counterparty.port_id)? {
        let Some(ch) = x_ibc::channel::get_channel(ctx, port_id, &active)? else {
            return Err(sdk::error::go_panic(format!(
                "active channel mapping set for {active} but channel does not exist in channel store"
            )));
        };
        if ch.state != State::Closed as i32 {
            return Err(errors::active_channel_already_set(&format!(
                "existing active channel {active} for portID {port_id} must be {}",
                x_ibc::channel::state_name(State::Closed as i32)
            )));
        }
    }
    SCOPED_KEEPER
        .claim_capability(
            ctx,
            cap,
            &x_ibc::host::channel_capability_path(port_id, channel_id),
        )
        .map_err(|e| {
            e.wrap(format!(
                "failed to claim capability for channel {channel_id} on port {port_id}"
            ))
        })?;
    let addr = if let Some(existing) =
        interchain_account_address(ctx, &metadata.host_connection_id, &counterparty.port_id)?
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
    Ok(metadata_json(&metadata))
}

/// `getAppMetadata`.
fn app_metadata(ctx: &mut Context<'_>, port_id: &str, channel_id: &str) -> Result<Metadata> {
    let Some(ch) = x_ibc::channel::get_channel(ctx, port_id, channel_id)? else {
        return Err(ierr::not_found(format!(
            "app version not found for port {port_id} and channel {channel_id}"
        )));
    };
    metadata_from_version(&ch.version)
}

/// `InterchainAccountPacketData.UnmarshalJSON`: its `(type, data)`, or
/// `None` where Go returns an error.
#[must_use]
pub fn packet_data(bz: &[u8]) -> Option<(i32, Vec<u8>)> {
    decode_packet_data(bz).ok()
}

/// `InterchainAccountPacketData` (`ModuleCdc.UnmarshalJSON`, jsonpb): `type`
/// an enum (a name, the raw bytes between the quotes, or an int32
/// number), `data` a `[]byte` (base64 or an array of numbers), `memo` a
/// string; `null` leaves a field unset; duplicate keys: the last.
fn decode_packet_data(bz: &[u8]) -> core::result::Result<(i32, Vec<u8>), ()> {
    use gojson::Kind;
    let v = gojson::parse_first(bz).map_err(|_| ())?;
    let mut fields: std::collections::BTreeMap<&str, &gojson::Value> = match &v.kind {
        Kind::Null => return Ok((0, Vec::new())),
        Kind::Object(ms) => ms.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        _ => return Err(()),
    };
    let (mut ty, mut data) = (0i32, Vec::new());
    if let Some(t) = fields.remove("type") {
        let raw = &bz[t.span.clone()];
        ty = match &t.kind {
            Kind::Null => 0,
            Kind::String(_) => match &raw[1..raw.len() - 1] {
                b"TYPE_UNSPECIFIED" => 0,
                b"TYPE_EXECUTE_TX" => 1,
                _ => return Err(()),
            },
            Kind::Number(n) => n.parse::<i32>().map_err(|_| ())?,
            _ => return Err(()),
        };
    }
    if let Some(d) = fields.remove("data") {
        data = gojson::bytes_value(d).map_err(|_| ())?.unwrap_or_default();
    }
    if let Some(m) = fields.remove("memo")
        && !matches!(m.kind, Kind::Null | Kind::String(_))
    {
        return Err(());
    }
    if !fields.is_empty() {
        return Err(());
    }
    Ok((ty, data))
}

/// Whether the app version is 9 (celestia-app v9.0.8's registry and schema
/// for the gogoproto decoding of host messages).
fn is_v9(ctx: &Context<'_>) -> bool {
    ctx.consensus_params
        .version
        .as_ref()
        .is_some_and(|v| v.app != 0 && v.app < 10)
}

/// Keeper `OnRecvPacket`: the `TxMsgData` result bytes.
fn keeper_on_recv_packet(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    reg: Registry,
    p: &Packet,
) -> Result<Vec<u8>> {
    let (ty, data) = decode_packet_data(&p.data).map_err(|()| {
        errors::unknown_data_type("cannot unmarshal ICS-27 interchain account packet data")
    })?;
    let metadata = app_metadata(ctx, &p.destination_port, &p.destination_channel)?;
    if ty != 1 {
        return Err(errors::unknown_data_type(""));
    }
    let v9 = is_v9(ctx);
    let msgs = deserialize_cosmos_tx(&data, &metadata.encoding, reg, v9)
        .map_err(|e| e.wrap("failed to deserialize interchain account transaction"))?;
    execute_tx(
        ctx,
        router,
        &p.source_port,
        &p.destination_port,
        &p.destination_channel,
        &msgs,
    )
    .map_err(|e| e.wrap("failed to execute interchain account transaction"))
}

/// `DeserializeCosmosTx`: the messages, `None` where Go holds a nil
/// `sdk.Msg` (a `null` message in proto3json, an `Any` with an empty type
/// URL in protobuf: `UnpackAny` returns early for both). `v9`: the
/// celestia-app v9.0.8 registry and schema.
pub fn deserialize_cosmos_tx(
    data: &[u8],
    encoding: &str,
    reg: Registry,
    v9: bool,
) -> Result<Vec<Option<Any>>> {
    txdecode::scoped_v9(v9, || deserialize_cosmos_tx_inner(data, encoding, reg, v9))
}

fn deserialize_cosmos_tx_inner(
    data: &[u8],
    encoding: &str,
    reg: Registry,
    v9: bool,
) -> Result<Vec<Option<Any>>> {
    let is_msg = reg.is_msg;
    let msgs: Vec<Option<Any>> = match encoding {
        ENCODING_PROTOBUF => {
            let wrap = |e: String| {
                x_ibc::errors::unregistered(format!("cannot unmarshal CosmosTx with protobuf: {e}"))
            };
            // `ProtoCodec.Unmarshal`: gogoproto's generated `Unmarshal`
            // (Go's error text), then `UnpackInterfaces`: each message in
            // turn is resolved as an `sdk.Msg`, unmarshaled and unpacked
            // (10 nested `Any` levels, 100 calls).
            let canon =
                txdecode::gogo_unmarshal("ibc.applications.interchain_accounts.v1.CosmosTx", data)
                    .map_err(wrap)?;
            let tx = CosmosTx::decode(canon.as_slice()).map_err(|e| wrap(e.to_string()))?;
            for m in &tx.messages {
                if m.type_url.is_empty() {
                    continue;
                }
                if !is_msg(&m.type_url) {
                    return Err(wrap(format!(
                        "no concrete type registered for type URL {} against interface *types.Msg",
                        m.type_url
                    )));
                }
                let name = m.type_url.rsplit('/').next().unwrap_or_default();
                txdecode::gogo_unmarshal(name, &m.value).map_err(wrap)?;
                match txdecode::unpack_cosmos_tx_msgs(std::slice::from_ref(m)) {
                    Ok(()) => {}
                    Err(txdecode::GoFail::Err(e)) => return Err(wrap(e)),
                    Err(txdecode::GoFail::Panic(p)) => return Err(sdk::error::go_panic(p)),
                }
            }
            tx.messages
                .into_iter()
                .map(|m| (!m.type_url.is_empty()).then_some(m))
                .collect()
        }
        // `UnmarshalJSON` (jsonpb, then `UnpackInterfaces`): any error is
        // replaced by `ErrUnknownDataType`; a panic (a nil element of a
        // `[]*T` field reaching gogoproto's `Marshal`, ...) unwinds.
        ENCODING_PROTO3_JSON => match txdecode::unmarshal_cosmos_tx_json(data, v9) {
            Ok(msgs) => msgs,
            Err(txdecode::GoFail::Panic(p)) => return Err(sdk::error::go_panic(p)),
            Err(txdecode::GoFail::Err(_)) => {
                return Err(errors::unknown_data_type(
                    "cannot unmarshal CosmosTx with proto3 json",
                ));
            }
        },
        other => {
            return Err(errors::invalid_codec(&format!(
                "unsupported encoding format {other}"
            )));
        }
    };
    for m in msgs.iter().flatten() {
        // `UnpackAny` into `sdk.Msg`: the type must be registered.
        if !is_msg(&m.type_url) {
            return Err(x_ibc::errors::unregistered(format!(
                "no concrete type registered for type URL {} against interface *types.Msg",
                m.type_url
            )));
        }
    }
    Ok(msgs)
}

/// The app's interface registry (`codectypes.InterfaceRegistry`).
#[derive(Debug, Clone, Copy)]
pub struct Registry {
    /// Whether a type URL resolves (`typeURLMap`).
    pub is_registered: fn(&str) -> bool,
    /// Whether a type URL is registered as `sdk.Msg`.
    pub is_msg: fn(&str) -> bool,
}

/// `authenticateTx`.
fn authenticate_tx(
    ctx: &mut Context<'_>,
    msgs: &[Option<Any>],
    connection_id: &str,
    port_id: &str,
) -> Result<()> {
    let Some(ica) = interchain_account_address(ctx, connection_id, port_id)? else {
        return Err(errors::interchain_account_not_found(&format!(
            "failed to retrieve interchain account on port {port_id}"
        )));
    };
    let allow = params(ctx)?.allow_messages;
    for m in msgs {
        let Some(m) = m else {
            // A nil `sdk.Msg`: `sdk.MsgTypeURL(nil)` is "/", and
            // `GetMsgV1Signers` fails in `NewAnyWithValue(nil)`.
            if !crate::contains_msg_type(&allow, "/") {
                return Err(ierr::unauthorized("message type not allowed: /"));
            }
            return Err(sdk::error::sdk::pack_any(
                "failed to obtain message signers for message type /: Expecting non nil value to create a new Any",
            ));
        };
        if !crate::contains_msg_type(&allow, &m.type_url) {
            return Err(ierr::unauthorized(format!(
                "message type not allowed: {}",
                m.type_url
            )));
        }
        let signers = msg_v1_signers(is_v9(ctx), m)?;
        for bytes in signers {
            let got = sdk::address::to_bech32(sdk::address::ACC_PREFIX, &bytes);
            if got != ica {
                return Err(ierr::unauthorized(format!(
                    "unexpected signer address: expected {ica}, got {got}"
                )));
            }
        }
    }
    Ok(())
}

/// `GetMsgV1Signers` of a host message: Go marshals the decoded message
/// again (`NewAnyWithValue`), runs `anyutil.Unpack` on those bytes
/// (dynamicpb UTF-8 / varint checks; its plain error keeps code 1 through
/// `Wrapf`) and reads the signers from them. The message as received may
/// hold encodings prost cannot read that gogoproto accepted (a 10-byte
/// length prefix with high bits), so the signers come from the
/// re-marshaled bytes too.
fn msg_v1_signers(v9: bool, m: &Any) -> Result<Vec<Vec<u8>>> {
    let repacked = txdecode::scoped_v9(v9, || txdecode::gogo_repack(m));
    if let Err(e) = celestia_proto::signers::unpack_check(&repacked.type_url, &repacked.value) {
        return Err(sdk::address::undefined(format!(
            "failed to obtain message signers for message type {}: {e}",
            m.type_url
        )));
    }
    let signers = match celestia_proto::signers::get_signers(&repacked.type_url, &repacked.value) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            return Err(x_ibc::errors::unregistered(format!(
                "failed to obtain message signers for message type {}: {e}",
                m.type_url
            )));
        }
        None => Vec::new(),
    };
    x_ante::txview::signer_bytes(&m.type_url, signers).map_err(|e| {
        e.wrap(format!(
            "failed to obtain message signers for message type {}",
            m.type_url
        ))
    })
}

/// `executeTx`.
fn execute_tx(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    source_port: &str,
    dest_port: &str,
    dest_channel: &str,
    msgs: &[Option<Any>],
) -> Result<Vec<u8>> {
    let Some(ch) = x_ibc::channel::get_channel(ctx, dest_port, dest_channel)? else {
        return Err(cherr::channel_not_found(""));
    };
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    authenticate_tx(ctx, msgs, &hop, source_port)?;
    // `authenticateTx`'s `GetMsgV1Signers` marshaled each message: nil
    // customtypes are zero when they execute. (A nil message failed it.)
    let msgs: Vec<Any> = txdecode::scoped_v9(is_v9(ctx), || {
        msgs.iter()
            .flatten()
            .map(txdecode::gogo_marshaled)
            .collect()
    });
    let msgs = msgs.as_slice();
    let cp = ctx.branch();
    let mut responses = Vec::with_capacity(msgs.len());
    for m in msgs {
        let res = router.validate_basic(m).and_then(|()| {
            if !router.has_handler(&m.type_url) {
                return Err(errors::invalid_route(""));
            }
            router.handle(ctx, m)
        });
        match res {
            Ok(r) => responses.push(r),
            Err(e) => {
                ctx.discard(cp);
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

fn packet_event(
    p: &Packet,
    success: bool,
    err: Option<&SdkError>,
) -> celestia_proto::celestia_core_proto::abci::Event {
    let mut attrs = vec![
        ("module", MODULE_NAME.to_owned()),
        ("host_channel_id", p.destination_channel.clone()),
        ("success", success.to_string()),
    ];
    if let Some(e) = err {
        attrs.push(("error", e.log.clone()));
    }
    event("ics27_packet", attrs)
}

// ---------------------------------------------------------------------------
// IBC module
// ---------------------------------------------------------------------------

/// The host `IBCModule`, executing packet messages through `router`.
#[derive(Debug, Clone)]
pub struct HostModule<R> {
    /// The app's message router (`baseapp.MsgServiceRouter`).
    pub router: R,
    /// The app's interface registry.
    pub registry: Registry,
}

fn flow(msg: &str) -> SdkError {
    errors::invalid_channel_flow(msg)
}

impl<R: MsgRouter> IbcModule for HostModule<R> {
    fn on_chan_open_init(
        &self,
        _ctx: &mut Context<'_>,
        _order: Order,
        _hops: &[String],
        _port_id: &str,
        _channel_id: &str,
        _cap: &Capability,
        _cp: &Counterparty,
        _version: &str,
    ) -> Result<String> {
        Err(flow(
            "channel handshake must be initiated by controller chain",
        ))
    }

    fn on_chan_open_try(
        &self,
        ctx: &mut Context<'_>,
        _order: Order,
        hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        cp: &Counterparty,
        cp_version: &str,
    ) -> Result<String> {
        if !params(ctx)?.host_enabled {
            return Err(errors::host_sub_module_disabled());
        }
        keeper_on_chan_open_try(ctx, hops, port_id, channel_id, cap, cp, cp_version)
    }

    fn on_chan_open_ack(
        &self,
        _ctx: &mut Context<'_>,
        _port_id: &str,
        _channel_id: &str,
        _cp_channel_id: &str,
        _cp_version: &str,
    ) -> Result<()> {
        Err(flow(
            "channel handshake must be initiated by controller chain",
        ))
    }

    fn on_chan_open_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        if !params(ctx)?.host_enabled {
            return Err(errors::host_sub_module_disabled());
        }
        let Some(ch) = x_ibc::channel::get_channel(ctx, port_id, channel_id)? else {
            return Err(cherr::channel_not_found(format!(
                "failed to retrieve channel {channel_id} on port {port_id}"
            )));
        };
        let hop = ch.connection_hops.first().cloned().unwrap_or_default();
        let cp_port = ch.counterparty.unwrap_or_default().port_id;
        crate::set_active_channel_id(ctx, &hop, &cp_port, channel_id)
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
        if !params(ctx)?.host_enabled {
            let e = errors::host_sub_module_disabled();
            ctx.events.emit(event(
                "ics27_packet",
                [
                    ("module", MODULE_NAME.to_owned()),
                    ("host_channel_id", p.destination_channel.clone()),
                    ("error", e.log.clone()),
                    ("success", "false".to_owned()),
                ],
            ));
            return Ok(Some(Acknowledgement::error(&e)));
        }
        match keeper_on_recv_packet(ctx, &self.router, self.registry, p) {
            Ok(res) => {
                ctx.events.emit(packet_event(p, true, None));
                Ok(Some(Acknowledgement::result(&res)))
            }
            Err(e) if e.codespace == "undefined" && e.code == 111_222 => Err(e),
            Err(e) => {
                ctx.events.emit(packet_event(p, false, Some(&e)));
                Ok(Some(Acknowledgement::error(&e)))
            }
        }
    }

    fn on_acknowledgement_packet(
        &self,
        _ctx: &mut Context<'_>,
        _p: &Packet,
        _ack: &[u8],
        _relayer: &[u8],
    ) -> Result<()> {
        Err(flow(
            "cannot receive acknowledgement on a host channel end, a host chain does not send a packet over the channel",
        ))
    }

    fn on_timeout_packet(
        &self,
        _ctx: &mut Context<'_>,
        _p: &Packet,
        _relayer: &[u8],
    ) -> Result<()> {
        Err(flow(
            "cannot cause a packet timeout on a host channel end, a host chain does not send a packet over the channel",
        ))
    }

    fn upgradable(&self) -> bool {
        true
    }

    fn on_chan_upgrade_init(
        &self,
        _ctx: &mut Context<'_>,
        _port_id: &str,
        _channel_id: &str,
        _order: Order,
        _hops: &[String],
        _version: &str,
    ) -> Result<String> {
        Err(flow(
            "channel upgrade handshake must be initiated by controller chain",
        ))
    }

    fn on_chan_upgrade_try(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        _order: Order,
        hops: &[String],
        cp_version: &str,
    ) -> Result<String> {
        if !params(ctx)?.host_enabled {
            return Err(errors::host_sub_module_disabled());
        }
        keeper_on_chan_upgrade_try(ctx, port_id, channel_id, hops, cp_version)
    }

    fn on_chan_upgrade_ack(
        &self,
        _ctx: &mut Context<'_>,
        _port_id: &str,
        _channel_id: &str,
        _cp_version: &str,
    ) -> Result<()> {
        Err(flow(
            "channel upgrade handshake must be initiated by controller chain",
        ))
    }
}

/// Keeper `OnChanUpgradeTry`.
fn keeper_on_chan_upgrade_try(
    ctx: &mut Context<'_>,
    port_id: &str,
    channel_id: &str,
    hops: &[String],
    cp_version: &str,
) -> Result<String> {
    if port_id != HOST_PORT_ID {
        return Err(x_ibc::errors::port::invalid_port(format!(
            "expected {HOST_PORT_ID}, got {port_id}"
        )));
    }
    let Some(ch) = x_ibc::channel::get_channel(ctx, port_id, channel_id)? else {
        return Err(cherr::channel_not_found(format!(
            "port ID ({port_id}) channel ID ({channel_id})"
        )));
    };
    let connection_id = ch.connection_hops.first().cloned().ok_or_else(|| {
        sdk::error::go_panic("runtime error: index out of range [0] with length 0")
    })?;
    if hops.len() != 1 || hops[0] != connection_id {
        return Err(cherr::invalid_upgrade(format!(
            "expected connection hops [{connection_id}], got [{}]",
            hops.join(" ")
        )));
    }
    if cp_version.trim().is_empty() {
        return Err(cherr::invalid_channel_version(
            "counterparty version cannot be empty",
        ));
    }
    let proposed = metadata_from_version(cp_version)?;
    let current = app_metadata(ctx, port_id, channel_id)?;
    validate_host_metadata(ctx, hops, &proposed).map_err(|e| e.wrap("invalid metadata"))?;
    if current.address != proposed.address {
        return Err(errors::invalid_account_address(
            "interchain account address cannot be changed",
        ));
    }
    if current.controller_connection_id != proposed.controller_connection_id {
        return Err(conerr::invalid_connection(
            "proposed controller connection ID must not change",
        ));
    }
    if current.host_connection_id != hops[0] {
        return Err(conerr::invalid_connection_identifier(
            "proposed connection hop must not change",
        ));
    }
    Ok(cp_version.to_owned())
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Whether `type_url` is an ICA host message.
#[must_use]
pub fn is_host_msg(type_url: &str) -> bool {
    type_url == MsgUpdateParams::type_url() || type_url == MsgModuleQuerySafe::type_url()
}

fn signer_check(signer: &str) -> Result<()> {
    x_auth::parse_bech32(signer).map(|_| ()).map_err(|e| {
        ierr::invalid_address(format!("string could not be parsed as address: {}", e.log))
    })
}

/// `ValidateBasic` of the host messages (`None` for other messages).
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if msg.type_url == MsgUpdateParams::type_url() {
        return Some(decode::<MsgUpdateParams>(msg).and_then(|m| {
            signer_check(&m.signer)?;
            crate::validate_params(&m.params.unwrap_or_default())
        }));
    }
    if msg.type_url == MsgModuleQuerySafe::type_url() {
        return Some(decode::<MsgModuleQuerySafe>(msg).and_then(|m| {
            signer_check(&m.signer)?;
            if m.requests.is_empty() {
                return Err(ierr::invalid_request("no queries provided"));
            }
            Ok(())
        }));
    }
    controller_validate_basic(msg)
}

/// `MaximumOwnerLength` of the controller messages (bytes).
const MAXIMUM_OWNER_LENGTH: usize = 2048;
/// `icatypes.MaxMemoCharLength`.
const MAX_MEMO_CHAR_LENGTH: usize = 32768;

/// `ValidateBasic` of the ICA *controller* messages. celestia-app has no
/// controller keeper, so no msg server serves them, but the types are in
/// the interface registry and `runTxAnte`'s `validateBasicTxMsgs` runs
/// their `ValidateBasic` before the handler-existence check (ibc-go
/// v8.7.2 `controller/types/msgs.go`).
fn controller_validate_basic(msg: &Any) -> Option<Result<()>> {
    use celestia_proto::ibc::applications::interchain_accounts::controller::v1 as ctl;
    let owner_check = |owner: &str| -> Result<()> {
        if gostr::is_blank(owner) {
            return Err(ierr::invalid_address("owner address cannot be empty"));
        }
        if gostr::go_len(owner) > MAXIMUM_OWNER_LENGTH {
            return Err(ierr::invalid_address(format!(
                "owner address must not exceed {MAXIMUM_OWNER_LENGTH} bytes"
            )));
        }
        Ok(())
    };
    if msg.type_url == ctl::MsgRegisterInterchainAccount::type_url() {
        return Some(
            decode::<ctl::MsgRegisterInterchainAccount>(msg).and_then(|m| {
                x_ibc::host::connection_identifier_validator(&m.connection_id)
                    .map_err(|e| e.wrap("invalid connection ID"))?;
                owner_check(&m.owner)
            }),
        );
    }
    if msg.type_url == ctl::MsgSendTx::type_url() {
        return Some(decode::<ctl::MsgSendTx>(msg).and_then(|m| {
            x_ibc::host::connection_identifier_validator(&m.connection_id)
                .map_err(|e| e.wrap("invalid connection ID"))?;
            owner_check(&m.owner)?;
            // `PacketData` is non-nullable: absent is the zero value.
            let pd = m.packet_data.unwrap_or_default();
            let outgoing = |log: String| {
                SdkError::new(
                    crate::MODULE_NAME,
                    6,
                    format!("{log}: invalid outgoing data"),
                )
                .wrap("invalid interchain account packet data")
            };
            if pd.r#type == 0 {
                return Err(outgoing("packet data type cannot be unspecified".into()));
            }
            if pd.data.is_empty() {
                return Err(outgoing("packet data cannot be empty".into()));
            }
            if gostr::go_len(&pd.memo) > MAX_MEMO_CHAR_LENGTH {
                return Err(outgoing(format!(
                    "packet data memo cannot be greater than {MAX_MEMO_CHAR_LENGTH} characters"
                )));
            }
            if m.relative_timeout == 0 {
                return Err(ierr::invalid_request("relative timeout cannot be zero"));
            }
            Ok(())
        }));
    }
    if msg.type_url == ctl::MsgUpdateParams::type_url() {
        return Some(decode::<ctl::MsgUpdateParams>(msg).and_then(|m| signer_check(&m.signer)));
    }
    None
}

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))
}

/// The host message server (`None` for other messages).
pub fn handle(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if msg.type_url == MsgUpdateParams::type_url() {
        return Some(decode::<MsgUpdateParams>(msg).and_then(|m| {
            let auth = x_ibc::msgs::authority();
            if auth != m.signer {
                return Err(ierr::unauthorized(format!(
                    "expected {auth}, got {}",
                    m.signer
                )));
            }
            crate::set_params(ctx, &m.params.unwrap_or_default())?;
            Ok(Any::pack(&MsgUpdateParamsResponse {}))
        }));
    }
    None
}

/// Query paths labeled `cosmos.query.v1.module_query_safe` in the proto
/// files linked into celestia-appd (`newModuleQuerySafeAllowList`).
pub const MODULE_QUERY_SAFE: &[&str] = &[
    "/cosmos.auth.v1beta1.Query/Accounts",
    "/cosmos.auth.v1beta1.Query/Account",
    "/cosmos.auth.v1beta1.Query/AccountAddressByID",
    "/cosmos.auth.v1beta1.Query/Params",
    "/cosmos.auth.v1beta1.Query/ModuleAccounts",
    "/cosmos.auth.v1beta1.Query/ModuleAccountByName",
    "/cosmos.auth.v1beta1.Query/AccountInfo",
    "/cosmos.bank.v1beta1.Query/Balance",
    "/cosmos.bank.v1beta1.Query/AllBalances",
    "/cosmos.bank.v1beta1.Query/SpendableBalances",
    "/cosmos.bank.v1beta1.Query/SpendableBalanceByDenom",
    "/cosmos.bank.v1beta1.Query/TotalSupply",
    "/cosmos.bank.v1beta1.Query/SupplyOf",
    "/cosmos.bank.v1beta1.Query/Params",
    "/cosmos.bank.v1beta1.Query/DenomMetadata",
    "/cosmos.bank.v1beta1.Query/DenomMetadataByQueryString",
    "/cosmos.bank.v1beta1.Query/DenomsMetadata",
    "/cosmos.bank.v1beta1.Query/DenomOwners",
    "/cosmos.bank.v1beta1.Query/DenomOwnersByQuery",
    "/cosmos.bank.v1beta1.Query/SendEnabled",
    "/cosmos.staking.v1beta1.Query/Validators",
    "/cosmos.staking.v1beta1.Query/Validator",
    "/cosmos.staking.v1beta1.Query/ValidatorDelegations",
    "/cosmos.staking.v1beta1.Query/ValidatorUnbondingDelegations",
    "/cosmos.staking.v1beta1.Query/Delegation",
    "/cosmos.staking.v1beta1.Query/UnbondingDelegation",
    "/cosmos.staking.v1beta1.Query/DelegatorDelegations",
    "/cosmos.staking.v1beta1.Query/DelegatorUnbondingDelegations",
    "/cosmos.staking.v1beta1.Query/Redelegations",
    "/cosmos.staking.v1beta1.Query/DelegatorValidators",
    "/cosmos.staking.v1beta1.Query/DelegatorValidator",
    "/cosmos.staking.v1beta1.Query/HistoricalInfo",
    "/cosmos.staking.v1beta1.Query/Pool",
    "/cosmos.staking.v1beta1.Query/Params",
    "/cosmos.circuit.v1.Query/Account",
    "/cosmos.circuit.v1.Query/Accounts",
    "/cosmos.circuit.v1.Query/DisabledList",
    "/ibc.core.client.v1.Query/UpgradedConsensusState",
    "/ibc.core.client.v1.Query/VerifyMembership",
];

/// A query router (`baseapp.GRPCQueryRouter.Route` + handler): `None`
/// when no route exists, else the marshaled response or the handler error.
pub type QueryFn<'a> = &'a mut dyn FnMut(&mut Context<'_>, &str, &[u8]) -> Option<Result<Vec<u8>>>;

/// `msgServer.ModuleQuerySafe`.
pub fn module_query_safe(
    ctx: &mut Context<'_>,
    m: &MsgModuleQuerySafe,
    query: QueryFn<'_>,
) -> Result<Any> {
    let mut responses = Vec::with_capacity(m.requests.len());
    for q in &m.requests {
        if !MODULE_QUERY_SAFE.contains(&q.path.as_str()) {
            return Err(ierr::invalid_request(format!(
                "not module query safe: {}",
                q.path
            )));
        }
        let Some(res) = query(ctx, &q.path, &q.data) else {
            return Err(ierr::invalid_request(format!(
                "no route to query: {}",
                q.path
            )));
        };
        responses.push(res?);
    }
    #[allow(clippy::cast_sign_loss)]
    let height = ctx.height() as u64;
    Ok(Any::pack(&MsgModuleQuerySafeResponse { height, responses }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `MsgSend` whose coin denom has a 10-byte length prefix with high
    /// bits: gogoproto decodes it (the host has no unknown-field check),
    /// Go reads the signers from the re-marshaled message.
    #[test]
    fn signers_read_from_the_remarshaled_message() {
        let from = sdk::address::to_bech32(sdk::address::ACC_PREFIX, &[7u8; 20]);
        let field = |n: u8, v: &[u8]| {
            let mut o = vec![n << 3 | 2, u8::try_from(v.len()).unwrap()];
            o.extend_from_slice(v);
            o
        };
        let mut coin = vec![
            0x0a, 0x84, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7e,
        ];
        coin.extend_from_slice(b"utia");
        coin.extend(field(2, b"5"));
        let value = [
            field(1, from.as_bytes()),
            field(2, from.as_bytes()),
            field(3, &coin),
        ]
        .concat();
        let m = Any {
            type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
            value,
        };
        assert!(matches!(
            celestia_proto::signers::get_signers(&m.type_url, &m.value),
            Some(Err(_)) | None
        ));
        assert_eq!(msg_v1_signers(false, &m).unwrap(), vec![vec![7u8; 20]]);
    }

    #[test]
    fn metadata_roundtrip() {
        let m = metadata_from_version(
            r#"{"version":"ics27-1","controllerConnectionId":"connection-0","host_connection_id":"connection-1","address":"","encoding":"proto3","tx_type":"sdk_multi_msg"}"#,
        )
        .unwrap();
        assert_eq!(m.controller_connection_id, "connection-0");
        assert_eq!(
            metadata_json(&m),
            r#"{"version":"ics27-1","controller_connection_id":"connection-0","host_connection_id":"connection-1","address":"","encoding":"proto3","tx_type":"sdk_multi_msg"}"#
        );
        assert!(metadata_from_version("ics27-1").is_err());
        assert!(metadata_from_version(r#"{"bogus":"x"}"#).is_err());
        assert!(validate_account_address("celestia1abc").is_ok());
        assert!(validate_account_address("a-b").is_err());
        assert_eq!(
            decode_packet_data(br#"{"type":"TYPE_EXECUTE_TX","data":"AQ==","memo":""}"#),
            Ok((1, vec![1]))
        );
        assert_eq!(decode_packet_data(br#"{"type":1}"#), Ok((1, vec![])));
        assert!(decode_packet_data(br#"{"type":"NOPE"}"#).is_err());
    }
}
