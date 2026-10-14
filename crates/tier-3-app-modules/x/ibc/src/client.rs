//! ICS-02 client submodule (`core/02-client`): params, client and
//! consensus state storage, the 09-localhost client, genesis and the
//! begin blocker.

use celestia_proto::Any;
use celestia_proto::ibc::core::client::v1::{GenesisState, Height, Params};
use celestia_proto::ibc::core::commitment::v1::MerkleRoot;
use celestia_proto::ibc::lightclients::localhost::v2::ClientState as LocalhostClientState;
use celestia_proto::ibc::lightclients::tendermint::v1::ConsensusState as TmConsensusState;
use prost::Message;
use sdk::{Context, Result, SdkError};

use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "client";
/// `KeyNextClientSequence`.
pub const KEY_NEXT_CLIENT_SEQUENCE: &[u8] = b"nextClientSequence";
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"clientParams";
/// `AllowAllClients` wildcard.
pub const ALLOW_ALL_CLIENTS: &str = "*";

/// `exported.Solomachine`.
pub const SOLOMACHINE: &str = "06-solomachine";
/// `exported.Tendermint`.
pub const TENDERMINT: &str = "07-tendermint";
/// `exported.Localhost` client type.
pub const LOCALHOST: &str = "09-localhost";
/// `exported.LocalhostClientID`.
pub const LOCALHOST_CLIENT_ID: &str = LOCALHOST;

/// Event type emitted when the upgraded consensus state is stored.
pub const EVENT_TYPE_UPGRADE_CHAIN: &str = "upgrade_chain";
/// `AttributeKeyUpgradePlanHeight`.
pub const ATTRIBUTE_KEY_UPGRADE_PLAN_HEIGHT: &str = "upgrade_plan_height";
/// `AttributeKeyUpgradeStore`.
pub const ATTRIBUTE_KEY_UPGRADE_STORE: &str = "upgrade_store";
/// `AttributeValueCategory` (`ibc_client`).
pub const ATTRIBUTE_VALUE_CATEGORY: &str = "ibc_client";

/// Light client status (`exported.Status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `Active`.
    Active,
    /// `Frozen`.
    Frozen,
    /// `Expired`.
    Expired,
    /// `Unknown`.
    Unknown,
    /// `Unauthorized`: the client type is not on the allow list.
    Unauthorized,
}

/// Client type of a packed client state, by type URL (the light-client
/// modules registered by celestia-app: 06-solomachine, 07-tendermint and
/// 09-localhost).
#[must_use]
pub fn client_type(client_state: &Any) -> Option<&'static str> {
    match client_state.type_url.as_str() {
        "/ibc.lightclients.solomachine.v3.ClientState" => Some(SOLOMACHINE),
        "/ibc.lightclients.tendermint.v1.ClientState" => Some(TENDERMINT),
        "/ibc.lightclients.localhost.v2.ClientState" => Some(LOCALHOST),
        _ => None,
    }
}

fn is_consensus_state(cs: &Any) -> bool {
    matches!(
        cs.type_url.as_str(),
        "/ibc.lightclients.solomachine.v3.ConsensusState"
            | "/ibc.lightclients.tendermint.v1.ConsensusState"
    )
}

fn decode_err(e: impl std::fmt::Display) -> SdkError {
    sdk::error::sdk::json_unmarshal(e.to_string())
}

// ---------------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------------

/// `Params.Validate`: no blank or duplicate client types; the `*` wildcard
/// must be alone.
pub fn validate_params(p: &Params) -> Result<()> {
    let clients = &p.allowed_clients;
    if clients.iter().any(|c| c == ALLOW_ALL_CLIENTS) && clients.len() > 1 {
        return Err(sdk::error::sdk::invalid_request(format!(
            "allow list must have only one element because the allow all clients wildcard ({ALLOW_ALL_CLIENTS}) is present"
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, c) in clients.iter().enumerate() {
        if c.trim().is_empty() {
            return Err(sdk::error::sdk::invalid_request(format!(
                "client type {i} cannot be blank"
            )));
        }
        if !seen.insert(c.as_str()) {
            return Err(sdk::error::sdk::invalid_request(format!(
                "duplicate client type: {c}"
            )));
        }
    }
    Ok(())
}

/// `Params.IsAllowedClient`.
#[must_use]
pub fn is_allowed_client(p: &Params, client_type: &str) -> bool {
    if client_type.trim().is_empty() {
        return false;
    }
    if p.allowed_clients.len() == 1 && p.allowed_clients[0] == ALLOW_ALL_CLIENTS {
        return true;
    }
    p.allowed_clients.iter().any(|c| c == client_type)
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(crate::STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetParams` (Go panics when unset; this returns an error).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(crate::STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("client params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(decode_err)
}

// ---------------------------------------------------------------------------
// Client store
// ---------------------------------------------------------------------------

/// `GetClientState` (packed).
pub fn get_client_state(ctx: &mut Context<'_>, client_id: &str) -> Result<Option<Any>> {
    let bz = ctx
        .get(crate::STORE_KEY, &host::full_client_state_key(client_id))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    Any::decode(bz.as_slice()).map(Some).map_err(decode_err)
}

/// `SetClientState` (stores the client state packed as `Any`).
pub fn set_client_state(ctx: &mut Context<'_>, client_id: &str, cs: &Any) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::full_client_state_key(client_id),
        cs.encode_to_vec(),
    )
}

/// `SetClientConsensusState`.
pub fn set_client_consensus_state(
    ctx: &mut Context<'_>,
    client_id: &str,
    height: &Height,
    cs: &Any,
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::full_consensus_state_key(client_id, height),
        cs.encode_to_vec(),
    )
}

/// `SetNextClientSequence`.
pub fn set_next_client_sequence(ctx: &mut Context<'_>, seq: u64) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        KEY_NEXT_CLIENT_SEQUENCE.to_vec(),
        seq.to_be_bytes().to_vec(),
    )
}

/// `GetNextClientSequence`.
pub fn get_next_client_sequence(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx
        .get(crate::STORE_KEY, KEY_NEXT_CLIENT_SEQUENCE)?
        .unwrap_or_default();
    crate::be_u64(&bz).ok_or_else(|| sdk::error::sdk::logic("next client sequence is nil"))
}

/// `SetAllClientMetadata`: raw key/value pairs in each client's store.
pub fn set_all_client_metadata(
    ctx: &mut Context<'_>,
    metadata: &[celestia_proto::ibc::core::client::v1::IdentifiedGenesisMetadata],
) -> Result<()> {
    for igm in metadata {
        let prefix = host::client_store_prefix(&igm.client_id);
        for md in &igm.client_metadata {
            let key = [prefix.as_bytes(), md.key.as_slice()].concat();
            ctx.set(crate::STORE_KEY, key, md.value.clone())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Self height and the 09-localhost client
// ---------------------------------------------------------------------------

/// `ParseChainID`: the revision number of a `{chain}-{N}` chain ID, else 0.
///
/// Mirrors the regex `^.*[^\n-]-{1}[1-9][0-9]*$`.
pub fn parse_chain_id(chain_id: &str) -> Result<u64> {
    let Some((head, rev)) = chain_id.rsplit_once('-') else {
        return Ok(0);
    };
    let rev_ok = rev
        .as_bytes()
        .first()
        .is_some_and(|b| (b'1'..=b'9').contains(b))
        && rev.bytes().all(|b| b.is_ascii_digit());
    let head_ok = !head.contains('\n') && head.chars().last().is_some_and(|c| c != '-');
    if !(rev_ok && head_ok) {
        return Ok(0);
    }
    rev.parse::<u64>().map_err(|_| {
        sdk::error::sdk::logic(format!(
            "regex allowed non-number value as last split element for chainID: {chain_id}"
        ))
    })
}

/// `GetSelfHeight`: `(ParseChainID(chain_id), block height)`.
pub fn get_self_height(ctx: &Context<'_>) -> Result<Height> {
    Ok(Height {
        revision_number: parse_chain_id(&ctx.header.chain_id)?,
        revision_height: u64::try_from(ctx.height()).unwrap_or_default(),
    })
}

fn localhost_client_state(height: Height) -> Any {
    Any::pack(&LocalhostClientState {
        latest_height: Some(height),
    })
}

/// `CreateLocalhostClient`: (re)writes the 09-localhost client state at
/// the current self height.
pub fn create_localhost_client(ctx: &mut Context<'_>) -> Result<()> {
    let h = get_self_height(ctx)?;
    set_client_state(ctx, LOCALHOST_CLIENT_ID, &localhost_client_state(h))
}

/// `UpdateLocalhostClient`: sets the localhost latest height to the self
/// height.
pub fn update_localhost_client(ctx: &mut Context<'_>) -> Result<()> {
    create_localhost_client(ctx)
}

/// `GetClientStatus`.
pub fn get_client_status(ctx: &mut Context<'_>, client_state: &Any) -> Result<Status> {
    let ty = client_type(client_state).unwrap_or_default();
    if !is_allowed_client(&get_params(ctx)?, ty) {
        return Ok(Status::Unauthorized);
    }
    match ty {
        // 09-localhost is always active.
        LOCALHOST => Ok(Status::Active),
        other => Err(sdk::error::sdk::not_supported(format!(
            "status of {other} clients is not implemented"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Upgrade keeper (expected keeper: x/upgrade store)
// ---------------------------------------------------------------------------

/// `upgradetypes.UpgradedClientKey`: `upgradedIBCState/<h>/upgradedClient`.
#[must_use]
pub fn upgraded_client_key(height: i64) -> Vec<u8> {
    format!("upgradedIBCState/{height}/upgradedClient").into_bytes()
}

/// `upgradetypes.UpgradedConsStateKey`: `upgradedIBCState/<h>/upgradedConsState`.
#[must_use]
pub fn upgraded_cons_state_key(height: i64) -> Vec<u8> {
    format!("upgradedIBCState/{height}/upgradedConsState").into_bytes()
}

fn get_upgraded_client(ctx: &mut Context<'_>, height: i64) -> Result<Option<Vec<u8>>> {
    ctx.get(x_upgrade::STORE_KEY, &upgraded_client_key(height))
}

fn set_upgraded_consensus_state(ctx: &mut Context<'_>, height: i64, bz: Vec<u8>) -> Result<()> {
    ctx.set(x_upgrade::STORE_KEY, upgraded_cons_state_key(height), bz)
}

// ---------------------------------------------------------------------------
// Genesis and begin blocker
// ---------------------------------------------------------------------------

/// Client `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let params = gs.params.clone().unwrap_or_default();
    validate_params(&params).map_err(|e| e.wrap("invalid ibc client genesis state parameters"))?;
    set_params(ctx, &params)?;

    if !gs.clients_metadata.is_empty() {
        set_all_client_metadata(ctx, &gs.clients_metadata)?;
    }

    for client in &gs.clients {
        let cs = client.client_state.clone().unwrap_or_default();
        let ty = client_type(&cs).ok_or_else(|| sdk::error::sdk::logic("invalid client state"))?;
        if !is_allowed_client(&params, ty) {
            return Err(sdk::error::sdk::logic(format!(
                "client state type {ty} is not registered on the allowlist"
            )));
        }
        set_client_state(ctx, &client.client_id, &cs)?;
    }

    for cs in &gs.clients_consensus {
        for cons in &cs.consensus_states {
            let height = cons.height.unwrap_or_default();
            let state = cons.consensus_state.clone().unwrap_or_default();
            if !is_consensus_state(&state) {
                return Err(sdk::error::sdk::logic(format!(
                    "invalid consensus state with client ID {} at height {}",
                    cs.client_id,
                    host::height_string(&height)
                )));
            }
            set_client_consensus_state(ctx, &cs.client_id, &height, &state)?;
        }
    }

    set_next_client_sequence(ctx, gs.next_client_sequence)?;

    // The localhost client is always (re)created at the genesis height.
    create_localhost_client(ctx)
}

/// Client `BeginBlocker`: stores the upgraded consensus state one block
/// before a scheduled upgrade that carries an upgraded client, and keeps an
/// active 09-localhost client at the current height.
pub fn begin_blocker(ctx: &mut Context<'_>) -> Result<()> {
    // `GetUpgradePlan` errors (no plan, undecodable plan) skip this part.
    if let Ok(Some(plan)) = x_upgrade::get_upgrade_plan(ctx) {
        let upgraded = get_upgraded_client(ctx, plan.height)?;
        if upgraded.is_some() && ctx.height() == plan.height - 1 {
            let cons = TmConsensusState {
                timestamp: Some(ctx.header.time),
                root: Some(MerkleRoot::default()),
                next_validators_hash: ctx.header.next_validators_hash.clone(),
            };
            set_upgraded_consensus_state(ctx, plan.height, Any::pack(&cons).encode_to_vec())?;
            ctx.events.emit_all([
                sdk::event::event(
                    EVENT_TYPE_UPGRADE_CHAIN,
                    [
                        (ATTRIBUTE_KEY_UPGRADE_PLAN_HEIGHT, plan.height.to_string()),
                        (ATTRIBUTE_KEY_UPGRADE_STORE, x_upgrade::STORE_KEY.to_owned()),
                    ],
                ),
                sdk::event::event("message", [("module", ATTRIBUTE_VALUE_CATEGORY)]),
            ]);
        }
    }

    if let Some(cs) = get_client_state(ctx, LOCALHOST_CLIENT_ID)? {
        if get_client_status(ctx, &cs)? == Status::Active {
            update_localhost_client(ctx)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_id_revision() {
        assert_eq!(parse_chain_id("rs-devnet-1").unwrap(), 1);
        assert_eq!(parse_chain_id("celestia").unwrap(), 0);
        assert_eq!(parse_chain_id("mocha-4").unwrap(), 4);
        assert_eq!(parse_chain_id("chain-0").unwrap(), 0);
        assert_eq!(parse_chain_id("chain-01").unwrap(), 0);
        assert_eq!(parse_chain_id("chain--1").unwrap(), 0);
        assert_eq!(parse_chain_id("-1").unwrap(), 0);
        assert_eq!(parse_chain_id("a\nb-1").unwrap(), 0);
        assert_eq!(parse_chain_id("c-1x").unwrap(), 0);
        assert!(parse_chain_id("c-99999999999999999999999").is_err());
    }

    #[test]
    fn params() {
        let p = Params {
            allowed_clients: vec![SOLOMACHINE.into(), TENDERMINT.into()],
        };
        assert!(validate_params(&p).is_ok());
        assert!(is_allowed_client(&p, TENDERMINT));
        assert!(!is_allowed_client(&p, LOCALHOST));
        let all = Params {
            allowed_clients: vec!["*".into()],
        };
        assert!(is_allowed_client(&all, LOCALHOST));
        assert!(!is_allowed_client(&all, " "));
        assert!(
            validate_params(&Params {
                allowed_clients: vec!["*".into(), "x".into()]
            })
            .is_err()
        );
        assert!(
            validate_params(&Params {
                allowed_clients: vec!["x".into(), "x".into()]
            })
            .is_err()
        );
    }
}
