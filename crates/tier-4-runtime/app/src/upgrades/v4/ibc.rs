//! ibc core 2 to 6 (ibc-go v8.7.0 `modules/core/module.go` migrations).

use celestia_proto::ibc::core::channel::v1::{Params as ChannelParams, Timeout};
use celestia_proto::ibc::core::client::v1::{Height, Params as ClientParams};
use celestia_proto::ibc::core::connection::v1::Params as ConnectionParams;
use celestia_proto::ibc::lightclients::solomachine::{v2, v3};
use celestia_proto::ibc::lightclients::tendermint::v1::ClientState as TmClientState;
use prost::Message as _;
use prost_types::Any as ProtoAny;
use sdk::{Context, Result};

/// `channeltypes.DefaultParams`: an upgrade timeout of 10 minutes.
const DEFAULT_UPGRADE_TIMEOUT_NANOS: u64 = 600_000_000_000;

fn internal(e: impl std::fmt::Display) -> sdk::SdkError {
    sdk::error::internal(e.to_string())
}

fn prefixed(ctx: &mut Context<'_>, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut it = ctx.prefix_iter(x_ibc::STORE_KEY, prefix)?;
    while let Some(kv) = it.next(ctx)? {
        out.push(kv);
    }
    Ok(out)
}

/// 02-client v7 `collectClients`: the client IDs whose `clients/<type>…`
/// keys hold a client state, stopping at the first Tendermint client.
fn collect_clients(ctx: &mut Context<'_>, client_type: &str) -> Result<Vec<String>> {
    let mut clients = Vec::new();
    for (k, _) in prefixed(ctx, format!("clients/{client_type}").as_bytes())? {
        let path = String::from_utf8_lossy(&k).into_owned();
        if !path.contains("clientState") {
            continue;
        }
        // `MustParseClientStatePath`: `clients/<id>/clientState`.
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 3 || parts[0] != "clients" || parts[2] != "clientState" {
            return Err(internal(format!("cannot parse client state path {path}")));
        }
        let id = parts[1].to_owned();
        let tendermint = id.contains("07-tendermint");
        clients.push(id);
        if tendermint {
            break;
        }
    }
    Ok(clients)
}

/// `removeAllClientConsensusStates`: every `consensusStates/<height>` key
/// of the client store (not the metadata keys below a height).
fn remove_consensus_states(ctx: &mut Context<'_>, client_id: &str) -> Result<()> {
    let store = format!("clients/{client_id}/");
    for (k, _) in prefixed(ctx, format!("{store}consensusStates").as_bytes())? {
        let rest = String::from_utf8_lossy(&k[store.len()..]).into_owned();
        let split: Vec<&str> = rest.split('/').collect();
        if split.len() == 2 && split[0] == "consensusStates" {
            ctx.delete(x_ibc::STORE_KEY, &k)?;
        }
    }
    Ok(())
}

/// ibc core 2 to 3 (02-client `v7.MigrateStore`): solo machine client
/// states from v2 to v3 (dropping `allow_update_after_proposal`) with their
/// consensus states pruned, a check that the first Tendermint client
/// decodes, and the v1 localhost client removed. 3 to 4: the localhost
/// client and the sentinel localhost connection. 4 to 5: the client and
/// connection params out of the subspace `ibc`. 5 to 6: the default
/// channel params.
pub fn migrate_ibc(ctx: &mut Context<'_>) -> Result<()> {
    for id in collect_clients(ctx, "06-solomachine")? {
        let key = format!("clients/{id}/clientState");
        let raw = ctx
            .get(x_ibc::STORE_KEY, key.as_bytes())?
            .filter(|b| !b.is_empty())
            .ok_or_else(|| internal(format!("clientID {id}: light client not found")))?;
        let any = ProtoAny::decode(raw.as_slice()).map_err(internal)?;
        let old = v2::ClientState::decode(any.value.as_slice()).map_err(internal)?;
        let new = v3::ClientState {
            sequence: old.sequence,
            is_frozen: old.is_frozen,
            consensus_state: old.consensus_state.map(|c| v3::ConsensusState {
                public_key: c.public_key,
                diversifier: c.diversifier,
                timestamp: c.timestamp,
            }),
        };
        let packed = ProtoAny {
            type_url: "/ibc.lightclients.solomachine.v3.ClientState".into(),
            value: new.encode_to_vec(),
        };
        ctx.set(x_ibc::STORE_KEY, key.into_bytes(), packed.encode_to_vec())?;
        remove_consensus_states(ctx, &id)?;
    }
    if let Some(id) = collect_clients(ctx, "07-tendermint")?.first() {
        let raw = ctx
            .get(
                x_ibc::STORE_KEY,
                format!("clients/{id}/clientState").as_bytes(),
            )?
            .ok_or_else(|| internal(format!("clientID {id}: light client not found")))?;
        let any = ProtoAny::decode(raw.as_slice()).map_err(internal)?;
        if any.type_url != "/ibc.lightclients.tendermint.v1.ClientState" {
            return Err(internal(
                "client state is not tendermint even though client id contains 07-tendermint",
            ));
        }
        TmClientState::decode(any.value.as_slice()).map_err(internal)?;
    }
    for id in collect_clients(ctx, "09-localhost")? {
        ctx.delete(
            x_ibc::STORE_KEY,
            format!("clients/{id}/clientState").as_bytes(),
        )?;
        remove_consensus_states(ctx, &id)?;
    }

    x_ibc::connection::create_sentinel_localhost_connection(ctx)?;
    x_ibc::client::create_localhost_client(ctx)?;

    let allowed: Vec<String> = match x_params::get_raw(ctx, "ibc", b"AllowedClients")? {
        Some(raw) => serde_json::from_slice(&raw).map_err(internal)?,
        None => Vec::new(),
    };
    x_ibc::client::set_params(
        ctx,
        &ClientParams {
            allowed_clients: allowed,
        },
    )?;
    let max_expected = x_params::v046::get_u64(ctx, "ibc", "MaxExpectedTimePerBlock")?;
    x_ibc::connection::set_params(
        ctx,
        &ConnectionParams {
            max_expected_time_per_block: max_expected,
        },
    )?;
    x_ibc::channel::set_params(
        ctx,
        &ChannelParams {
            upgrade_timeout: Some(Timeout {
                height: Some(Height::default()),
                timestamp: DEFAULT_UPGRADE_TIMEOUT_NANOS,
            }),
        },
    )
}
