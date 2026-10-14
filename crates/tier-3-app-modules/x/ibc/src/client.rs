//! ICS-02 client submodule (`core/02-client`): params, client and
//! consensus state storage, the 09-localhost client, genesis and the
//! begin blocker.

use celestia_proto::Any;
use celestia_proto::ibc::core::client::v1::{GenesisState, Height, Params};
use celestia_proto::ibc::core::commitment::v1::{MerklePath, MerkleRoot};
use celestia_proto::ibc::lightclients::localhost::v2::ClientState as LocalhostClientState;
use celestia_proto::ibc::lightclients::solomachine::v3 as sm;
use celestia_proto::ibc::lightclients::tendermint::v1 as tmpb;
use celestia_proto::ibc::lightclients::tendermint::v1::ConsensusState as TmConsensusState;
use prost::{Message, Name};
use sdk::{Context, Result, SdkError};

use crate::errors::{client as cerr, ibc as ierr};
use crate::height;
use crate::host;
use crate::lightclients::tendermint as tmc;

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
        return Err(crate::errors::unregistered(format!(
            "allow list must have only one element because the allow all clients wildcard ({ALLOW_ALL_CLIENTS}) is present"
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, c) in clients.iter().enumerate() {
        if c.trim().is_empty() {
            return Err(crate::errors::unregistered(format!(
                "client type {i} cannot be blank"
            )));
        }
        if !seen.insert(c.as_str()) {
            return Err(crate::errors::unregistered(format!(
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

    if let Some(cs) = client_state(ctx, LOCALHOST_CLIENT_ID)?
        && client_status(ctx, LOCALHOST_CLIENT_ID, &cs)? == Status::Active
    {
        update_localhost_client(ctx)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Light-client dispatch
// ---------------------------------------------------------------------------

/// A decoded client state of one of the registered light clients.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientState {
    /// 07-tendermint.
    Tendermint(Box<tmpb::ClientState>),
    /// 06-solomachine.
    Solomachine(Box<sm::ClientState>),
    /// 09-localhost.
    Localhost(LocalhostClientState),
}

impl ClientState {
    /// `UnpackClientState` (the type URL must be a registered client state).
    pub fn from_any(any: &Any) -> Result<Self> {
        let dec = |e: prost::DecodeError| ierr::unpack_any(e.to_string());
        if let Some(r) = any.unpack::<tmpb::ClientState>() {
            return Ok(Self::Tendermint(Box::new(r.map_err(dec)?)));
        }
        if let Some(r) = any.unpack::<sm::ClientState>() {
            return Ok(Self::Solomachine(Box::new(r.map_err(dec)?)));
        }
        if let Some(r) = any.unpack::<LocalhostClientState>() {
            return Ok(Self::Localhost(r.map_err(dec)?));
        }
        Err(ierr::unpack_any(format!(
            "cannot unpack Any into ClientState {}",
            any.type_url
        )))
    }

    /// `ClientType()`.
    #[must_use]
    pub fn client_type(&self) -> &'static str {
        match self {
            Self::Tendermint(_) => TENDERMINT,
            Self::Solomachine(_) => SOLOMACHINE,
            Self::Localhost(_) => LOCALHOST,
        }
    }

    /// `GetLatestHeight()`.
    #[must_use]
    pub fn latest_height(&self) -> Height {
        match self {
            Self::Tendermint(c) => c.latest_height.unwrap_or_default(),
            Self::Solomachine(c) => Height {
                revision_number: 0,
                revision_height: c.sequence,
            },
            Self::Localhost(c) => c.latest_height.unwrap_or_default(),
        }
    }

    /// `MarshalInterface` form.
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Tendermint(c) => tmc::client_state_any(c),
            Self::Solomachine(c) => Any::pack(c.as_ref()),
            Self::Localhost(c) => localhost_client_state(c.latest_height.unwrap_or_default()),
        }
    }

    /// `ClientState.Validate`.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Tendermint(c) => tmc::validate(c),
            Self::Solomachine(c) => {
                if c.sequence == 0 {
                    return Err(cerr::invalid_client("sequence cannot be 0"));
                }
                let Some(cons) = c.consensus_state.as_ref() else {
                    return Err(cerr::invalid_consensus("consensus state cannot be nil"));
                };
                validate_solomachine_consensus(cons)
            }
            Self::Localhost(c) => {
                if c.latest_height.unwrap_or_default().revision_height == 0 {
                    return Err(ierr::invalid_height("local revision height cannot be zero"));
                }
                Ok(())
            }
        }
    }
}

/// solomachine `ConsensusState.ValidateBasic`.
fn validate_solomachine_consensus(c: &sm::ConsensusState) -> Result<()> {
    if c.timestamp == 0 {
        return Err(cerr::invalid_consensus("timestamp cannot be 0"));
    }
    if !c.diversifier.is_empty() && c.diversifier.trim().is_empty() {
        return Err(cerr::invalid_consensus(
            "diversifier cannot contain only spaces",
        ));
    }
    if c.public_key.is_none() {
        return Err(cerr::invalid_consensus("public key cannot be empty"));
    }
    Ok(())
}

/// `GetClientState` (decoded; undecodable state is a Go panic).
pub fn client_state(ctx: &mut Context<'_>, client_id: &str) -> Result<Option<ClientState>> {
    match get_client_state(ctx, client_id)? {
        None => Ok(None),
        Some(any) => ClientState::from_any(&any)
            .map(Some)
            .map_err(|e| sdk::error::go_panic(e.log)),
    }
}

/// `GetClientStatus` for a decoded client.
pub fn client_status(ctx: &mut Context<'_>, client_id: &str, cs: &ClientState) -> Result<Status> {
    if !is_allowed_client(&get_params(ctx)?, cs.client_type()) {
        return Ok(Status::Unauthorized);
    }
    Ok(match cs {
        ClientState::Tendermint(c) => tmc::status(ctx, client_id, c)?,
        ClientState::Solomachine(c) => {
            if c.is_frozen {
                Status::Frozen
            } else {
                Status::Active
            }
        }
        ClientState::Localhost(_) => Status::Active,
    })
}

impl Status {
    /// `Status.String()`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Frozen => "Frozen",
            Self::Expired => "Expired",
            Self::Unknown => "Unknown",
            Self::Unauthorized => "Unauthorized",
        }
    }
}

/// `IsRevisionFormat`: `^.*[^\n-]-{1}[1-9][0-9]*$`.
#[must_use]
pub fn is_revision_format(chain_id: &str) -> bool {
    let Some((head, rev)) = chain_id.rsplit_once('-') else {
        return false;
    };
    let rev_ok = rev
        .as_bytes()
        .first()
        .is_some_and(|b| (b'1'..=b'9').contains(b))
        && rev.bytes().all(|b| b.is_ascii_digit());
    let head_ok = !head.contains('\n') && head.chars().last().is_some_and(|c| c != '-');
    rev_ok && head_ok
}

/// `SetRevisionNumber` (callers check `is_revision_format` first).
#[must_use]
pub fn set_revision_number(chain_id: &str, revision: u64) -> String {
    let mut parts: Vec<String> = chain_id.split('-').map(str::to_owned).collect();
    if let Some(last) = parts.last_mut() {
        *last = revision.to_string();
    }
    parts.join("-")
}

/// Latest height of a packed client state.
pub fn latest_height_of(any: &Any) -> Result<Height> {
    Ok(ClientState::from_any(any)?.latest_height())
}

/// `GenerateClientIdentifier`.
pub fn generate_client_identifier(ctx: &mut Context<'_>, client_type: &str) -> Result<String> {
    let seq = get_next_client_sequence(ctx)?;
    let id = format!("{client_type}-{seq}");
    set_next_client_sequence(ctx, seq.wrapping_add(1))?;
    Ok(id)
}

/// A decoded client message.
#[derive(Debug, Clone)]
pub enum ClientMessage {
    /// 07-tendermint header or misbehaviour.
    Tendermint(tmc::ClientMessage),
    /// 06-solomachine header or misbehaviour (verification not ported).
    Solomachine(Any),
}

impl ClientMessage {
    /// `UnpackClientMessage`.
    pub fn from_any(any: Option<&Any>) -> Result<Self> {
        let Some(any) = any else {
            return Err(ierr::unpack_any("protobuf Any message cannot be nil"));
        };
        if let Some(r) = tmc::unpack_client_message(any) {
            return r.map(Self::Tendermint);
        }
        if any.type_url == sm::Header::type_url() || any.type_url == sm::Misbehaviour::type_url() {
            return Ok(Self::Solomachine(any.clone()));
        }
        Err(ierr::unpack_any(format!(
            "cannot unpack Any into ClientMessage {}",
            any.type_url
        )))
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        match self {
            Self::Tendermint(m) => tmc::validate_client_message(m),
            Self::Solomachine(_) => Err(solomachine_not_ported()),
        }
    }

    /// `MustMarshalClientMessage` (for the update event).
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Tendermint(tmc::ClientMessage::Header(h)) => Any::pack(h.as_ref()),
            Self::Tendermint(tmc::ClientMessage::Misbehaviour(m)) => Any::pack(m.as_ref()),
            Self::Solomachine(a) => a.clone(),
        }
    }
}

/// 06-solomachine signature verification is not ported yet.
#[must_use]
pub fn solomachine_not_ported() -> SdkError {
    sdk::error::sdk::not_supported("06-solomachine verification is not ported yet")
}

/// Type of a packed consensus state (`UnpackConsensusState`).
fn consensus_type(any: Option<&Any>) -> Result<&'static str> {
    let Some(any) = any else {
        return Err(ierr::unpack_any("protobuf Any message cannot be nil"));
    };
    if any.type_url == tmpb::ConsensusState::type_url() {
        Ok(TENDERMINT)
    } else if any.type_url == sm::ConsensusState::type_url() {
        Ok(SOLOMACHINE)
    } else {
        Err(ierr::unpack_any(format!(
            "cannot unpack Any into ConsensusState {}",
            any.type_url
        )))
    }
}

/// `MsgCreateClient.ValidateBasic` (after the signer check).
pub fn validate_create_client(
    client_state: Option<&Any>,
    consensus_state: Option<&Any>,
) -> Result<()> {
    let cs = ClientState::from_any(
        client_state.ok_or_else(|| ierr::unpack_any("protobuf Any message cannot be nil"))?,
    )?;
    cs.validate()?;
    let ty = consensus_type(consensus_state)?;
    if cs.client_type() != ty {
        return Err(cerr::invalid_client_type(
            "client type for client state and consensus state do not match",
        ));
    }
    let any = consensus_state.cloned().unwrap_or_default();
    let dec = |e: prost::DecodeError| ierr::unpack_any(e.to_string());
    match ty {
        TENDERMINT => tmc::validate_consensus_state(
            &any.unpack::<tmpb::ConsensusState>()
                .unwrap_or(Ok(tmpb::ConsensusState::default()))
                .map_err(dec)?,
        ),
        _ => validate_solomachine_consensus(
            &any.unpack::<sm::ConsensusState>()
                .unwrap_or(Ok(sm::ConsensusState::default()))
                .map_err(dec)?,
        ),
    }
}

/// `MsgUpgradeClient.ValidateBasic` type checks.
pub fn validate_upgrade_client(
    client_state: Option<&Any>,
    consensus_state: Option<&Any>,
) -> Result<()> {
    let cs = ClientState::from_any(
        client_state.ok_or_else(|| ierr::unpack_any("protobuf Any message cannot be nil"))?,
    )?;
    let ty = consensus_type(consensus_state)?;
    if cs.client_type() != ty {
        return Err(cerr::invalid_upgrade_client(
            "consensus state's client-type does not match client",
        ));
    }
    Ok(())
}

fn client_events(
    ty: &str,
    attrs: Vec<(&str, String)>,
) -> Vec<celestia_proto::celestia_core_proto::abci::Event> {
    vec![
        sdk::event::event(ty, attrs),
        sdk::event::event("message", [("module", ATTRIBUTE_VALUE_CATEGORY)]),
    ]
}

/// Keeper `CreateClient`.
pub fn create_client(
    ctx: &mut Context<'_>,
    client_state: &Any,
    consensus_state: &Any,
) -> Result<String> {
    let cs = ClientState::from_any(client_state)?;
    if cs.client_type() == LOCALHOST {
        return Err(cerr::invalid_client_type(format!(
            "cannot create client of type: {LOCALHOST}"
        )));
    }
    if !is_allowed_client(&get_params(ctx)?, cs.client_type()) {
        return Err(cerr::invalid_client_type(format!(
            "client state type {} is not registered in the allowlist",
            cs.client_type()
        )));
    }
    let client_id = generate_client_identifier(ctx, cs.client_type())?;
    match &cs {
        ClientState::Tendermint(c) => tmc::initialize(ctx, &client_id, c, consensus_state)?,
        ClientState::Solomachine(c) => {
            let cons = consensus_state
                .unpack::<sm::ConsensusState>()
                .and_then(core::result::Result::ok);
            if c.consensus_state != cons {
                return Err(cerr::invalid_consensus(
                    "consensus state in initial client does not equal initial consensus state",
                ));
            }
            set_client_state(ctx, &client_id, &cs.to_any())?;
        }
        ClientState::Localhost(_) => {
            return Err(cerr::invalid_client_type(
                "cannot create client of type: 09-localhost",
            ));
        }
    }
    let status = client_status(ctx, &client_id, &cs)?;
    if status != Status::Active {
        return Err(cerr::client_not_active(format!(
            "cannot create client ({client_id}) with status {}",
            status.as_str()
        )));
    }
    ctx.events.emit_all(client_events(
        "create_client",
        vec![
            ("client_id", client_id.clone()),
            ("client_type", cs.client_type().to_owned()),
            ("consensus_height", height::to_string(&cs.latest_height())),
        ],
    ));
    Ok(client_id)
}

fn hex_lower(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

/// Keeper `UpdateClient` (also handles misbehaviour submissions).
pub fn update_client(ctx: &mut Context<'_>, client_id: &str, msg: &ClientMessage) -> Result<()> {
    let cs = client_state(ctx, client_id)?.ok_or_else(|| {
        cerr::client_not_found(format!("cannot update client with ID {client_id}"))
    })?;
    let status = client_status(ctx, client_id, &cs)?;
    if status != Status::Active {
        return Err(cerr::client_not_active(format!(
            "cannot update client ({client_id}) with status {}",
            status.as_str()
        )));
    }
    match (&cs, msg) {
        (ClientState::Tendermint(c), ClientMessage::Tendermint(m)) => {
            tmc::verify_client_message(ctx, client_id, c, m)?;
            if tmc::check_for_misbehaviour(ctx, client_id, m)? {
                tmc::update_state_on_misbehaviour(ctx, client_id, c)?;
                ctx.events.emit_all(client_events(
                    "client_misbehaviour",
                    vec![
                        ("client_id", client_id.to_owned()),
                        ("client_type", TENDERMINT.to_owned()),
                    ],
                ));
                return Ok(());
            }
            let heights = tmc::update_state(ctx, client_id, c, m)?;
            let header = hex_lower(&msg.to_any().encode_to_vec());
            ctx.events.emit_all(client_events(
                "update_client",
                vec![
                    ("client_id", client_id.to_owned()),
                    ("client_type", TENDERMINT.to_owned()),
                    (
                        "consensus_height",
                        heights.first().map(height::to_string).unwrap_or_default(),
                    ),
                    (
                        "consensus_heights",
                        heights
                            .iter()
                            .map(height::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                    ("header", header),
                ],
            ));
            Ok(())
        }
        (ClientState::Localhost(_), _) => Err(cerr::update_client_failed(
            "client message verification is unsupported by the localhost client",
        )),
        // Go's type switch falls through to ErrInvalidClientType.
        (ClientState::Tendermint(_), ClientMessage::Solomachine(_)) => {
            Err(cerr::invalid_client_type(""))
        }
        (ClientState::Solomachine(_), _) => Err(solomachine_not_ported()),
    }
}

/// Keeper `UpgradeClient`.
pub fn upgrade_client(
    ctx: &mut Context<'_>,
    client_id: &str,
    upgraded_client: &Any,
    upgraded_cons: &Any,
    proof_client: &[u8],
    proof_cons: &[u8],
) -> Result<()> {
    let cs = client_state(ctx, client_id)?.ok_or_else(|| {
        cerr::client_not_found(format!("cannot update client with ID {client_id}"))
    })?;
    let status = client_status(ctx, client_id, &cs)?;
    if status != Status::Active {
        return Err(cerr::client_not_active(format!(
            "cannot upgrade client ({client_id}) with status {}",
            status.as_str()
        )));
    }
    let res = match &cs {
        ClientState::Tendermint(c) => tmc::verify_upgrade_and_update_state(
            ctx,
            client_id,
            c,
            upgraded_client,
            upgraded_cons,
            proof_client,
            proof_cons,
        ),
        ClientState::Solomachine(_) => Err(cerr::invalid_upgrade_client(
            "cannot upgrade solomachine client",
        )),
        ClientState::Localhost(_) => Err(cerr::invalid_upgrade_client(
            "cannot upgrade localhost client",
        )),
    };
    res.map_err(|e| e.wrap(format!("cannot upgrade client with ID {client_id}")))?;
    let up = ClientState::from_any(upgraded_client)?;
    ctx.events.emit_all(client_events(
        "upgrade_client",
        vec![
            ("client_id", client_id.to_owned()),
            ("client_type", up.client_type().to_owned()),
            ("consensus_height", height::to_string(&up.latest_height())),
        ],
    ));
    Ok(())
}

/// Keeper `RecoverClient`.
pub fn recover_client(ctx: &mut Context<'_>, subject_id: &str, substitute_id: &str) -> Result<()> {
    let subject = client_state(ctx, subject_id)?
        .ok_or_else(|| cerr::client_not_found(format!("subject client with ID {subject_id}")))?;
    if client_status(ctx, subject_id, &subject)? == Status::Active {
        return Err(cerr::invalid_recovery_client(
            "cannot recover Active subject client",
        ));
    }
    let substitute = client_state(ctx, substitute_id)?.ok_or_else(|| {
        cerr::client_not_found(format!("substitute client with ID {substitute_id}"))
    })?;
    if height::gte(&subject.latest_height(), &substitute.latest_height()) {
        return Err(cerr::invalid_height(
            "subject client state latest height is greater or equal to substitute client state latest height",
        ));
    }
    let status = client_status(ctx, substitute_id, &substitute)?;
    if status != Status::Active {
        return Err(cerr::client_not_active(format!(
            "substitute client is not Active, status is {}",
            status.as_str()
        )));
    }
    let res = match &subject {
        ClientState::Tendermint(c) => tmc::check_substitute_and_update_state(
            ctx,
            subject_id,
            c,
            substitute_id,
            &substitute.to_any(),
        ),
        ClientState::Solomachine(_) => Err(solomachine_not_ported()),
        ClientState::Localhost(_) => Err(cerr::update_client_failed(
            "cannot update localhost client with a proposal",
        )),
    };
    res.map_err(|e| e.wrap("failed to validate substitute client"))?;
    ctx.events.emit_all(client_events(
        "recover_client",
        vec![
            ("subject_client_id", subject_id.to_owned()),
            ("client_type", substitute.client_type().to_owned()),
        ],
    ));
    Ok(())
}

/// `ClientState.GetTimestampAtHeight`.
pub fn get_timestamp_at_height(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    h: &Height,
) -> Result<u64> {
    match cs {
        ClientState::Tendermint(_) => tmc::get_timestamp_at_height(ctx, client_id, h),
        ClientState::Solomachine(c) => Ok(c.consensus_state.as_ref().map_or(0, |c| c.timestamp)),
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        ClientState::Localhost(_) => Ok(tmc::block_time(ctx) as u64),
    }
}

/// `SentinelProof` of the 09-localhost client.
pub const LOCALHOST_SENTINEL_PROOF: &[u8] = &[0x01];

/// `ClientState.VerifyMembership` / `VerifyNonMembership` (`value = None`).
/// For 09-localhost the store is the IBC store itself.
#[allow(clippy::too_many_arguments)]
pub fn verify_membership(
    ctx: &mut Context<'_>,
    client_id: &str,
    cs: &ClientState,
    h: &Height,
    delay_time: u64,
    delay_block: u64,
    proof: &[u8],
    path: &MerklePath,
    value: Option<&[u8]>,
) -> Result<()> {
    match cs {
        ClientState::Tendermint(c) => tmc::verify_membership(
            ctx,
            client_id,
            c,
            h,
            delay_time,
            delay_block,
            proof,
            path,
            value,
        ),
        ClientState::Solomachine(_) => Err(solomachine_not_ported()),
        ClientState::Localhost(_) => {
            if proof != LOCALHOST_SENTINEL_PROOF {
                return Err(crate::errors::commitment::invalid_proof(
                    "expected sentinel proof",
                ));
            }
            if path.key_path.len() != 2 {
                return Err(crate::errors::host::invalid_path(
                    "path must be of length 2",
                ));
            }
            let key = path.key_path[1].as_bytes();
            match value {
                Some(v) => match ctx.get(crate::STORE_KEY, key)? {
                    None => Err(cerr::failed_membership_verification(
                        "value not found for path",
                    )),
                    Some(bz) if bz != v => Err(cerr::failed_membership_verification(
                        "value provided does not equal value stored at path",
                    )),
                    Some(_) => Ok(()),
                },
                None => {
                    if ctx.has(crate::STORE_KEY, key)? {
                        Err(cerr::failed_non_membership_verification(
                            "value found for path",
                        ))
                    } else {
                        Ok(())
                    }
                }
            }
        }
    }
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
