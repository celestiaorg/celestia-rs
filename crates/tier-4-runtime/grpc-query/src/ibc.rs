//! ibc-go v8.7.2 queries: `ibc.core.client.v1`, `ibc.core.connection.v1`,
//! `ibc.core.channel.v1`, `ibc.applications.transfer.v1` and
//! `ibc.applications.interchain_accounts.host.v1`.

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::ibc::applications::interchain_accounts::host::v1 as icapb;
use celestia_proto::ibc::applications::transfer::v1 as trpb;
use celestia_proto::ibc::core::channel::v1 as chpb;
use celestia_proto::ibc::core::client::v1 as clpb;
use celestia_proto::ibc::core::connection::v1 as cnpb;
use clpb::Height;
use prost::Message;
use sdk::{Context, SdkError};
use x_ibc::{errors as ibcerr, host};

use crate::Router;
use crate::error::{Code, QResult, QueryError, invalid_arg, not_found};
use crate::pagination::{filtered_paginate, paginate};
use crate::util::{ACC, bech32, go_hex_decode, page, validate_denom};

const IBC: &str = x_ibc::STORE_KEY;

#[allow(clippy::many_single_char_names)]
pub(crate) fn register(r: &mut Router) {
    let c = "/ibc.core.client.v1.Query/";
    r.add(leak(c, "ClientState"), client_state);
    r.add(leak(c, "ClientStates"), client_states);
    r.add(leak(c, "ConsensusState"), consensus_state);
    r.add(leak(c, "ConsensusStates"), consensus_states);
    r.add(leak(c, "ConsensusStateHeights"), consensus_state_heights);
    r.add(leak(c, "ClientStatus"), client_status);
    r.add(leak(c, "ClientParams"), client_params);
    r.add(leak(c, "UpgradedClientState"), upgraded_client_state);
    r.add(leak(c, "UpgradedConsensusState"), upgraded_consensus_state);
    r.add(leak(c, "VerifyMembership"), verify_membership);
    let n = "/ibc.core.connection.v1.Query/";
    r.add(leak(n, "Connection"), connection);
    r.add(leak(n, "Connections"), connections);
    r.add(leak(n, "ClientConnections"), client_connections);
    r.add(leak(n, "ConnectionClientState"), connection_client_state);
    r.add(
        leak(n, "ConnectionConsensusState"),
        connection_consensus_state,
    );
    r.add(leak(n, "ConnectionParams"), connection_params);
    let h = "/ibc.core.channel.v1.Query/";
    r.add(leak(h, "Channel"), channel);
    r.add(leak(h, "Channels"), channels);
    r.add(leak(h, "ConnectionChannels"), connection_channels);
    r.add(leak(h, "ChannelClientState"), channel_client_state);
    r.add(leak(h, "ChannelConsensusState"), channel_consensus_state);
    r.add(leak(h, "PacketCommitment"), packet_commitment);
    r.add(leak(h, "PacketCommitments"), packet_commitments);
    r.add(leak(h, "PacketReceipt"), packet_receipt);
    r.add(leak(h, "PacketAcknowledgement"), packet_acknowledgement);
    r.add(leak(h, "PacketAcknowledgements"), packet_acknowledgements);
    r.add(leak(h, "UnreceivedPackets"), unreceived_packets);
    r.add(leak(h, "UnreceivedAcks"), unreceived_acks);
    r.add(leak(h, "NextSequenceReceive"), next_sequence_receive);
    r.add(leak(h, "NextSequenceSend"), next_sequence_send);
    r.add(leak(h, "UpgradeError"), upgrade_error);
    r.add(leak(h, "Upgrade"), upgrade);
    r.add(leak(h, "ChannelParams"), channel_params);
    let t = "/ibc.applications.transfer.v1.Query/";
    r.add(leak(t, "DenomTrace"), denom_trace);
    r.add(leak(t, "DenomTraces"), denom_traces);
    r.add(leak(t, "Params"), transfer_params);
    r.add(leak(t, "DenomHash"), denom_hash);
    r.add(leak(t, "EscrowAddress"), escrow_address);
    r.add(leak(t, "TotalEscrowForDenom"), total_escrow_for_denom);
    r.add(
        "/ibc.applications.interchain_accounts.host.v1.Query/Params",
        ica_host_params,
    );
}

fn leak(service: &str, method: &str) -> &'static str {
    Box::leak(format!("{service}{method}").into_boxed_str())
}

// ---------------------------------------------------------------------------
// Shared helpers.

/// `status.Error(codes.InvalidArgument, err.Error())` of a validator.
#[allow(clippy::needless_pass_by_value)]
fn arg(e: SdkError) -> QueryError {
    invalid_arg(e.log)
}

/// `status.Error(codes.NotFound, err.Error())`.
#[allow(clippy::needless_pass_by_value)]
fn nf(e: SdkError) -> QueryError {
    not_found(e.log)
}

/// An ibc keeper error: unregistered → plain, panic → panic.
#[allow(clippy::needless_pass_by_value)]
fn ke(e: SdkError) -> QueryError {
    match (e.codespace, e.code) {
        ("undefined", 1) => QueryError::Plain(e.log),
        ("undefined", 111_222) => QueryError::Panic(e.log),
        _ => QueryError::Sdk(e),
    }
}

fn self_height(ctx: &Context<'_>) -> QResult<Height> {
    x_ibc::client::get_self_height(ctx).map_err(ke)
}

fn decode<M: Message + Default>(b: &[u8]) -> QResult<M> {
    M::decode(b).map_err(|e| QueryError::Plain(e.to_string()))
}

fn get_raw(ctx: &mut Context<'_>, key: &[u8]) -> QResult<Option<Vec<u8>>> {
    Ok(ctx.get(IBC, key)?.filter(|b| !b.is_empty()))
}

fn client_state_any(ctx: &mut Context<'_>, client_id: &str) -> QResult<Option<Any>> {
    get_raw(ctx, &host::full_client_state_key(client_id))?
        .map(|b| decode::<Any>(&b))
        .transpose()
}

fn consensus_any(ctx: &mut Context<'_>, client_id: &str, h: &Height) -> QResult<Option<Any>> {
    get_raw(ctx, &host::full_consensus_state_key(client_id, h))?
        .map(|b| decode::<Any>(&b))
        .transpose()
}

fn get_connection(ctx: &mut Context<'_>, id: &str) -> QResult<Option<cnpb::ConnectionEnd>> {
    get_raw(ctx, &host::connection_key(id))?
        .map(|b| decode(&b))
        .transpose()
}

fn get_channel(ctx: &mut Context<'_>, port: &str, chan: &str) -> QResult<Option<chpb::Channel>> {
    get_raw(ctx, &host::channel_key(port, chan))?
        .map(|b| decode(&b))
        .transpose()
}

fn height_str(h: &Height) -> String {
    format!("{}-{}", h.revision_number, h.revision_height)
}

/// `validategRPCRequest`.
fn validate_port_channel(port: &str, chan: &str) -> QResult<()> {
    host::port_identifier_validator(port).map_err(arg)?;
    host::channel_identifier_validator(chan).map_err(arg)
}

fn channel_not_found(port: &str, chan: &str) -> QueryError {
    nf(ibcerr::channel::channel_not_found(format!(
        "port-id: {port}, channel-id {chan}"
    )))
}

fn must_channel(ctx: &mut Context<'_>, port: &str, chan: &str) -> QResult<chpb::Channel> {
    get_channel(ctx, port, chan)?.ok_or_else(|| channel_not_found(port, chan))
}

/// The client id of the channel's first connection hop.
fn channel_client(
    ctx: &mut Context<'_>,
    ch: &chpb::Channel,
) -> QResult<Result<String, (String, String)>> {
    let hop = ch.connection_hops.first().cloned().unwrap_or_default();
    Ok(match get_connection(ctx, &hop)? {
        Some(c) => Ok(c.client_id),
        None => Err((hop, String::new())),
    })
}

// ---------------------------------------------------------------------------
// ibc.core.client.v1

fn client_state(
    ctx: &mut Context<'_>,
    req: clpb::QueryClientStateRequest,
) -> QResult<clpb::QueryClientStateResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let cs = client_state_any(ctx, &req.client_id)?
        .ok_or_else(|| nf(ibcerr::client::client_not_found(&req.client_id)))?;
    Ok(clpb::QueryClientStateResponse {
        client_state: Some(cs),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn client_states(
    ctx: &mut Context<'_>,
    req: clpb::QueryClientStatesRequest,
) -> QResult<clpb::QueryClientStatesResponse> {
    let mut states = Vec::new();
    let pagination = filtered_paginate(
        ctx,
        IBC,
        host::KEY_CLIENT_STORE_PREFIX.as_bytes(),
        req.pagination.as_ref(),
        |_, k, v, _| {
            let key = String::from_utf8_lossy(k);
            let split: Vec<&str> = key.split('/').collect();
            if split.last() != Some(&host::KEY_CLIENT_STATE) {
                return Ok(false);
            }
            let cs = decode::<Any>(v)?;
            let id = split.get(1).copied().unwrap_or_default().to_owned();
            host::client_identifier_validator(&id).map_err(ke)?;
            states.push(clpb::IdentifiedClientState {
                client_id: id,
                client_state: Some(cs),
            });
            Ok(true)
        },
    )?;
    states.sort_by(|a, b| a.client_id.cmp(&b.client_id));
    Ok(clpb::QueryClientStatesResponse {
        client_states: states,
        pagination: page(pagination),
    })
}

fn consensus_state(
    ctx: &mut Context<'_>,
    req: clpb::QueryConsensusStateRequest,
) -> QResult<clpb::QueryConsensusStateResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let height = Height {
        revision_number: req.revision_number,
        revision_height: req.revision_height,
    };
    let cs = if req.latest_height {
        match client_state_any(ctx, &req.client_id)? {
            None => None,
            Some(any) => {
                let latest = x_ibc::client::latest_height_of(&any).map_err(ke)?;
                consensus_any(ctx, &req.client_id, &latest)?
            }
        }
    } else {
        if req.revision_height == 0 {
            return Err(invalid_arg("consensus state height cannot be 0"));
        }
        consensus_any(ctx, &req.client_id, &height)?
    };
    let cs = cs.ok_or_else(|| {
        nf(ibcerr::client::consensus_state_not_found(format!(
            "client-id: {}, height: {}",
            req.client_id,
            height_str(&height)
        )))
    })?;
    Ok(clpb::QueryConsensusStateResponse {
        consensus_state: Some(cs),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn consensus_prefix(client_id: &str) -> Vec<u8> {
    format!(
        "{}/{client_id}/{}/",
        host::KEY_CLIENT_STORE_PREFIX,
        host::KEY_CONSENSUS_STATE_PREFIX
    )
    .into_bytes()
}

fn consensus_states(
    ctx: &mut Context<'_>,
    req: clpb::QueryConsensusStatesRequest,
) -> QResult<clpb::QueryConsensusStatesResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let mut out = Vec::new();
    let pagination = filtered_paginate(
        ctx,
        IBC,
        &consensus_prefix(&req.client_id),
        req.pagination.as_ref(),
        |_, k, v, _| {
            if k.contains(&b'/') {
                return Ok(false);
            }
            let height = x_ibc::height::parse(&String::from_utf8_lossy(k)).map_err(ke)?;
            out.push(clpb::ConsensusStateWithHeight {
                height: Some(height),
                consensus_state: Some(decode::<Any>(v)?),
            });
            Ok(true)
        },
    )?;
    Ok(clpb::QueryConsensusStatesResponse {
        consensus_states: out,
        pagination: page(pagination),
    })
}

fn consensus_state_heights(
    ctx: &mut Context<'_>,
    req: clpb::QueryConsensusStateHeightsRequest,
) -> QResult<clpb::QueryConsensusStateHeightsResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let mut out = Vec::new();
    let pagination = filtered_paginate(
        ctx,
        IBC,
        &consensus_prefix(&req.client_id),
        req.pagination.as_ref(),
        |_, k, _, _| {
            if k.contains(&b'/') {
                return Ok(false);
            }
            out.push(x_ibc::height::parse(&String::from_utf8_lossy(k)).map_err(ke)?);
            Ok(true)
        },
    )?;
    Ok(clpb::QueryConsensusStateHeightsResponse {
        consensus_state_heights: out,
        pagination: page(pagination),
    })
}

fn client_status(
    ctx: &mut Context<'_>,
    req: clpb::QueryClientStatusRequest,
) -> QResult<clpb::QueryClientStatusResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let cs = x_ibc::client::client_state(ctx, &req.client_id)
        .map_err(ke)?
        .ok_or_else(|| nf(ibcerr::client::client_not_found(&req.client_id)))?;
    let status = x_ibc::client::client_status(ctx, &req.client_id, &cs).map_err(ke)?;
    Ok(clpb::QueryClientStatusResponse {
        status: status.as_str().to_owned(),
    })
}

fn client_params(
    ctx: &mut Context<'_>,
    _: clpb::QueryClientParamsRequest,
) -> QResult<clpb::QueryClientParamsResponse> {
    Ok(clpb::QueryClientParamsResponse {
        params: Some(x_ibc::client::get_params(ctx).map_err(ke)?),
    })
}

fn upgraded_client_state(
    ctx: &mut Context<'_>,
    _: clpb::QueryUpgradedClientStateRequest,
) -> QResult<clpb::QueryUpgradedClientStateResponse> {
    let plan = x_upgrade::get_upgrade_plan(ctx)
        .map_err(ke)?
        .ok_or_else(|| not_found("upgrade plan not found"))?;
    let bz = ctx
        .get(
            x_upgrade::STORE_KEY,
            &x_upgrade::upgraded_client_key(plan.height),
        )?
        .filter(|b| !b.is_empty())
        .ok_or_else(|| not_found("upgraded client not found"))?;
    let any =
        decode::<Any>(&bz).map_err(|e| QueryError::Status(Code::Internal, e.go_error_string()))?;
    Ok(clpb::QueryUpgradedClientStateResponse {
        upgraded_client_state: Some(any),
    })
}

fn upgraded_consensus_state(
    ctx: &mut Context<'_>,
    _: clpb::QueryUpgradedConsensusStateRequest,
) -> QResult<clpb::QueryUpgradedConsensusStateResponse> {
    let height = ctx.height();
    let bz = ctx
        .get(
            x_upgrade::STORE_KEY,
            &x_upgrade::upgraded_cons_state_key(height),
        )?
        .filter(|b| !b.is_empty())
        .ok_or_else(|| {
            not_found(format!(
                "upgraded consensus state not found, height {height}"
            ))
        })?;
    let any =
        decode::<Any>(&bz).map_err(|e| QueryError::Status(Code::Internal, e.go_error_string()))?;
    Ok(clpb::QueryUpgradedConsensusStateResponse {
        upgraded_consensus_state: Some(any),
    })
}

/// `ParseClientIdentifier`: the client type.
fn parse_client_type(id: &str) -> Result<String, SdkError> {
    if id == x_ibc::client::LOCALHOST_CLIENT_ID {
        return Ok(x_ibc::client::LOCALHOST.to_owned());
    }
    let split: Vec<&str> = id.split('-').collect();
    if split.len() < 2 {
        return Err(host::err_invalid_id(&format!(
            "invalid client identifier {id} is not in format: `{{client-type}}-{{N}}`"
        )));
    }
    let ty = split[..split.len() - 1].join("-");
    if ty.trim().is_empty() {
        return Err(host::err_invalid_id(
            "client identifier must be in format: `{client-type}-{N}` and client type cannot be blank",
        ));
    }
    let seq = split[split.len() - 1];
    if x_ibc::height::parse_u64(seq).is_none() {
        return Err(SdkError::new(
            "undefined",
            1,
            format!(
                "failed to parse client identifier sequence: strconv.ParseUint: parsing \"{seq}\": invalid syntax"
            ),
        ));
    }
    Ok(ty)
}

fn verify_membership(
    ctx: &mut Context<'_>,
    req: clpb::QueryVerifyMembershipRequest,
) -> QResult<clpb::QueryVerifyMembershipResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let ty = parse_client_type(&req.client_id).map_err(arg)?;
    if ty == x_ibc::client::LOCALHOST || ty == x_ibc::client::SOLOMACHINE {
        return Err(invalid_arg(
            ibcerr::client::invalid_client_type(
                "verify membership is disabled for client types [09-localhost 06-solomachine]",
            )
            .log,
        ));
    }
    if req.proof.is_empty() {
        return Err(invalid_arg("empty proof"));
    }
    let proof_height = req.proof_height.unwrap_or_default();
    if x_ibc::height::is_zero(&proof_height) {
        return Err(invalid_arg("proof height must be non-zero"));
    }
    let path = req.merkle_path.clone().unwrap_or_default();
    if path.key_path.is_empty() {
        return Err(invalid_arg("empty merkle path"));
    }
    if req.value.is_empty() {
        return Err(invalid_arg("empty value"));
    }
    let cs = x_ibc::client::client_state(ctx, &req.client_id)
        .map_err(ke)?
        .ok_or_else(|| nf(ibcerr::client::client_not_found(&req.client_id)))?;
    let status = x_ibc::client::client_status(ctx, &req.client_id, &cs).map_err(ke)?;
    if status != x_ibc::client::Status::Active {
        return Err(QueryError::Status(
            Code::FailedPrecondition,
            ibcerr::client::client_not_active(format!(
                "cannot verify membership using client ({}) with status {}",
                req.client_id,
                status.as_str()
            ))
            .log,
        ));
    }
    let ok = x_ibc::client::verify_membership(
        ctx,
        &req.client_id,
        &cs,
        &proof_height,
        req.time_delay,
        req.block_delay,
        &req.proof,
        &path,
        Some(&req.value),
    )
    .is_ok();
    Ok(clpb::QueryVerifyMembershipResponse { success: ok })
}

// ---------------------------------------------------------------------------
// ibc.core.connection.v1

fn connection(
    ctx: &mut Context<'_>,
    req: cnpb::QueryConnectionRequest,
) -> QResult<cnpb::QueryConnectionResponse> {
    host::connection_identifier_validator(&req.connection_id).map_err(arg)?;
    let c = get_connection(ctx, &req.connection_id)?
        .ok_or_else(|| nf(ibcerr::connection::connection_not_found(&req.connection_id)))?;
    Ok(cnpb::QueryConnectionResponse {
        connection: Some(c),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn connections(
    ctx: &mut Context<'_>,
    req: cnpb::QueryConnectionsRequest,
) -> QResult<cnpb::QueryConnectionsResponse> {
    let mut out = Vec::new();
    let pagination = paginate(
        ctx,
        IBC,
        host::KEY_CONNECTION_PREFIX.as_bytes(),
        req.pagination.as_ref(),
        |_, k, v| {
            let c: cnpb::ConnectionEnd = decode(v)?;
            let path = String::from_utf8_lossy(k);
            let split: Vec<&str> = path.split('/').collect();
            if split.len() != 2 {
                return Err(QueryError::Sdk(ibcerr::host::invalid_path(format!(
                    "cannot parse connection path {path}"
                ))));
            }
            out.push(cnpb::IdentifiedConnection {
                id: split[1].to_owned(),
                client_id: c.client_id,
                versions: c.versions,
                state: c.state,
                counterparty: c.counterparty,
                delay_period: c.delay_period,
            });
            Ok(())
        },
    )?;
    Ok(cnpb::QueryConnectionsResponse {
        connections: out,
        pagination: page(pagination),
        height: Some(self_height(ctx)?),
    })
}

fn client_connections(
    ctx: &mut Context<'_>,
    req: cnpb::QueryClientConnectionsRequest,
) -> QResult<cnpb::QueryClientConnectionsResponse> {
    host::client_identifier_validator(&req.client_id).map_err(arg)?;
    let paths = x_ibc::connection::get_client_connection_paths(ctx, &req.client_id)
        .map_err(ke)?
        .ok_or_else(|| {
            nf(ibcerr::connection::client_connection_paths_not_found(
                &req.client_id,
            ))
        })?;
    Ok(cnpb::QueryClientConnectionsResponse {
        connection_paths: paths,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn connection_client_state(
    ctx: &mut Context<'_>,
    req: cnpb::QueryConnectionClientStateRequest,
) -> QResult<cnpb::QueryConnectionClientStateResponse> {
    host::connection_identifier_validator(&req.connection_id).map_err(arg)?;
    let c = get_connection(ctx, &req.connection_id)?.ok_or_else(|| {
        nf(ibcerr::connection::connection_not_found(format!(
            "connection-id: {}",
            req.connection_id
        )))
    })?;
    let cs = client_state_any(ctx, &c.client_id)?.ok_or_else(|| {
        nf(ibcerr::client::client_not_found(format!(
            "client-id: {}",
            c.client_id
        )))
    })?;
    Ok(cnpb::QueryConnectionClientStateResponse {
        identified_client_state: Some(clpb::IdentifiedClientState {
            client_id: c.client_id,
            client_state: Some(cs),
        }),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn connection_consensus_state(
    ctx: &mut Context<'_>,
    req: cnpb::QueryConnectionConsensusStateRequest,
) -> QResult<cnpb::QueryConnectionConsensusStateResponse> {
    host::connection_identifier_validator(&req.connection_id).map_err(arg)?;
    let c = get_connection(ctx, &req.connection_id)?.ok_or_else(|| {
        nf(ibcerr::connection::connection_not_found(format!(
            "connection-id: {}",
            req.connection_id
        )))
    })?;
    let height = Height {
        revision_number: req.revision_number,
        revision_height: req.revision_height,
    };
    let cs = consensus_any(ctx, &c.client_id, &height)?.ok_or_else(|| {
        nf(ibcerr::client::consensus_state_not_found(format!(
            "client-id: {}",
            c.client_id
        )))
    })?;
    Ok(cnpb::QueryConnectionConsensusStateResponse {
        consensus_state: Some(cs),
        client_id: c.client_id,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn connection_params(
    ctx: &mut Context<'_>,
    _: cnpb::QueryConnectionParamsRequest,
) -> QResult<cnpb::QueryConnectionParamsResponse> {
    Ok(cnpb::QueryConnectionParamsResponse {
        params: Some(x_ibc::connection::get_params(ctx).map_err(ke)?),
    })
}

// ---------------------------------------------------------------------------
// ibc.core.channel.v1

fn channel(
    ctx: &mut Context<'_>,
    req: chpb::QueryChannelRequest,
) -> QResult<chpb::QueryChannelResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let ch = must_channel(ctx, &req.port_id, &req.channel_id)?;
    Ok(chpb::QueryChannelResponse {
        channel: Some(ch),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

/// `host.ParseChannelPath` of a key under `channelEnds`.
fn parse_channel_path(path: &str) -> QResult<(String, String)> {
    let split: Vec<&str> = path.split('/').collect();
    if split.len() < 5 || split[1] != host::KEY_PORT_PREFIX || split[3] != host::KEY_CHANNEL_PREFIX
    {
        return Err(QueryError::Sdk(ibcerr::host::invalid_path(format!(
            "cannot parse channel path {path}"
        ))));
    }
    Ok((split[2].to_owned(), split[4].to_owned()))
}

fn identified(port: String, chan: String, ch: chpb::Channel) -> chpb::IdentifiedChannel {
    chpb::IdentifiedChannel {
        state: ch.state,
        ordering: ch.ordering,
        counterparty: ch.counterparty,
        connection_hops: ch.connection_hops,
        version: ch.version,
        port_id: port,
        channel_id: chan,
        upgrade_sequence: ch.upgrade_sequence,
    }
}

fn channels(
    ctx: &mut Context<'_>,
    req: chpb::QueryChannelsRequest,
) -> QResult<chpb::QueryChannelsResponse> {
    let mut out = Vec::new();
    let pagination = paginate(
        ctx,
        IBC,
        host::KEY_CHANNEL_END_PREFIX.as_bytes(),
        req.pagination.as_ref(),
        |_, k, v| {
            let ch: chpb::Channel = decode(v)?;
            let (port, chan) = parse_channel_path(&String::from_utf8_lossy(k))?;
            out.push(identified(port, chan, ch));
            Ok(())
        },
    )?;
    Ok(chpb::QueryChannelsResponse {
        channels: out,
        pagination: page(pagination),
        height: Some(self_height(ctx)?),
    })
}

fn connection_channels(
    ctx: &mut Context<'_>,
    req: chpb::QueryConnectionChannelsRequest,
) -> QResult<chpb::QueryConnectionChannelsResponse> {
    host::connection_identifier_validator(&req.connection).map_err(arg)?;
    let mut out = Vec::new();
    let pagination = filtered_paginate(
        ctx,
        IBC,
        host::KEY_CHANNEL_END_PREFIX.as_bytes(),
        req.pagination.as_ref(),
        |_, k, v, _| {
            let ch: chpb::Channel = decode(v)?;
            let Some(hop) = ch.connection_hops.first() else {
                return Err(QueryError::Panic(
                    "runtime error: index out of range [0] with length 0".into(),
                ));
            };
            if *hop != req.connection {
                return Ok(false);
            }
            let (port, chan) = parse_channel_path(&String::from_utf8_lossy(k))?;
            out.push(identified(port, chan, ch));
            Ok(true)
        },
    )?;
    Ok(chpb::QueryConnectionChannelsResponse {
        channels: out,
        pagination: page(pagination),
        height: Some(self_height(ctx)?),
    })
}

fn channel_client_state(
    ctx: &mut Context<'_>,
    req: chpb::QueryChannelClientStateRequest,
) -> QResult<chpb::QueryChannelClientStateResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let ch = get_channel(ctx, &req.port_id, &req.channel_id)?.ok_or_else(|| {
        nf(ibcerr::channel::channel_not_found(format!(
            "port-id: {}, channel-id: {}",
            req.port_id, req.channel_id
        )))
    })?;
    let client_id = channel_client(ctx, &ch)?.map_err(|(hop, _)| {
        nf(ibcerr::connection::connection_not_found(format!(
            "connection-id: {hop}"
        )))
    })?;
    let cs = client_state_any(ctx, &client_id)?.ok_or_else(|| {
        nf(ibcerr::client::client_not_found(format!(
            "client-id: {client_id}"
        )))
    })?;
    Ok(chpb::QueryChannelClientStateResponse {
        identified_client_state: Some(clpb::IdentifiedClientState {
            client_id,
            client_state: Some(cs),
        }),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn channel_consensus_state(
    ctx: &mut Context<'_>,
    req: chpb::QueryChannelConsensusStateRequest,
) -> QResult<chpb::QueryChannelConsensusStateResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let ch = must_channel(ctx, &req.port_id, &req.channel_id)?;
    let client_id = channel_client(ctx, &ch)?.map_err(|(hop, _)| {
        nf(ibcerr::connection::connection_not_found(format!(
            "connection-id: {hop}"
        )))
    })?;
    let height = Height {
        revision_number: req.revision_number,
        revision_height: req.revision_height,
    };
    let cs = consensus_any(ctx, &client_id, &height)?.ok_or_else(|| {
        nf(ibcerr::client::consensus_state_not_found(format!(
            "client-id: {client_id}"
        )))
    })?;
    Ok(chpb::QueryChannelConsensusStateResponse {
        consensus_state: Some(cs),
        client_id,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn packet_commitment(
    ctx: &mut Context<'_>,
    req: chpb::QueryPacketCommitmentRequest,
) -> QResult<chpb::QueryPacketCommitmentResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    if req.sequence == 0 {
        return Err(invalid_arg("packet sequence cannot be 0"));
    }
    let c = x_ibc::channel::get_packet_commitment(ctx, &req.port_id, &req.channel_id, req.sequence)
        .map_err(ke)?;
    if c.is_empty() {
        return Err(not_found("packet commitment hash not found"));
    }
    Ok(chpb::QueryPacketCommitmentResponse {
        commitment: c,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

/// Packet states under `<kind>/ports/<p>/channels/<c>/sequences`.
fn packet_states(
    ctx: &mut Context<'_>,
    kind: &str,
    port: &str,
    chan: &str,
    page_req: Option<&celestia_proto::cosmos::base::query::v1beta1::PageRequest>,
) -> QResult<(
    Vec<chpb::PacketState>,
    celestia_proto::cosmos::base::query::v1beta1::PageResponse,
)> {
    let prefix = format!(
        "{kind}/{}/{port}/{}/{chan}/{}",
        host::KEY_PORT_PREFIX,
        host::KEY_CHANNEL_PREFIX,
        host::KEY_SEQUENCE_PREFIX
    );
    let mut out = Vec::new();
    let pagination = paginate(ctx, IBC, prefix.as_bytes(), page_req, |_, k, v| {
        let key = String::from_utf8_lossy(k);
        let last = key.rsplit('/').next().unwrap_or_default();
        let seq = x_ibc::height::parse_u64(last).ok_or_else(|| {
            QueryError::Plain(format!(
                "strconv.ParseUint: parsing \"{last}\": invalid syntax"
            ))
        })?;
        out.push(chpb::PacketState {
            port_id: port.to_owned(),
            channel_id: chan.to_owned(),
            sequence: seq,
            data: v.to_vec(),
        });
        Ok(())
    })?;
    Ok((out, pagination))
}

fn packet_commitments(
    ctx: &mut Context<'_>,
    req: chpb::QueryPacketCommitmentsRequest,
) -> QResult<chpb::QueryPacketCommitmentsResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let (commitments, pagination) = packet_states(
        ctx,
        host::KEY_PACKET_COMMITMENT_PREFIX,
        &req.port_id,
        &req.channel_id,
        req.pagination.as_ref(),
    )?;
    Ok(chpb::QueryPacketCommitmentsResponse {
        commitments,
        pagination: page(pagination),
        height: Some(self_height(ctx)?),
    })
}

fn packet_receipt(
    ctx: &mut Context<'_>,
    req: chpb::QueryPacketReceiptRequest,
) -> QResult<chpb::QueryPacketReceiptResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    if req.sequence == 0 {
        return Err(invalid_arg("packet sequence cannot be 0"));
    }
    let received =
        x_ibc::channel::has_packet_receipt(ctx, &req.port_id, &req.channel_id, req.sequence)
            .map_err(ke)?;
    Ok(chpb::QueryPacketReceiptResponse {
        received,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn get_ack(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> QResult<Vec<u8>> {
    Ok(ctx
        .get(IBC, &host::packet_acknowledgement_key(port, chan, seq))?
        .unwrap_or_default())
}

fn packet_acknowledgement(
    ctx: &mut Context<'_>,
    req: chpb::QueryPacketAcknowledgementRequest,
) -> QResult<chpb::QueryPacketAcknowledgementResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    if req.sequence == 0 {
        return Err(invalid_arg("packet sequence cannot be 0"));
    }
    let ack = get_ack(ctx, &req.port_id, &req.channel_id, req.sequence)?;
    if ack.is_empty() {
        return Err(not_found("packet acknowledgement hash not found"));
    }
    Ok(chpb::QueryPacketAcknowledgementResponse {
        acknowledgement: ack,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn packet_acknowledgements(
    ctx: &mut Context<'_>,
    req: chpb::QueryPacketAcknowledgementsRequest,
) -> QResult<chpb::QueryPacketAcknowledgementsResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    if !req.packet_commitment_sequences.is_empty() {
        let mut acks = Vec::new();
        for &seq in &req.packet_commitment_sequences {
            let ack = get_ack(ctx, &req.port_id, &req.channel_id, seq)?;
            if !ack.is_empty() {
                acks.push(chpb::PacketState {
                    port_id: req.port_id.clone(),
                    channel_id: req.channel_id.clone(),
                    sequence: seq,
                    data: ack,
                });
            }
        }
        return Ok(chpb::QueryPacketAcknowledgementsResponse {
            acknowledgements: acks,
            pagination: None,
            height: Some(self_height(ctx)?),
        });
    }
    let (acknowledgements, pagination) = packet_states(
        ctx,
        host::KEY_PACKET_ACK_PREFIX,
        &req.port_id,
        &req.channel_id,
        req.pagination.as_ref(),
    )?;
    Ok(chpb::QueryPacketAcknowledgementsResponse {
        acknowledgements,
        pagination: page(pagination),
        height: Some(self_height(ctx)?),
    })
}

fn unreceived_packets(
    ctx: &mut Context<'_>,
    req: chpb::QueryUnreceivedPacketsRequest,
) -> QResult<chpb::QueryUnreceivedPacketsResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let ch = must_channel(ctx, &req.port_id, &req.channel_id)?;
    let mut seqs = Vec::new();
    let zero = |i: usize| invalid_arg(format!("packet sequence {i} cannot be 0"));
    if ch.ordering == chpb::Order::Unordered as i32 {
        for (i, &seq) in req.packet_commitment_sequences.iter().enumerate() {
            if seq == 0 {
                return Err(zero(i));
            }
            if !x_ibc::channel::has_packet_receipt(ctx, &req.port_id, &req.channel_id, seq)
                .map_err(ke)?
            {
                seqs.push(seq);
            }
        }
    } else if ch.ordering == chpb::Order::Ordered as i32 {
        let next = x_ibc::channel::get_next_sequence_recv(ctx, &req.port_id, &req.channel_id)
            .map_err(ke)?
            .ok_or_else(|| {
                nf(ibcerr::channel::sequence_receive_not_found(format!(
                    "destination port: {}, destination channel: {}",
                    req.port_id, req.channel_id
                )))
            })?;
        for (i, &seq) in req.packet_commitment_sequences.iter().enumerate() {
            if seq == 0 {
                return Err(zero(i));
            }
            if seq >= next {
                seqs.push(seq);
            }
        }
    } else {
        return Err(invalid_arg(
            ibcerr::channel::invalid_channel_ordering(format!(
                "channel order {} is not supported",
                x_ibc::channel::order_name(ch.ordering)
            ))
            .log,
        ));
    }
    Ok(chpb::QueryUnreceivedPacketsResponse {
        sequences: seqs,
        height: Some(self_height(ctx)?),
    })
}

fn unreceived_acks(
    ctx: &mut Context<'_>,
    req: chpb::QueryUnreceivedAcksRequest,
) -> QResult<chpb::QueryUnreceivedAcksResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let mut seqs = Vec::new();
    for (i, &seq) in req.packet_ack_sequences.iter().enumerate() {
        if seq == 0 {
            return Err(invalid_arg(format!("packet sequence {i} cannot be 0")));
        }
        if !x_ibc::channel::get_packet_commitment(ctx, &req.port_id, &req.channel_id, seq)
            .map_err(ke)?
            .is_empty()
        {
            seqs.push(seq);
        }
    }
    Ok(chpb::QueryUnreceivedAcksResponse {
        sequences: seqs,
        height: Some(self_height(ctx)?),
    })
}

fn next_sequence_receive(
    ctx: &mut Context<'_>,
    req: chpb::QueryNextSequenceReceiveRequest,
) -> QResult<chpb::QueryNextSequenceReceiveResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let ch = must_channel(ctx, &req.port_id, &req.channel_id)?;
    let mut seq = 0;
    if ch.ordering != chpb::Order::Unordered as i32 {
        seq = x_ibc::channel::get_next_sequence_recv(ctx, &req.port_id, &req.channel_id)
            .map_err(ke)?
            .ok_or_else(|| {
                nf(ibcerr::channel::sequence_receive_not_found(format!(
                    "port-id: {}, channel-id {}",
                    req.port_id, req.channel_id
                )))
            })?;
    }
    Ok(chpb::QueryNextSequenceReceiveResponse {
        next_sequence_receive: seq,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn next_sequence_send(
    ctx: &mut Context<'_>,
    req: chpb::QueryNextSequenceSendRequest,
) -> QResult<chpb::QueryNextSequenceSendResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    let seq = x_ibc::channel::get_next_sequence_send(ctx, &req.port_id, &req.channel_id)
        .map_err(ke)?
        .ok_or_else(|| {
            nf(ibcerr::channel::sequence_send_not_found(format!(
                "port-id: {}, channel-id {}",
                req.port_id, req.channel_id
            )))
        })?;
    Ok(chpb::QueryNextSequenceSendResponse {
        next_sequence_send: seq,
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn upgrade_key(kind: &str, port: &str, chan: &str) -> Vec<u8> {
    format!("channelUpgrades/{kind}/ports/{port}/channels/{chan}").into_bytes()
}

fn upgrade_error(
    ctx: &mut Context<'_>,
    req: chpb::QueryUpgradeErrorRequest,
) -> QResult<chpb::QueryUpgradeErrorResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    must_channel(ctx, &req.port_id, &req.channel_id)?;
    let receipt: chpb::ErrorReceipt = get_raw(
        ctx,
        &upgrade_key("upgradeError", &req.port_id, &req.channel_id),
    )?
    .map(|b| decode(&b))
    .transpose()?
    .ok_or_else(|| {
        nf(ibcerr::channel::upgrade_error_not_found(format!(
            "port-id {}, channel-id {}",
            req.port_id, req.channel_id
        )))
    })?;
    Ok(chpb::QueryUpgradeErrorResponse {
        error_receipt: Some(receipt),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn upgrade(
    ctx: &mut Context<'_>,
    req: chpb::QueryUpgradeRequest,
) -> QResult<chpb::QueryUpgradeResponse> {
    validate_port_channel(&req.port_id, &req.channel_id)?;
    must_channel(ctx, &req.port_id, &req.channel_id)?;
    let up: chpb::Upgrade = get_raw(ctx, &upgrade_key("upgrades", &req.port_id, &req.channel_id))?
        .map(|b| decode(&b))
        .transpose()?
        .ok_or_else(|| {
            nf(ibcerr::channel::upgrade_not_found(format!(
                "port-id: {}, channel-id {}",
                req.port_id, req.channel_id
            )))
        })?;
    Ok(chpb::QueryUpgradeResponse {
        upgrade: Some(up),
        proof: Vec::new(),
        proof_height: Some(self_height(ctx)?),
    })
}

fn channel_params(
    ctx: &mut Context<'_>,
    _: chpb::QueryChannelParamsRequest,
) -> QResult<chpb::QueryChannelParamsResponse> {
    Ok(chpb::QueryChannelParamsResponse {
        params: Some(x_ibc::channel::get_params(ctx).map_err(ke)?),
    })
}

// ---------------------------------------------------------------------------
// ibc.applications.transfer.v1

/// `ParseHexHash`: Go hex, then `ValidateHash` (empty or 32 bytes).
fn parse_hex_hash(s: &str) -> Result<Vec<u8>, String> {
    let b = go_hex_decode(s)?;
    if !b.is_empty() && b.len() != 32 {
        return Err(format!(
            "expected size to be 32 bytes, got {} bytes",
            b.len()
        ));
    }
    Ok(b)
}

fn upper_hex(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02X}");
        s
    })
}

fn denom_trace(
    ctx: &mut Context<'_>,
    req: trpb::QueryDenomTraceRequest,
) -> QResult<trpb::QueryDenomTraceResponse> {
    let hash = parse_hex_hash(req.hash.strip_prefix("ibc/").unwrap_or(&req.hash))
        .map_err(|e| invalid_arg(format!("invalid denom trace hash: , error: {e}")))?;
    let t = x_transfer::get_denom_trace(ctx, &hash)
        .map_err(ke)?
        .ok_or_else(|| nf(transfer_trace_not_found(&req.hash)))?;
    Ok(trpb::QueryDenomTraceResponse {
        denom_trace: Some(t),
    })
}

/// `ErrTraceNotFound` (`transfer` codespace, code 6).
fn transfer_trace_not_found(log: &str) -> SdkError {
    SdkError::new(
        "transfer",
        6,
        format!("{log}: denomination trace not found"),
    )
}

fn denom_traces(
    ctx: &mut Context<'_>,
    req: trpb::QueryDenomTracesRequest,
) -> QResult<trpb::QueryDenomTracesResponse> {
    let mut traces: Vec<trpb::DenomTrace> = Vec::new();
    let pagination = paginate(
        ctx,
        x_transfer::STORE_KEY,
        x_transfer::DENOM_TRACE_KEY,
        req.pagination.as_ref(),
        |_, _, v| {
            traces.push(decode(v)?);
            Ok(())
        },
    )?;
    traces.sort_by_key(x_transfer::full_denom_path);
    Ok(trpb::QueryDenomTracesResponse {
        denom_traces: traces,
        pagination: page(pagination),
    })
}

fn transfer_params(
    ctx: &mut Context<'_>,
    _: trpb::QueryParamsRequest,
) -> QResult<trpb::QueryParamsResponse> {
    Ok(trpb::QueryParamsResponse {
        params: Some(x_transfer::get_params(ctx).map_err(ke)?),
    })
}

/// `validateTraceIdentifiers` (plain errors and wrapped host errors).
fn validate_trace_identifiers(ids: &[&str]) -> Result<(), String> {
    if ids.is_empty() || !ids.len().is_multiple_of(2) {
        return Err(format!(
            "trace info must come in pairs of port and channel identifiers '{{portID}}/{{channelID}}', got the identifiers: [{}]",
            ids.join(" ")
        ));
    }
    for i in (0..ids.len()).step_by(2) {
        host::port_identifier_validator(ids[i])
            .map_err(|e| format!("invalid port ID at position {i}: {}", e.log))?;
        host::channel_identifier_validator(ids[i + 1])
            .map_err(|e| format!("invalid channel ID at position {i}: {}", e.log))?;
    }
    Ok(())
}

/// `DenomTrace.Validate`.
fn validate_trace(t: &trpb::DenomTrace) -> Result<(), String> {
    if t.path.is_empty() && !t.base_denom.is_empty() {
        return Ok(());
    }
    if t.base_denom.trim().is_empty() {
        return Err("base denomination cannot be blank".into());
    }
    let ids: Vec<&str> = t.path.split('/').collect();
    validate_trace_identifiers(&ids)
}

fn denom_hash(
    ctx: &mut Context<'_>,
    req: trpb::QueryDenomHashRequest,
) -> QResult<trpb::QueryDenomHashResponse> {
    let t = x_transfer::relay::parse_denom_trace(&req.trace);
    validate_trace(&t).map_err(invalid_arg)?;
    let hash = x_transfer::denom_trace_hash(&t);
    let key = [x_transfer::DENOM_TRACE_KEY, hash.as_slice()].concat();
    if !ctx.has(x_transfer::STORE_KEY, &key)? {
        return Err(nf(transfer_trace_not_found(&req.trace)));
    }
    Ok(trpb::QueryDenomHashResponse {
        hash: upper_hex(&hash),
    })
}

fn escrow_address(
    _: &mut Context<'_>,
    req: trpb::QueryEscrowAddressRequest,
) -> QResult<trpb::QueryEscrowAddressResponse> {
    Ok(trpb::QueryEscrowAddressResponse {
        escrow_address: bech32(
            ACC,
            &x_transfer::escrow_address(&req.port_id, &req.channel_id),
        ),
    })
}

fn total_escrow_for_denom(
    ctx: &mut Context<'_>,
    req: trpb::QueryTotalEscrowForDenomRequest,
) -> QResult<trpb::QueryTotalEscrowForDenomResponse> {
    validate_denom(&req.denom)?;
    let c = x_transfer::get_total_escrow_for_denom(ctx, &req.denom).map_err(ke)?;
    Ok(trpb::QueryTotalEscrowForDenomResponse {
        amount: Some(PbCoin {
            denom: c.denom,
            amount: c.amount,
        }),
    })
}

fn ica_host_params(
    ctx: &mut Context<'_>,
    _: icapb::QueryParamsRequest,
) -> QResult<icapb::QueryParamsResponse> {
    Ok(icapb::QueryParamsResponse {
        params: Some(x_icahost::get_params(ctx).map_err(ke)?),
    })
}
