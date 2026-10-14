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
        "blob" | "circuit" | "fibre" | "gov" | "hyperlane" | "minfee" | "mint" | "warp"
            if genesis.is_null() => {}
        "blob" => x_blob::init_genesis_json(ctx, genesis)?,
        "circuit" => x_circuit::init_genesis_json(ctx, genesis)?,
        "minfee" => x_minfee::init_genesis_json(ctx, genesis)?,
        "mint" => x_mint::init_genesis_json(ctx, genesis)?,
        "fibre" => x_fibre::init_genesis_json(ctx, genesis)?,
        "gov" => x_gov::init_genesis_json(ctx, genesis)?,
        // x/params has no genesis state.
        "params" => x_params::init_genesis(ctx)?,
        "capability" => x_capability::init_genesis_json(ctx, genesis)?,
        "ibc" => x_ibc::init_genesis_json(ctx, genesis)?,
        "transfer" => x_transfer::init_genesis_json(ctx, genesis, &mut TransferBank)?,
        "interchainaccounts" => x_icahost::init_genesis_json(ctx, genesis)?,
        "packetfowardmiddleware" => x_packetforward::init_genesis_json(ctx, genesis)?,
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
        "warp" => x_warp::init_genesis_json(ctx, genesis)?,
        _ => tracing::trace!(module, "init_genesis not ported"),
    }
    let _ = ctx;
    Ok(Vec::new())
}

/// Runs one module's `PreBlock`.
pub fn pre_block(ctx: &mut Context<'_>, module: &str) -> Result<()> {
    match module {
        "upgrade" => x_upgrade::pre_block(ctx),
        _ => Ok(()),
    }
}

/// Runs one module's `BeginBlock`.
pub fn begin_block(ctx: &mut Context<'_>, module: &str) -> Result<()> {
    match module {
        "mint" => x_mint::begin_block(ctx),
        "fibre" => x_fibre::begin_block(ctx),
        "capability" => x_capability::begin_block(ctx),
        "ibc" => x_ibc::begin_block(ctx),
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
        "gov" => x_gov::end_block(ctx)?,
        _ => tracing::trace!(module, "end_block not ported"),
    }
    Ok(Vec::new())
}
