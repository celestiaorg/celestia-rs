//! Module ordering and dispatch (`app/modules.go`).
//!
//! Every module named in Go's order lists appears here. Modules not yet
//! ported are explicit no-ops so the differential harness can attribute
//! each diverging store to the module that owns it.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use sdk::{Context, Result};

/// `SetOrderInitGenesis` (also export order).
pub const INIT_GENESIS: &[&str] = &[
    "capability",
    "consensus",
    "auth",
    "bank",
    "distribution",
    "staking",
    "slashing",
    "gov",
    "mint",
    "ibc",
    "minfee",
    "genutil",
    "evidence",
    "transfer",
    "blob",
    "vesting",
    "feegrant",
    "params",
    "authz",
    "signal",
    "packetfowardmiddleware",
    "interchainaccounts",
    "upgrade",
    "circuit",
    "hyperlane",
    "warp",
    "zkism",
    "forwarding",
    "fibre",
    "valaddr",
];

/// `SetOrderPreBlockers`.
pub const PRE_BLOCKERS: &[&str] = &["upgrade"];

/// `SetOrderBeginBlockers`.
pub const BEGIN_BLOCKERS: &[&str] = &[
    "capability",
    "mint",
    "distribution",
    "slashing",
    "evidence",
    "staking",
    "ibc",
    "transfer",
    "genutil",
    "blob",
    "params",
    "authz",
    "signal",
    "minfee",
    "interchainaccounts",
    "packetfowardmiddleware",
    "zkism",
    "fibre",
    "valaddr",
];

/// `SetOrderEndBlockers`.
pub const END_BLOCKERS: &[&str] = &[
    "gov",
    "staking",
    "capability",
    "mint",
    "distribution",
    "slashing",
    "evidence",
    "ibc",
    "transfer",
    "feegrant",
    "auth",
    "bank",
    "genutil",
    "blob",
    "params",
    "authz",
    "vesting",
    "signal",
    "minfee",
    "packetfowardmiddleware",
    "interchainaccounts",
    "fibre",
    "valaddr",
];

/// `ModuleManager.GetVersionMap()`: every module with its consensus version.
pub const VERSION_MAP: &[(&str, u64)] = &[
    ("06-solomachine", 0),
    ("07-tendermint", 0),
    ("auth", x_auth::CONSENSUS_VERSION),
    ("authz", 2),
    ("bank", x_bank::CONSENSUS_VERSION),
    ("blob", 3),
    ("capability", 1),
    ("circuit", 1),
    ("consensus", x_consensus::CONSENSUS_VERSION),
    ("distribution", 4),
    ("evidence", 1),
    ("feegrant", 2),
    ("fibre", 1),
    ("forwarding", 1),
    ("genutil", 1),
    ("gov", 5),
    ("hyperlane", x_hyperlane::CONSENSUS_VERSION),
    ("ibc", 6),
    ("interchainaccounts", 3),
    ("minfee", 2),
    ("mint", 1),
    ("packetfowardmiddleware", 3),
    ("params", 1),
    ("signal", 3),
    ("slashing", 4),
    ("staking", 5),
    ("transfer", 5),
    ("upgrade", x_upgrade::CONSENSUS_VERSION),
    ("valaddr", 1),
    ("vesting", 1),
    ("warp", x_warp::CONSENSUS_VERSION),
    ("zkism", 1),
];

/// Runs one module's `InitGenesis`.
pub fn init_genesis(
    ctx: &mut Context<'_>,
    module: &str,
    genesis: &serde_json::Value,
) -> Result<Vec<ValidatorUpdate>> {
    match module {
        // Consensus params are stored by baseapp before the init chainer.
        "consensus" | "upgrade" => {}
        "auth" => x_auth::init_genesis(ctx, genesis)?,
        "bank" => x_bank::init_genesis(ctx, genesis)?,
        // Go's module manager skips modules whose genesis section is null.
        "blob" | "circuit" | "fibre" | "forwarding" | "gov" | "hyperlane" | "minfee" | "mint"
        | "valaddr" | "warp" | "zkism"
            if genesis.is_null() => {}
        "blob" => x_blob::init_genesis_json(ctx, genesis)?,
        "circuit" => x_circuit::init_genesis_json(ctx, genesis)?,
        "minfee" => x_minfee::init_genesis_json(ctx, genesis)?,
        "mint" => x_mint::init_genesis_json(ctx, genesis)?,
        "fibre" => x_fibre::init_genesis_json(ctx, genesis)?,
        "gov" => x_gov::init_genesis_json(ctx, genesis)?,
        "zkism" => x_zkism::init_genesis_json(ctx, genesis)?,
        "forwarding" => x_forwarding::init_genesis_json(ctx, genesis)?,
        "valaddr" => x_valaddr::init_genesis_json(ctx, genesis)?,
        // x/params has no genesis state.
        "params" => x_params::init_genesis(ctx)?,
        "capability" => x_capability::init_genesis_json(ctx, genesis)?,
        "ibc" => x_ibc::init_genesis_json(ctx, genesis)?,
        "transfer" => x_transfer::init_genesis_json(ctx, genesis, &mut TransferBank)?,
        "interchainaccounts" => x_icahost::init_genesis_json(ctx, genesis)?,
        "packetfowardmiddleware" => x_packetforward::init_genesis_json(ctx, genesis)?,
        "distribution" => x_distribution::init_genesis(ctx, genesis)?,
        "staking" => return x_staking::init_genesis(ctx, genesis),
        "slashing" => x_slashing::init_genesis(ctx, genesis)?,
        "genutil" => {
            return crate::genutil::init_genesis(
                ctx,
                genesis,
                crate::txpipe::decode_tx,
                crate::txpipe::ante,
                crate::txpipe::apply_validator_set,
            );
        }
        "hyperlane" => x_hyperlane::init_genesis_json(ctx, genesis)?,
        "authz" if !genesis.is_null() => x_authz::init_genesis_json(ctx, genesis)?,
        "feegrant" if !genesis.is_null() => x_feegrant::init_genesis_json(ctx, genesis)?,
        "evidence" if !genesis.is_null() => x_evidence::init_genesis_json(ctx, genesis)?,
        "warp" => x_warp::init_genesis_json(ctx, genesis)?,
        _ => tracing::trace!(module, "init_genesis not ported"),
    }
    let _ = ctx;
    Ok(Vec::new())
}

/// Installs the staking hooks (`NewMultiStakingHooks(distr, slashing)`).
pub fn install_staking_hooks() {
    x_staking::set_hooks(std::sync::Arc::new(x_staking::MultiStakingHooks(vec![
        Box::new(x_distribution::Hooks),
        Box::new(x_slashing::Hooks),
    ])));
}

/// Runs one module's `PreBlock`; returns whether consensus params may have
/// changed.
pub fn pre_block(
    ctx: &mut Context<'_>,
    module: &str,
    upgrade: &mut x_upgrade::NodeState,
) -> Result<bool> {
    match module {
        "upgrade" => {
            let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
            x_upgrade::pre_block(ctx, &crate::upgrades::Handlers::at(app_version), upgrade)
        }
        _ => Ok(false),
    }
}

/// Runs one module's `BeginBlock`.
pub fn begin_block(ctx: &mut Context<'_>, module: &str) -> Result<()> {
    match module {
        "mint" => x_mint::begin_block(ctx),
        "fibre" => x_fibre::begin_block(ctx),
        "capability" => x_capability::begin_block(ctx),
        "ibc" => x_ibc::begin_block(ctx),
        "distribution" => x_distribution::begin_block(ctx),
        "slashing" => x_slashing::begin_block(ctx),
        "staking" => x_staking::begin_block(ctx),
        "authz" => x_authz::begin_block(ctx),
        "evidence" => x_evidence::begin_block(ctx),
        // No begin blocker in Go.
        "transfer" | "interchainaccounts" | "packetfowardmiddleware" => Ok(()),
        _ => {
            tracing::trace!(module, "begin_block not ported");
            Ok(())
        }
    }
}

/// `x/bank` as the transfer module's expected bank keeper.
struct TransferBank;

impl x_transfer::BankKeeper for TransferBank {
    fn set_denom_metadata(
        &mut self,
        ctx: &mut Context<'_>,
        md: &celestia_proto::cosmos::bank::v1beta1::Metadata,
    ) -> Result<()> {
        x_bank::set_denom_metadata(ctx, md)
    }
}

/// Runs one module's `EndBlock`.
pub fn end_block(ctx: &mut Context<'_>, module: &str) -> Result<Vec<ValidatorUpdate>> {
    match module {
        "staking" => return x_staking::end_block(ctx),
        "gov" => x_gov::end_block(ctx, &crate::router::AppRouter)?,
        "feegrant" => x_feegrant::end_block(ctx)?,
        "valaddr" => x_valaddr::end_block(ctx, &mut ValaddrStaking)?,
        _ => tracing::trace!(module, "end_block not ported"),
    }
    Ok(Vec::new())
}

/// `x/staking` as the valaddr module's expected staking keeper.
pub(crate) struct ValaddrStaking;

impl x_valaddr::StakingKeeper for ValaddrStaking {
    fn validator_by_cons_addr(
        &mut self,
        ctx: &mut Context<'_>,
        cons_addr: &[u8],
    ) -> Result<Option<x_valaddr::ValidatorInfo>> {
        Ok(
            x_staking::store::get_validator_by_cons_addr(ctx, cons_addr)?.map(|v| {
                x_valaddr::ValidatorInfo {
                    jailed: v.jailed,
                    bonded: v.is_bonded(),
                    unbonding_time: v.unbonding_time,
                }
            }),
        )
    }

    fn validator_cons_addr(
        &mut self,
        ctx: &mut Context<'_>,
        val_addr: &[u8],
    ) -> Result<Option<core::result::Result<Vec<u8>, String>>> {
        Ok(x_staking::store::get_validator(ctx, val_addr)?
            .map(|v| v.cons_addr().map_err(|e| e.log)))
    }
}

/// `x/staking` as the signal module's expected staking keeper.
pub struct SignalStaking;

impl x_signal::StakingKeeper for SignalStaking {
    fn last_validator_power(&mut self, ctx: &mut Context<'_>, val: &[u8]) -> Result<i64> {
        x_staking::store::get_last_validator_power(ctx, val)
    }

    fn last_total_power(&mut self, ctx: &mut Context<'_>) -> Result<math::Int> {
        x_staking::store::get_last_total_power(ctx)
    }

    fn validator(
        &mut self,
        ctx: &mut Context<'_>,
        val: &[u8],
    ) -> Result<Option<x_signal::ValidatorStatus>> {
        Ok(
            x_staking::store::get_validator(ctx, val)?.map(|v| x_signal::ValidatorStatus {
                bonded: v.is_bonded(),
            }),
        )
    }
}

/// Router entry for `MsgSignalVersion`.
pub fn handle_signal_version(
    ctx: &mut Context<'_>,
    msg: &celestia_proto::celestia::signal::v1::MsgSignalVersion,
) -> Result<celestia_proto::celestia::signal::v1::MsgSignalVersionResponse> {
    x_signal::signal_version(ctx, &mut SignalStaking, msg)
}

/// Router entry for `MsgTryUpgrade`.
pub fn handle_try_upgrade(
    ctx: &mut Context<'_>,
    msg: &celestia_proto::celestia::signal::v1::MsgTryUpgrade,
) -> Result<celestia_proto::celestia::signal::v1::MsgTryUpgradeResponse> {
    x_signal::try_upgrade(ctx, &mut SignalStaking, msg)
}

/// celestia `App.EndBlocker` after the module manager's EndBlock: once a
/// signalled upgrade's height is reached, schedule the `v<N>` plan for the
/// next height, bump the app version and reset the signal tally. The next
/// block's `PreBlocker` then runs the `v<N>` handler, or halts the chain if
/// this binary has none (exactly like Go).
pub fn app_end_block(ctx: &mut Context<'_>) -> Result<()> {
    let current = x_consensus::get_params(ctx)?
        .and_then(|p| p.version)
        .map_or(0, |v| v.app);
    if let Some(up) = x_signal::should_upgrade(ctx)?
        && up.app_version > current
    {
        let plan = celestia_proto::cosmos::upgrade::v1beta1::Plan {
            name: format!("v{}", up.app_version),
            height: up.upgrade_height + 1,
            ..Default::default()
        };
        x_upgrade::schedule_upgrade(ctx, &plan)
            .map_err(|e| sdk::error::internal(format!("failed to schedule upgrade: {}", e.log)))?;
        crate::upgrades::set_app_version(ctx, up.app_version)?;
        x_signal::reset_tally(ctx)?;
    }
    Ok(())
}
