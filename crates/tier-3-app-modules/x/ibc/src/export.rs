//! IBC core `ExportGenesis` (`02-client`, `03-connection`, `04-channel`
//! `genesis.go` and the keeper iterators they use), over the path-keyed
//! `ibc` store.

use celestia_proto::Any;
use celestia_proto::ibc::core::channel::v1 as chpb;
use celestia_proto::ibc::core::client::v1 as clpb;
use celestia_proto::ibc::core::connection::v1 as copb;
use celestia_proto::ibc::core::types::v1::GenesisState;
use prost::Message;
use sdk::{Context, Result};

use crate::STORE_KEY;

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

fn prefixed(ctx: &mut Context<'_>, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut it = ctx.prefix_iter(STORE_KEY, prefix.as_bytes())?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        out.push((String::from_utf8_lossy(&k).into_owned(), v));
    }
    Ok(out)
}

/// `types.MustParseHeight` (`<revision>-<height>`).
fn parse_height(s: &str) -> clpb::Height {
    let (n, h) = s.split_once('-').unwrap_or(("0", s));
    clpb::Height {
        revision_number: n.parse().unwrap_or(0),
        revision_height: h.parse().unwrap_or(0),
    }
}

const TENDERMINT_CLIENT_STATE: &str = "/ibc.lightclients.tendermint.v1.ClientState";

/// `02-client` `ExportGenesis`.
pub fn export_client_genesis(ctx: &mut Context<'_>) -> Result<clpb::GenesisState> {
    let entries = prefixed(ctx, "clients/")?;
    // GetAllGenesisClients: `clients/<id>/clientState`, sorted by id.
    let mut clients: Vec<clpb::IdentifiedClientState> = entries
        .iter()
        .filter(|(k, _)| k.contains("clientState"))
        .filter_map(|(k, v)| {
            let id = k.split('/').nth(1)?.to_owned();
            Some(decode::<Any>(v).map(|cs| clpb::IdentifiedClientState {
                client_id: id,
                client_state: Some(cs),
            }))
        })
        .collect::<Result<_>>()?;
    clients.sort_by(|a, b| a.client_id.cmp(&b.client_id));
    // GetAllClientMetadata: 07-tendermint's processed time / height and
    // iteration keys, in store order per client.
    let mut clients_metadata = Vec::new();
    for c in &clients {
        if c.client_state.as_ref().map(|a| a.type_url.as_str()) != Some(TENDERMINT_CLIENT_STATE) {
            continue;
        }
        let store = format!("clients/{}/", c.client_id);
        let mut md = Vec::new();
        for (k, v) in prefixed(ctx, &format!("{store}consensusStates"))? {
            let rel = &k[store.len()..];
            let parts: Vec<&str> = rel.split('/').collect();
            if parts.len() == 3 && (parts[2] == "processedTime" || parts[2] == "processedHeight") {
                md.push(clpb::GenesisMetadata {
                    key: rel.as_bytes().to_vec(),
                    value: v,
                });
            }
        }
        for (k, v) in prefixed(ctx, &format!("{store}iterateConsensusStates"))? {
            md.push(clpb::GenesisMetadata {
                key: k.as_bytes()[store.len()..].to_vec(),
                value: v,
            });
        }
        if !md.is_empty() {
            clients_metadata.push(clpb::IdentifiedGenesisMetadata {
                client_id: c.client_id.clone(),
                client_metadata: md,
            });
        }
    }
    // GetAllConsensusStates: `clients/<id>/consensusStates/<height>`,
    // grouped by client in store order, sorted by client id.
    let mut clients_consensus: Vec<clpb::ClientConsensusStates> = Vec::new();
    for (k, v) in &entries {
        let parts: Vec<&str> = k.split('/').collect();
        if parts.len() != 4 || parts[2] != "consensusStates" {
            continue;
        }
        let cs = clpb::ConsensusStateWithHeight {
            height: Some(parse_height(parts[3])),
            consensus_state: Some(decode::<Any>(v)?),
        };
        match clients_consensus
            .iter_mut()
            .find(|c| c.client_id == parts[1])
        {
            Some(c) => c.consensus_states.push(cs),
            None => clients_consensus.push(clpb::ClientConsensusStates {
                client_id: parts[1].to_owned(),
                consensus_states: vec![cs],
            }),
        }
    }
    clients_consensus.sort_by(|a, b| a.client_id.cmp(&b.client_id));
    Ok(clpb::GenesisState {
        clients,
        clients_consensus,
        clients_metadata,
        params: Some(crate::client::get_params(ctx)?),
        #[allow(deprecated)]
        create_localhost: false,
        next_client_sequence: crate::client::get_next_client_sequence(ctx)?,
    })
}

/// `03-connection` `ExportGenesis`.
pub fn export_connection_genesis(ctx: &mut Context<'_>) -> Result<copb::GenesisState> {
    let mut connections = Vec::new();
    for (k, v) in prefixed(ctx, "connections/")? {
        let c: copb::ConnectionEnd = decode(&v)?;
        connections.push(copb::IdentifiedConnection {
            id: k.split('/').nth(1).unwrap_or_default().to_owned(),
            client_id: c.client_id,
            versions: c.versions,
            state: c.state,
            counterparty: c.counterparty,
            delay_period: c.delay_period,
        });
    }
    // GetAllClientConnectionPaths: in client-state iteration order.
    let mut client_connection_paths = Vec::new();
    let entries = prefixed(ctx, "clients/")?;
    for (k, _) in entries.iter().filter(|(k, _)| k.contains("clientState")) {
        let id = k.split('/').nth(1).unwrap_or_default();
        if let Some(v) = ctx.get(STORE_KEY, format!("clients/{id}/connections").as_bytes())? {
            let paths: copb::ClientPaths = decode(&v)?;
            client_connection_paths.push(copb::ConnectionPaths {
                client_id: id.to_owned(),
                paths: paths.paths,
            });
        }
    }
    Ok(copb::GenesisState {
        connections,
        client_connection_paths,
        next_connection_sequence: crate::connection::get_next_connection_sequence(ctx)?,
        params: Some(crate::connection::get_params(ctx)?),
    })
}

/// `(port, channel)` of `<prefix>/ports/<port>/channels/<channel>[/…]`.
fn port_channel(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 5 || parts[1] != "ports" || parts[3] != "channels" {
        return None;
    }
    Some((parts[2].to_owned(), parts[4].to_owned()))
}

fn packet_states(ctx: &mut Context<'_>, prefix: &str) -> Result<Vec<chpb::PacketState>> {
    let mut out = Vec::new();
    for (k, v) in prefixed(ctx, &format!("{prefix}/"))? {
        let parts: Vec<&str> = k.split('/').collect();
        if parts.len() < 5 {
            continue;
        }
        let Ok(sequence) = parts[parts.len() - 1].parse::<u64>() else {
            continue;
        };
        out.push(chpb::PacketState {
            port_id: parts[2].to_owned(),
            channel_id: parts[4].to_owned(),
            sequence,
            data: v,
        });
    }
    Ok(out)
}

fn sequences(ctx: &mut Context<'_>, prefix: &str) -> Result<Vec<chpb::PacketSequence>> {
    let mut out = Vec::new();
    for (k, v) in prefixed(ctx, &format!("{prefix}/"))? {
        let Some((port_id, channel_id)) = port_channel(&k) else {
            // Go's `IteratePacketSequence` stops at an unparsable key.
            break;
        };
        out.push(chpb::PacketSequence {
            port_id,
            channel_id,
            sequence: sdk::codec::decode_u64(&v).unwrap_or(0),
        });
    }
    Ok(out)
}

/// `04-channel` `ExportGenesis`.
pub fn export_channel_genesis(ctx: &mut Context<'_>) -> Result<chpb::GenesisState> {
    let mut channels = Vec::new();
    for (k, v) in prefixed(ctx, "channelEnds/")? {
        let Some((port_id, channel_id)) = port_channel(&k) else {
            continue;
        };
        let c: chpb::Channel = decode(&v)?;
        channels.push(chpb::IdentifiedChannel {
            state: c.state,
            ordering: c.ordering,
            counterparty: c.counterparty,
            connection_hops: c.connection_hops,
            version: c.version,
            port_id,
            channel_id,
            upgrade_sequence: c.upgrade_sequence,
        });
    }
    Ok(chpb::GenesisState {
        channels,
        acknowledgements: packet_states(ctx, "acks")?,
        commitments: packet_states(ctx, "commitments")?,
        receipts: packet_states(ctx, "receipts")?,
        send_sequences: sequences(ctx, "nextSequenceSend")?,
        recv_sequences: sequences(ctx, "nextSequenceRecv")?,
        ack_sequences: sequences(ctx, "nextSequenceAck")?,
        next_channel_sequence: crate::channel::get_next_channel_sequence(ctx)?,
        params: Some(crate::channel::get_params(ctx)?),
    })
}

/// IBC core `ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    Ok(GenesisState {
        client_genesis: Some(export_client_genesis(ctx)?),
        connection_genesis: Some(export_connection_genesis(ctx)?),
        channel_genesis: Some(export_channel_genesis(ctx)?),
    })
}
