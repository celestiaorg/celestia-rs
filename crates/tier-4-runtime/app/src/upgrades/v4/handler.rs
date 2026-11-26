//! The `v4` upgrade handler (celestia-app v4.1.0 `RegisterUpgradeHandlers`)
//! and its `ModuleManager.RunMigrations`.

use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
use celestia_proto::cosmos::base::v1beta1::Coin;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

/// `ModuleManager.GetVersionMap` of celestia-app v4.1.0: its modules and
/// consensus versions (`interchainaccounts` is not registered; crisis and
/// qgb are gone, their stale entries stay in the stored map).
pub const VERSION_MAP: &[(&str, u64)] = &[
    ("06-solomachine", 0),
    ("07-tendermint", 0),
    ("auth", 5),
    ("authz", 2),
    ("bank", 4),
    ("blob", 3),
    ("capability", 1),
    ("circuit", 1),
    ("consensus", 1),
    ("distribution", 4),
    ("evidence", 1),
    ("feegrant", 2),
    ("genutil", 1),
    ("gov", 5),
    ("hyperlane", 1),
    ("ibc", 6),
    ("minfee", 2),
    ("mint", 1),
    ("packetfowardmiddleware", 3),
    ("params", 1),
    ("signal", 3),
    ("slashing", 4),
    ("staking", 5),
    ("transfer", 5),
    ("upgrade", 3),
    ("vesting", 1),
    ("warp", 1),
];

/// The circuit breaker's disabled messages (`DisableList.Set`).
const DISABLED_MESSAGES: [&str; 3] = [
    "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
    "/cosmos.upgrade.v1beta1.MsgCancelUpgrade",
    "/ibc.core.client.v1.MsgIBCSoftwareUpgrade",
];

/// `const tia = 1_000_000` utia; the expedited min deposit is 50,000 TIA.
const EXPEDITED_MIN_DEPOSIT_UTIA: u64 = 50_000 * 1_000_000;

/// The `v4` upgrade handler:
///
/// 1. The key tables attached to the legacy subspaces hold no state.
/// 2. The consensus params from the legacy subspace `baseapp`
///    (`baseapp.GetConsensusParams`: block, evidence and validator params;
///    v0.50 reads no `VersionParams`, so the version is nil and becomes
///    `{App: 3}`) go to x/consensus.
/// 3. The circuit breaker disables the software-upgrade messages.
/// 4. `RunMigrations` ([`run_migrations`]).
/// 5. gov's expedited min deposit becomes 50,000 TIA.
///
/// x/upgrade's `ApplyUpgrade` then raises the app version by one
/// ([`super::super::Handlers`]).
pub fn handler(ctx: &mut Context<'_>, from: &VersionMap) -> Result<VersionMap> {
    let mut cp = super::legacy(ctx, |ctx| crate::legacy::consensus_params::load(ctx, 0))?;
    cp.version = Some(VersionParams { app: 3 });
    x_consensus::set_params(ctx, &cp)?;

    for url in DISABLED_MESSAGES {
        // `DisableList` (`collections.KeySet[string]` at prefix 0x02).
        let key = [&[x_circuit::DISABLE_LIST_PREFIX][..], url.as_bytes()].concat();
        ctx.set(x_circuit::STORE_KEY, key, Vec::new())?;
    }

    let versions = run_migrations(ctx, from)?;

    let mut params = x_gov::get_params(ctx)?;
    params.expedited_min_deposit = vec![Coin {
        denom: "utia".into(),
        amount: EXPEDITED_MIN_DEPOSIT_UTIA.to_string(),
    }];
    x_gov::set_params(ctx, &params)?;
    Ok(versions)
}

/// `module.DefaultMigrationsOrder`: by name, with auth last.
fn migration_order() -> impl Iterator<Item = (&'static str, u64)> {
    VERSION_MAP
        .iter()
        .copied()
        .filter(|(name, _)| *name != "auth")
        .chain(
            VERSION_MAP
                .iter()
                .copied()
                .filter(|(name, _)| *name == "auth"),
        )
}

/// `ModuleManager.RunMigrations`: each module's migrations from its stored
/// version, or `InitGenesis` with its default genesis when the stored map
/// lacks it (minfee, signal, packetfowardmiddleware and upgrade, which the
/// v3 map never recorded, and the modules new in v4).
pub fn run_migrations(ctx: &mut Context<'_>, from: &VersionMap) -> Result<VersionMap> {
    let mut out = VersionMap::new();
    for (name, to) in migration_order() {
        match from.get(name).copied() {
            Some(v) if v == to => {}
            Some(v) => migrate(ctx, name, v, to)?,
            None => init_default_genesis(ctx, name)?,
        }
        out.insert(name.to_owned(), to);
    }
    Ok(out)
}

fn migrate(ctx: &mut Context<'_>, name: &str, from: u64, to: u64) -> Result<()> {
    match (name, from, to) {
        ("auth", 3, 5) => super::migrate_auth(ctx),
        ("bank", 3, 4) => super::migrate_bank(ctx),
        ("blob", 2, 3) => super::migrate_blob(ctx),
        ("distribution", 2, 4) => super::migrate_distribution(ctx),
        ("gov", 3, 5) => super::migrate_gov(ctx),
        ("ibc", 2, 6) => super::migrate_ibc(ctx),
        ("slashing", 2, 4) => super::migrate_slashing(ctx),
        ("staking", 3, 5) => super::migrate_staking(ctx),
        ("transfer", 2, 5) => super::migrate_transfer(ctx),
        _ => Err(sdk::error::sdk::not_supported(format!(
            "migration of module {name} from version {from} to {to}"
        ))),
    }
}

/// `InitGenesis` with `DefaultGenesis` for a module the stored map lacks.
fn init_default_genesis(ctx: &mut Context<'_>, name: &str) -> Result<()> {
    match name {
        // No genesis state, or a default genesis that writes nothing.
        "06-solomachine" | "07-tendermint" | "circuit" | "consensus" | "signal" | "upgrade" => {
            Ok(())
        }
        "packetfowardmiddleware" => x_packetforward::init_genesis(
            ctx,
            &celestia_proto::packetforward::v1::GenesisState::default(),
        ),
        "minfee" => x_minfee::set_params(
            ctx,
            &x_minfee::MinfeeParams {
                network_min_gas_price: x_minfee::default_network_min_gas_price(),
            },
        ),
        "hyperlane" => {
            x_hyperlane::genesis::init_genesis(
                ctx,
                &celestia_proto::hyperlane::core::v1::GenesisState::default(),
            )?;
            x_hyperlane::ism::init_genesis(
                ctx,
                &celestia_proto::hyperlane::core::interchain_security::v1::GenesisState::default(),
            )?;
            x_hyperlane::post_dispatch::init_genesis(
                ctx,
                &celestia_proto::hyperlane::core::post_dispatch::v1::GenesisState::default(),
            )
        }
        "warp" => x_warp::init_genesis(
            ctx,
            &celestia_proto::hyperlane::warp::v1::GenesisState::default(),
        ),
        _ => Err(sdk::error::sdk::not_supported(format!(
            "adding module {name} during the v4 upgrade"
        ))),
    }
}
