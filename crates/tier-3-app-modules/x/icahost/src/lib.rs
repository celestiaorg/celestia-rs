//! ICS-27 interchain accounts host (`ibc-go/v8` v8.7.2,
//! `apps/27-interchain-accounts/host`).
//!
//! celestia-app wires the `interchainaccounts` module with the host
//! keeper only (no controller), so genesis initializes only the host
//! sub-state. Store layout (`icahost`):
//!
//! - `port/<port_id>` → `0x01`;
//! - `activeChannel/<port_id>/<connection_id>` → channel ID;
//! - `owner/<port_id>/<connection_id>` → interchain account address;
//! - `params` → host [`Params`].
//!
//! The module has no begin or end blocker.

use celestia_proto::ibc::applications::interchain_accounts::genesis::v1::{
    ActiveChannel, GenesisState, HostGenesisState, RegisteredInterchainAccount,
};
use celestia_proto::ibc::applications::interchain_accounts::host::v1::Params;
use prost::Message;
use sdk::{Context, Result};
use x_capability::ScopedKeeper;

/// Module name (`interchainaccounts`).
pub const MODULE_NAME: &str = "interchainaccounts";
/// Host sub-module name, also its store key and capability scope.
pub const SUB_MODULE_NAME: &str = "icahost";
/// Store key.
pub const STORE_KEY: &str = "icahost";
/// Host port ID.
pub const HOST_PORT_ID: &str = "icahost";
/// ICS-27 version.
pub const VERSION: &str = "ics27-1";
/// Consensus version of the `interchainaccounts` module.
pub const CONSENSUS_VERSION: u64 = 3;
/// Params key.
pub const PARAMS_KEY: &[u8] = b"params";
/// `ActiveChannelKeyPrefix`.
pub const ACTIVE_CHANNEL_KEY_PREFIX: &str = "activeChannel";
/// `OwnerKeyPrefix`.
pub const OWNER_KEY_PREFIX: &str = "owner";
/// `PortKeyPrefix`.
pub const PORT_KEY_PREFIX: &str = "port";
/// `AllowAllHostMsgs`.
pub const ALLOW_ALL_HOST_MSGS: &str = "*";

/// The host's scoped capability keeper (`ScopedICAHostKeeper`).
pub const SCOPED_KEEPER: ScopedKeeper = ScopedKeeper::new(SUB_MODULE_NAME);

/// `KeyActiveChannel`.
#[must_use]
pub fn key_active_channel(port_id: &str, connection_id: &str) -> Vec<u8> {
    format!("{ACTIVE_CHANNEL_KEY_PREFIX}/{port_id}/{connection_id}").into_bytes()
}

/// `KeyOwnerAccount`.
#[must_use]
pub fn key_owner_account(port_id: &str, connection_id: &str) -> Vec<u8> {
    format!("{OWNER_KEY_PREFIX}/{port_id}/{connection_id}").into_bytes()
}

/// `KeyPort`.
#[must_use]
pub fn key_port(port_id: &str) -> Vec<u8> {
    format!("{PORT_KEY_PREFIX}/{port_id}").into_bytes()
}

/// `ContainsMsgType`: whether `type_url` is on the host allow list.
#[must_use]
pub fn contains_msg_type(allow: &[String], type_url: &str) -> bool {
    if allow.len() == 1 && allow[0] == ALLOW_ALL_HOST_MSGS {
        return true;
    }
    allow.iter().any(|m| m == type_url)
}

/// `Params.Validate`: no blank allow-list entries.
pub fn validate_params(p: &Params) -> Result<()> {
    if p.allow_messages.iter().any(|m| m.trim().is_empty()) {
        return Err(sdk::error::sdk::invalid_request(format!(
            "parameter must not contain empty strings: {:?}",
            p.allow_messages
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Keeper
// ---------------------------------------------------------------------------

/// `setPort`.
pub fn set_port(ctx: &mut Context<'_>, port_id: &str) -> Result<()> {
    ctx.set(STORE_KEY, key_port(port_id), vec![0x01])
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetParams` (Go panics when unset; this returns an error).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("ica/host params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `SetActiveChannelID`.
pub fn set_active_channel_id(
    ctx: &mut Context<'_>,
    connection_id: &str,
    port_id: &str,
    channel_id: &str,
) -> Result<()> {
    ctx.set(
        STORE_KEY,
        key_active_channel(port_id, connection_id),
        channel_id.as_bytes().to_vec(),
    )
}

/// `GetActiveChannelID`.
pub fn get_active_channel_id(
    ctx: &mut Context<'_>,
    connection_id: &str,
    port_id: &str,
) -> Result<Option<String>> {
    let bz = ctx
        .get(STORE_KEY, &key_active_channel(port_id, connection_id))?
        .unwrap_or_default();
    Ok((!bz.is_empty()).then(|| String::from_utf8_lossy(&bz).into_owned()))
}

/// `SetInterchainAccountAddress`.
pub fn set_interchain_account_address(
    ctx: &mut Context<'_>,
    connection_id: &str,
    port_id: &str,
    address: &str,
) -> Result<()> {
    ctx.set(
        STORE_KEY,
        key_owner_account(port_id, connection_id),
        address.as_bytes().to_vec(),
    )
}

/// Iterates `<prefix>/<port>/<connection>` → value.
fn iterate_port_connection(
    ctx: &mut Context<'_>,
    prefix: &str,
) -> Result<Vec<(String, String, String)>> {
    let p = format!("{prefix}/");
    let mut it = ctx.prefix_iter(STORE_KEY, p.as_bytes())?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let key = String::from_utf8_lossy(&k).into_owned();
        let parts: Vec<&str> = key.split('/').collect();
        let (Some(port), Some(conn)) = (parts.get(1), parts.get(2)) else {
            return Err(sdk::error::sdk::logic(format!("malformed key {key}")));
        };
        out.push((
            (*port).to_owned(),
            (*conn).to_owned(),
            String::from_utf8_lossy(&v).into_owned(),
        ));
    }
    Ok(out)
}

/// `GetAllActiveChannels`.
pub fn get_all_active_channels(ctx: &mut Context<'_>) -> Result<Vec<ActiveChannel>> {
    Ok(iterate_port_connection(ctx, ACTIVE_CHANNEL_KEY_PREFIX)?
        .into_iter()
        .map(|(port_id, connection_id, channel_id)| ActiveChannel {
            connection_id,
            port_id,
            channel_id,
            is_middleware_enabled: false,
        })
        .collect())
}

/// `GetAllInterchainAccounts`.
pub fn get_all_interchain_accounts(
    ctx: &mut Context<'_>,
) -> Result<Vec<RegisteredInterchainAccount>> {
    Ok(iterate_port_connection(ctx, OWNER_KEY_PREFIX)?
        .into_iter()
        .map(
            |(port_id, connection_id, account_address)| RegisteredInterchainAccount {
                connection_id,
                port_id,
                account_address,
            },
        )
        .collect())
}

/// `hasCapability`: whether the host owns the port capability.
pub fn has_capability(ctx: &mut Context<'_>, port_id: &str) -> Result<bool> {
    Ok(SCOPED_KEEPER
        .get_capability(ctx, &x_ibc::host::port_path(port_id))?
        .is_some())
}

// ---------------------------------------------------------------------------
// Genesis
// ---------------------------------------------------------------------------

/// Decodes the `interchainaccounts` genesis JSON.
pub fn parse_genesis(genesis: &serde_json::Value) -> Result<GenesisState> {
    serde_json::from_value(genesis.clone())
        .map_err(|e| sdk::error::sdk::json_unmarshal(format!("interchainaccounts genesis: {e}")))
}

/// Host `InitGenesis`.
pub fn init_host_genesis(ctx: &mut Context<'_>, gs: &HostGenesisState) -> Result<()> {
    set_port(ctx, &gs.port)?;
    if !has_capability(ctx, &gs.port)? {
        let cap = x_ibc::port::bind_port(ctx, &gs.port)?;
        SCOPED_KEEPER
            .claim_capability(ctx, &cap, &x_ibc::host::port_path(&gs.port))
            .map_err(|e| e.wrap("could not claim port capability"))?;
    }
    for ch in &gs.active_channels {
        set_active_channel_id(ctx, &ch.connection_id, &ch.port_id, &ch.channel_id)?;
    }
    for acc in &gs.interchain_accounts {
        set_interchain_account_address(
            ctx,
            &acc.connection_id,
            &acc.port_id,
            &acc.account_address,
        )?;
    }
    let params = gs.params.clone().unwrap_or_default();
    validate_params(&params).map_err(|e| e.wrap("could not set ica host params at genesis"))?;
    set_params(ctx, &params)
}

/// `interchainaccounts` `InitGenesis`: the controller keeper is not wired
/// in celestia-app, so only the host state is initialized.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    init_host_genesis(ctx, &gs.host_genesis_state.clone().unwrap_or_default())
}

/// `InitGenesis` from the module's `app_state` JSON (skipped when absent).
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    if genesis.is_null() {
        return Ok(());
    }
    init_genesis(ctx, &parse_genesis(genesis)?)
}

/// Host `ExportGenesis`.
pub fn export_host_genesis(ctx: &mut Context<'_>) -> Result<HostGenesisState> {
    Ok(HostGenesisState {
        active_channels: get_all_active_channels(ctx)?,
        interchain_accounts: get_all_interchain_accounts(ctx)?,
        port: HOST_PORT_ID.to_owned(),
        params: Some(get_params(ctx)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [STORE_KEY, x_ibc::STORE_KEY, x_capability::STORE_KEY] {
            ms.mount(s, Kind::Iavl).unwrap();
        }
        ms.mount(x_capability::MEM_STORE_KEY, Kind::Memory).unwrap();
        ms
    }

    fn devnet_genesis() -> serde_json::Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../conformance/fixtures/devnet/empty/genesis.json"
        );
        let g: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        g["app_state"]["interchainaccounts"].clone()
    }

    #[test]
    fn devnet_genesis_matches_go_state() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        x_capability::init_genesis(
            &mut ctx,
            &celestia_proto::capability::v1::GenesisState {
                index: 2,
                owners: vec![],
            },
        )?;
        init_genesis_json(&mut ctx, &devnet_genesis())?;
        let all = ctx
            .multistore()
            .range(STORE_KEY, None, None, false)
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, b"params");
        assert!(hex::encode(&all[0].1).starts_with(
            "080112292f6962632e6170706c69636174696f6e732e7472616e736665722e76312e4d73675472616e73666572"
        ));
        assert_eq!(all[1], (b"port/icahost".to_vec(), vec![1]));
        assert_eq!(
            hex::encode(
                ctx.get(
                    x_capability::STORE_KEY,
                    b"capability_index\0\0\0\0\0\0\0\x02"
                )?
                .unwrap()
            ),
            "0a140a03696263120d706f7274732f696361686f73740a180a07696361686f7374120d706f7274732f696361686f7374"
        );
        let p = get_params(&mut ctx)?;
        assert!(p.host_enabled);
        assert_eq!(p.allow_messages.len(), 12);
        assert!(contains_msg_type(
            &p.allow_messages,
            "/cosmos.bank.v1beta1.MsgSend"
        ));
        Ok(())
    }

    #[test]
    fn host_genesis_roundtrip() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        x_capability::init_genesis(
            &mut ctx,
            &celestia_proto::capability::v1::GenesisState {
                index: 1,
                owners: vec![],
            },
        )?;
        let gs = HostGenesisState {
            active_channels: vec![ActiveChannel {
                connection_id: "connection-0".into(),
                port_id: "icacontroller-abc".into(),
                channel_id: "channel-3".into(),
                is_middleware_enabled: false,
            }],
            interchain_accounts: vec![RegisteredInterchainAccount {
                connection_id: "connection-0".into(),
                port_id: "icacontroller-abc".into(),
                account_address: "celestia1xyz".into(),
            }],
            port: HOST_PORT_ID.into(),
            params: Some(Params {
                host_enabled: true,
                allow_messages: vec!["*".into()],
            }),
        };
        init_host_genesis(&mut ctx, &gs)?;
        assert_eq!(export_host_genesis(&mut ctx)?, gs);
        assert_eq!(
            get_active_channel_id(&mut ctx, "connection-0", "icacontroller-abc")?,
            Some("channel-3".into())
        );
        assert!(contains_msg_type(&["*".into()], "/anything"));
        assert!(
            validate_params(&Params {
                host_enabled: true,
                allow_messages: vec![" ".into()]
            })
            .is_err()
        );
        Ok(())
    }
}
