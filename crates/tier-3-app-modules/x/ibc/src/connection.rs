//! ICS-03 connection submodule (`core/03-connection`): storage, params,
//! the sentinel localhost connection and genesis.

use celestia_proto::ibc::core::commitment::v1::MerklePrefix;
use celestia_proto::ibc::core::connection::v1::{
    ClientPaths, ConnectionEnd, Counterparty, GenesisState, Params, State, Version,
};
use prost::Message;
use sdk::{Context, Result};

use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "connection";
/// `KeyNextConnectionSequence`.
pub const KEY_NEXT_CONNECTION_SEQUENCE: &[u8] = b"nextConnectionSequence";
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"connectionParams";
/// `ConnectionPrefix`.
pub const CONNECTION_PREFIX: &str = "connection-";
/// `exported.LocalhostConnectionID`.
pub const LOCALHOST_CONNECTION_ID: &str = "connection-localhost";
/// `DefaultIBCVersionIdentifier`.
pub const DEFAULT_IBC_VERSION_IDENTIFIER: &str = "1";
/// `SupportedOrderings`.
pub const SUPPORTED_ORDERINGS: &[&str] = &["ORDER_ORDERED", "ORDER_UNORDERED"];

/// `GetCompatibleVersions`: `[DefaultIBCVersion]`.
#[must_use]
pub fn get_compatible_versions() -> Vec<Version> {
    vec![Version {
        identifier: DEFAULT_IBC_VERSION_IDENTIFIER.to_owned(),
        features: SUPPORTED_ORDERINGS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    }]
}

/// Fills gogoproto non-nullable embedded messages (`counterparty`,
/// `counterparty.prefix`) so they are always encoded, as Go does.
#[must_use]
pub fn normalize_connection(mut c: ConnectionEnd) -> ConnectionEnd {
    let mut cp = c.counterparty.take().unwrap_or_default();
    if cp.prefix.is_none() {
        cp.prefix = Some(MerklePrefix::default());
    }
    c.counterparty = Some(cp);
    c
}

/// `Params.Validate`.
pub fn validate_params(p: &Params) -> Result<()> {
    if p.max_expected_time_per_block == 0 {
        return Err(sdk::error::sdk::invalid_request(
            "MaxExpectedTimePerBlock cannot be zero",
        ));
    }
    Ok(())
}

/// `SetConnection`.
pub fn set_connection(ctx: &mut Context<'_>, connection_id: &str, c: &ConnectionEnd) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::connection_key(connection_id),
        normalize_connection(c.clone()).encode_to_vec(),
    )
}

/// `GetConnection`.
pub fn get_connection(ctx: &mut Context<'_>, connection_id: &str) -> Result<Option<ConnectionEnd>> {
    ctx.get(crate::STORE_KEY, &host::connection_key(connection_id))?
        .map(|b| {
            ConnectionEnd::decode(b.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `SetClientConnectionPaths`.
pub fn set_client_connection_paths(
    ctx: &mut Context<'_>,
    client_id: &str,
    paths: &[String],
) -> Result<()> {
    let cp = ClientPaths {
        paths: paths.to_vec(),
    };
    ctx.set(
        crate::STORE_KEY,
        host::client_connections_key(client_id),
        cp.encode_to_vec(),
    )
}

/// `SetNextConnectionSequence`.
pub fn set_next_connection_sequence(ctx: &mut Context<'_>, seq: u64) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        KEY_NEXT_CONNECTION_SEQUENCE.to_vec(),
        seq.to_be_bytes().to_vec(),
    )
}

/// `GetNextConnectionSequence`.
pub fn get_next_connection_sequence(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx
        .get(crate::STORE_KEY, KEY_NEXT_CONNECTION_SEQUENCE)?
        .unwrap_or_default();
    crate::be_u64(&bz).ok_or_else(|| sdk::error::sdk::logic("next connection sequence is nil"))
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(crate::STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetParams`.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(crate::STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("ibc connection params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `CreateSentinelLocalhostConnection`: an OPEN connection from
/// 09-localhost to itself with the `ibc` commitment prefix.
pub fn create_sentinel_localhost_connection(ctx: &mut Context<'_>) -> Result<()> {
    let end = ConnectionEnd {
        client_id: crate::client::LOCALHOST_CLIENT_ID.to_owned(),
        versions: get_compatible_versions(),
        state: State::Open as i32,
        counterparty: Some(Counterparty {
            client_id: crate::client::LOCALHOST_CLIENT_ID.to_owned(),
            connection_id: LOCALHOST_CONNECTION_ID.to_owned(),
            prefix: Some(MerklePrefix {
                key_prefix: crate::COMMITMENT_PREFIX.to_vec(),
            }),
        }),
        delay_period: 0,
    };
    set_connection(ctx, LOCALHOST_CONNECTION_ID, &end)
}

/// Connection `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for c in &gs.connections {
        let end = ConnectionEnd {
            client_id: c.client_id.clone(),
            versions: c.versions.clone(),
            state: c.state,
            counterparty: c.counterparty.clone(),
            delay_period: c.delay_period,
        };
        set_connection(ctx, &c.id, &end)?;
    }
    for p in &gs.client_connection_paths {
        set_client_connection_paths(ctx, &p.client_id, &p.paths)?;
    }
    set_next_connection_sequence(ctx, gs.next_connection_sequence)?;
    set_params(ctx, &gs.params.unwrap_or_default())?;
    create_sentinel_localhost_connection(ctx)
}
