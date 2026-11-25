//! celestia-app v1's InitChainer: the upgrade module's version map, then
//! each v1 module's InitGenesis in `SetOrderInitGenesis` order
//! (`app/app.go` `InitChainer`, `app/module/manager.go` `InitGenesis`,
//! `app/modules.go` at v3.13.0).
//!
//! A module runs when the app state has its key (`null` included) and it
//! exists at app version 1. Exactly one module may return validator
//! updates and the set must not be empty.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use sdk::{Context, Result, SdkError};
use serde_json::Value as Json;

use super::params::{self, Fields};

/// `SetOrderInitGenesis` restricted to the modules of app version 1
/// (`minfee`, `signal`, packet forward middleware and ICA join at v2).
pub const V1_INIT_GENESIS: &[&str] = &[
    "capability",
    "auth",
    "bank",
    "distribution",
    "staking",
    "slashing",
    "gov",
    "mint",
    "crisis",
    "ibc",
    "genutil",
    "evidence",
    "transfer",
    "blob",
    "qgb",
    "vesting",
    "feegrant",
    "params",
    "authz",
];

/// `GetVersionMap(1)`: each v1 module's consensus version.
pub const V1_VERSION_MAP: &[(&str, u64)] = &[
    ("auth", 3),
    ("authz", 2),
    ("bank", 3),
    ("blob", 2),
    ("capability", 1),
    ("crisis", 1),
    ("distribution", 2),
    ("evidence", 1),
    ("feegrant", 2),
    ("genutil", 1),
    ("gov", 3),
    ("ibc", 2),
    ("mint", 1),
    ("params", 1),
    ("qgb", 2),
    ("slashing", 2),
    ("staking", 3),
    ("transfer", 2),
    ("vesting", 1),
];

fn unmarshal(what: impl std::fmt::Display) -> SdkError {
    sdk::error::sdk::json_unmarshal(what.to_string())
}

/// pbjson decode of a genesis message (unknown fields rejected).
macro_rules! decode {
    ($what:literal, $v:expr) => {
        serde_json::from_value($v).map_err(|e| unmarshal(format!("{}: {e}", $what)))
    };
}

/// The InitChainer of app version 1. `app_state` is the genesis
/// `app_state` (tmjson into `map[string]json.RawMessage`).
pub fn init_chainer(ctx: &mut Context<'_>, app_state: &[u8]) -> Result<Vec<ValidatorUpdate>> {
    let state: serde_json::Map<String, Json> =
        serde_json::from_slice(app_state).map_err(|e| unmarshal(format!("app state: {e}")))?;
    crate::modules::install_staking_hooks();
    x_upgrade::set_module_version_map(ctx, V1_VERSION_MAP.iter().copied())?;
    let mut updates = Vec::new();
    for &module in V1_INIT_GENESIS {
        let Some(genesis) = state.get(module) else {
            continue;
        };
        let module_updates = init_module(ctx, module, genesis)?;
        if !module_updates.is_empty() {
            if !updates.is_empty() {
                return Err(sdk::error::sdk::logic(
                    "validator InitGenesis updates already set by a previous module",
                ));
            }
            updates = module_updates;
        }
    }
    if updates.is_empty() {
        return Err(sdk::error::sdk::logic(
            "validator set is empty after InitGenesis, please ensure at least one validator is initialized with a delegation greater than or equal to the DefaultPowerReduction (1000000)",
        ));
    }
    Ok(updates)
}

/// One v1 module's InitGenesis.
fn init_module(ctx: &mut Context<'_>, module: &str, g: &Json) -> Result<Vec<ValidatorUpdate>> {
    match module {
        "capability" => x_capability::init_genesis_json(ctx, g)?,
        "gov" => x_gov::init_genesis_json(ctx, g)?,
        "mint" => init_mint(ctx, g)?,
        "crisis" => init_crisis(ctx, g)?,
        "ibc" => init_ibc(ctx, g)?,
        "transfer" => init_transfer(ctx, g)?,
        "blob" => {
            let mut f = Fields::new("blob GenesisState", g)?;
            let p = f.take("params").unwrap_or_default();
            f.finish()?;
            params::set_blob(ctx, &p)?;
        }
        "qgb" => super::qgb::init_genesis(ctx, g)?,
        "evidence" => x_evidence::init_genesis_json(ctx, g)?,
        "feegrant" => x_feegrant::init_genesis_json(ctx, g)?,
        "authz" => x_authz::init_genesis_json(ctx, g)?,
        // No genesis state.
        "vesting" | "params" => {}
        "genutil" => {
            return crate::genutil::init_genesis(
                ctx,
                g,
                crate::txpipe::decode_tx,
                crate::txpipe::ante_v046,
                crate::txpipe::apply_validator_set,
            );
        }
        "auth" => init_auth(ctx, g)?,
        // The params go to their subspace first (as `SetParams` does at the
        // start of each v0.46 InitGenesis); the rest of the module's
        // genesis is the v9 code, whose state writes follow the v0.46
        // layout at app versions 1 to 3 (`Context::is_sdk_v046`).
        "bank" | "distribution" | "staking" | "slashing" => {
            let p = g.get("params").cloned().unwrap_or_default();
            match module {
                "bank" => params::set_bank(ctx, &p)?,
                "distribution" => params::set_distribution(ctx, &p)?,
                "staking" => params::set_staking(ctx, &p)?,
                _ => params::set_slashing(ctx, &p)?,
            }
            return crate::modules::init_genesis(ctx, module, g);
        }
        _ => {
            return Err(sdk::error::sdk::logic(format!(
                "unknown v1 module {module}"
            )));
        }
    }
    Ok(Vec::new())
}

/// cosmos-sdk v0.46 auth `InitGenesis`: params into subspace `auth`, then
/// the genesis accounts sorted by account number (`SanitizeGenesisAccounts`)
/// each stored under the next account number (`NewAccount`), then the fee
/// collector module account.
fn init_auth(ctx: &mut Context<'_>, g: &Json) -> Result<()> {
    use celestia_proto::cosmos::auth::v1beta1::GenesisState;
    let raw_params = g.get("params").cloned().unwrap_or_default();
    let gs: GenesisState = decode!("auth GenesisState", g.clone())?;
    params::set_auth(ctx, &raw_params)?;
    let mut accounts = gs
        .accounts
        .iter()
        .map(x_auth::Account::from_any)
        .collect::<Result<Vec<_>>>()?;
    // Go's pdqsort leaves a non-decreasing slice as it is (its pivot
    // choice reports it sorted and `partialInsertionSort` finds no
    // inversion), so equal account numbers keep their genesis order, as on
    // mainnet, where every genesis account number is 0. Duplicates in an
    // unsorted list would need Go's unstable order.
    let sorted = accounts
        .windows(2)
        .all(|w| w[0].account_number() <= w[1].account_number());
    if !sorted {
        accounts.sort_by_key(x_auth::Account::account_number);
        if accounts
            .windows(2)
            .any(|w| w[0].account_number() == w[1].account_number())
        {
            return Err(sdk::error::sdk::not_supported(
                "duplicate account numbers in an unsorted genesis account list",
            ));
        }
    }
    for mut acc in accounts {
        acc.base_mut().account_number = x_auth::next_account_number(ctx)?;
        x_auth::set_account(ctx, &acc)?;
    }
    x_auth::get_module_account(ctx, x_auth::FEE_COLLECTOR)?;
    Ok(())
}

/// celestia-app v1 mint `InitGenesis`: the v1 default minter (initial
/// inflation 8 %, not v9's rate) with the genesis bond denom, the
/// InitChain time as genesis time, and the module account.
fn init_mint(ctx: &mut Context<'_>, g: &Json) -> Result<()> {
    let mut f = Fields::new("mint GenesisState", g)?;
    let bond_denom = f.string("bond_denom")?;
    f.finish()?;
    let minter = x_mint::Minter {
        inflation_rate: math::LegacyDec::new_with_prec(80, 3)
            .map_err(|e| sdk::error::sdk::logic(e.to_string()))?,
        annual_provisions: math::LegacyDec::zero(),
        previous_block_time: None,
        bond_denom,
    };
    x_mint::set_minter(ctx, &minter)?;
    let t = ctx.header.time;
    x_mint::set_genesis_time(ctx, &t)?;
    x_auth::get_module_account(ctx, x_mint::MODULE_NAME)?;
    Ok(())
}

/// crisis `InitGenesis`: `SetConstantFee`. The genesis invariant check
/// (unless the node's `--x-crisis-skip-assert-invariants`) only reads.
fn init_crisis(ctx: &mut Context<'_>, g: &Json) -> Result<()> {
    let mut f = Fields::new("crisis GenesisState", g)?;
    let fee = f.take("constant_fee").unwrap_or_default();
    f.finish()?;
    params::set_crisis(ctx, &fee)
}

/// ibc-go v6 core `InitGenesis`: client, connection and channel genesis.
/// Params live in subspace `ibc`; there is no channel params field and
/// no localhost client or connection. Genesis light clients are not
/// supported (v6's client registry differs from v8's).
fn init_ibc(ctx: &mut Context<'_>, g: &Json) -> Result<()> {
    use celestia_proto::ibc::core::{channel::v1 as chan, connection::v1 as conn};
    let mut f = Fields::new("ibc GenesisState", g)?;
    let client = f.take("client_genesis").unwrap_or_default();
    let connection = f.take("connection_genesis").unwrap_or_default();
    let channel = f.take("channel_genesis").unwrap_or_default();
    f.finish()?;

    let mut c = Fields::new("client GenesisState", &client)?;
    for list in ["clients", "clients_consensus", "clients_metadata"] {
        if c.list(list)?.is_some_and(|l| !l.is_empty()) {
            return Err(sdk::error::sdk::not_supported(format!(
                "ibc-go v6 genesis {list}"
            )));
        }
    }
    let client_params = c.take("params").unwrap_or_default();
    // `CreateLocalhost` is ignored by ibc-go v6.
    let _ = c.take("create_localhost");
    let next_client: u64 = c.int("next_client_sequence")?;
    c.finish()?;
    params::set_ibc_client(ctx, &client_params)?;
    x_ibc::client::set_next_client_sequence(ctx, next_client)?;

    let mut c = Fields::new("connection GenesisState", &connection)?;
    let connection_params = c.take("params").unwrap_or_default();
    let mut rest = serde_json::Map::new();
    for k in [
        "connections",
        "client_connection_paths",
        "next_connection_sequence",
    ] {
        if let Some(v) = c.take(k) {
            rest.insert(k.to_owned(), v);
        }
    }
    c.finish()?;
    let gs: conn::GenesisState = decode!("connection GenesisState", Json::Object(rest))?;
    for c in &gs.connections {
        let end = conn::ConnectionEnd {
            client_id: c.client_id.clone(),
            versions: c.versions.clone(),
            state: c.state,
            counterparty: c.counterparty.clone(),
            delay_period: c.delay_period,
        };
        x_ibc::connection::set_connection(ctx, &c.id, &end)?;
    }
    for p in &gs.client_connection_paths {
        x_ibc::connection::set_client_connection_paths(ctx, &p.client_id, &p.paths)?;
    }
    x_ibc::connection::set_next_connection_sequence(ctx, gs.next_connection_sequence)?;
    params::set_ibc_connection(ctx, &connection_params)?;

    if channel.get("params").is_some() {
        return Err(unmarshal("unknown field \"params\" in types.GenesisState"));
    }
    let gs: chan::GenesisState = decode!("channel GenesisState", channel)?;
    for c in &gs.channels {
        let ch = chan::Channel {
            state: c.state,
            ordering: c.ordering,
            counterparty: c.counterparty.clone(),
            connection_hops: c.connection_hops.clone(),
            version: c.version.clone(),
            upgrade_sequence: 0,
        };
        x_ibc::channel::set_channel(ctx, &c.port_id, &c.channel_id, &ch)?;
    }
    for a in &gs.acknowledgements {
        x_ibc::channel::set_packet_acknowledgement(
            ctx,
            &a.port_id,
            &a.channel_id,
            a.sequence,
            &a.data,
        )?;
    }
    for c in &gs.commitments {
        x_ibc::channel::set_packet_commitment(ctx, &c.port_id, &c.channel_id, c.sequence, &c.data)?;
    }
    for r in &gs.receipts {
        x_ibc::channel::set_packet_receipt(ctx, &r.port_id, &r.channel_id, r.sequence)?;
    }
    for s in &gs.send_sequences {
        x_ibc::channel::set_next_sequence_send(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.recv_sequences {
        x_ibc::channel::set_next_sequence_recv(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.ack_sequences {
        x_ibc::channel::set_next_sequence_ack(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    x_ibc::channel::set_next_channel_sequence(ctx, gs.next_channel_sequence)
}

/// ibc-go v6 transfer `InitGenesis`: port, denom traces (no bank
/// metadata, no escrow totals), port binding, params in subspace
/// `transfer`.
fn init_transfer(ctx: &mut Context<'_>, g: &Json) -> Result<()> {
    use celestia_proto::ibc::applications::transfer::v1::DenomTrace;
    let mut f = Fields::new("transfer GenesisState", g)?;
    let port_id = f.string("port_id")?;
    let traces = f.list("denom_traces")?.unwrap_or_default();
    let p = f.take("params").unwrap_or_default();
    f.finish()?;
    x_transfer::set_port(ctx, &port_id)?;
    for t in traces {
        let t: DenomTrace = decode!("DenomTrace", t)?;
        x_transfer::set_denom_trace(ctx, &t)?;
    }
    if !x_transfer::has_capability(ctx, &port_id)? {
        x_transfer::bind_port(ctx, &port_id)
            .map_err(|e| e.wrap("could not claim port capability"))?;
    }
    params::set_transfer(ctx, &p)
}
