//! `celestia-rs rollback [--height N] [--hard] [--force]`: Go
//! `celestia-appd rollback` — checks the retained block, consensus and app
//! histories, then rolls all three back to the same height. `--height 0`
//! (the default) means one executed height. `--hard` also deletes the block
//! after the target and is limited to one height. More than 100 heights
//! requires `--force`.
//!
//! The privval state is untouched: a validator that signed height `n`
//! refuses to sign a different block at `n` after the rollback, which is
//! the double-sign protection working as intended. Re-executing the same
//! block reproduces the same votes.

use std::path::Path;

use anyhow::{anyhow, bail};
use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use nodestore::{BlockStore, State, StateStore};
use prost::Message;

/// Deepest operator rollback accepted without `--force`.
pub const MAX_ROLLBACK_DEPTH: i64 = 100;

fn app_version_at(view: &sdk::MultiStore) -> anyhow::Result<u64> {
    let params = view
        .get(x_consensus::STORE_KEY, x_consensus::PARAMS_KEY)
        .map_err(|e| anyhow!("historical consensus params: {e}"))?
        .map(|bytes| {
            ConsensusParams::decode(bytes.as_slice())
                .map_err(|e| anyhow!("decode historical consensus params: {e}"))
        })
        .transpose()?;
    Ok(params
        .and_then(|params| params.version)
        .map_or(app::INITIAL_APP_VERSION, |version| version.app))
}

/// The retained histories and target selected by a checked rollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackInfo {
    /// First retained block height.
    pub block_base: i64,
    /// Latest retained block height.
    pub block_height: i64,
    /// Latest executed consensus-state height.
    pub state_height: i64,
    /// Requested final consensus and app height.
    pub target_height: i64,
    /// App hash after executing `target_height`.
    pub target_app_hash: Vec<u8>,
    /// Application protocol version at `state_height`.
    pub current_app_version: u64,
    /// Application protocol version at `target_height`.
    pub target_app_version: u64,
}

/// Resolves the default target (one executed height) without writing.
pub fn default_rollback_height(bs: &BlockStore, ss: &StateStore) -> anyhow::Result<i64> {
    let state = ss
        .load()
        .map_err(|e| anyhow!("{e}"))?
        .filter(|state| !state.is_empty())
        .ok_or_else(|| anyhow!("no state found"))?;
    let block_height = bs.height();
    if block_height == state.last_block_height {
        return Ok(state.last_block_height - 1);
    }
    if block_height == state.last_block_height + 1 {
        bail!(
            "block saved but not executed at height {block_height}; use --height {} to keep it or --height {} to roll back state",
            state.last_block_height,
            state.last_block_height - 1
        );
    }
    bail!(
        "statestore height ({}) is not one below or equal to blockstore height ({block_height})",
        state.last_block_height
    )
}

/// Checks all records needed to reach `target` without writing.
pub fn check_rollback_to_height(
    bs: &BlockStore,
    ss: &StateStore,
    target: i64,
    remove_block: bool,
) -> anyhow::Result<RollbackInfo> {
    let state = ss
        .load()
        .map_err(|e| anyhow!("{e}"))?
        .filter(|state| !state.is_empty())
        .ok_or_else(|| anyhow!("no state found"))?;
    let mut info = RollbackInfo {
        block_base: bs.base(),
        block_height: bs.height(),
        state_height: state.last_block_height,
        target_height: target,
        target_app_hash: Vec::new(),
        current_app_version: state.version.app,
        target_app_version: 0,
    };
    if info.block_height != info.state_height && info.block_height != info.state_height + 1 {
        bail!(
            "statestore height ({}) is not one below or equal to blockstore height ({})",
            info.state_height,
            info.block_height
        );
    }
    if target < info.block_base || target > info.state_height {
        bail!(
            "target height {target} out of range [{}, {}]",
            info.block_base,
            info.state_height
        );
    }
    if remove_block && info.state_height - target > 1 {
        bail!("--hard rolls back at most one height");
    }

    for height in target..=info.block_height {
        let meta = bs
            .load_block_meta(height)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("block meta at height {height} not found"))?;
        if meta.header.height != height {
            bail!(
                "block meta at height {height} has height {}",
                meta.header.height
            );
        }
    }
    for height in target..info.state_height {
        ss.load_validators(height)
            .map_err(|e| anyhow!("validator set at height {height}: {e}"))?;
        ss.load_consensus_params(height + 1)
            .map_err(|e| anyhow!("consensus params at height {}: {e}", height + 1))?;
    }

    if target == info.state_height {
        info.target_app_hash = state.app_hash;
        info.target_app_version = state.version.app;
    } else {
        info.target_app_hash = bs
            .load_block_meta(target + 1)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("block meta at height {} not found", target + 1))?
            .header
            .app_hash;
        info.target_app_version = ss
            .load_consensus_params(target + 1)
            .map_err(|e| anyhow!("consensus params at height {}: {e}", target + 1))?
            .version
            .app;
    }
    Ok(info)
}

/// Rolls the state store back one height; returns the new height and
/// app hash (`state.Rollback`).
pub fn rollback_state(
    bs: &BlockStore,
    ss: &StateStore,
    remove_block: bool,
) -> anyhow::Result<(i64, Vec<u8>)> {
    let invalid = ss
        .load()
        .map_err(|e| anyhow!("{e}"))?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("no state found"))?;
    let height = bs.height();
    // Persistence of state and blocks is not atomic: a block one above the
    // state is discarded (with --hard) and the state stays.
    if height == invalid.last_block_height + 1 {
        if remove_block {
            bs.delete_latest_block()
                .map_err(|e| anyhow!("failed to remove final block from blockstore: {e}"))?;
        }
        return Ok((invalid.last_block_height, invalid.app_hash));
    }
    if height != invalid.last_block_height {
        bail!(
            "statestore height ({}) is not one below or equal to blockstore height ({height})",
            invalid.last_block_height
        );
    }
    let rollback_height = invalid.last_block_height - 1;
    let rollback_block = bs
        .load_block_meta(rollback_height)
        .map_err(|e| anyhow!("{e}"))?
        .ok_or_else(|| anyhow!("block at height {rollback_height} not found"))?;
    let latest_block = bs
        .load_block_meta(invalid.last_block_height)
        .map_err(|e| anyhow!("{e}"))?
        .ok_or_else(|| anyhow!("block at height {} not found", invalid.last_block_height))?;
    let previous_last_validators = ss
        .load_validators(rollback_height)
        .map_err(|e| anyhow!("{e}"))?;
    let previous_params = ss
        .load_consensus_params(rollback_height + 1)
        .map_err(|e| anyhow!("{e}"))?;
    let next_height = rollback_height + 1;
    let mut val_change_height = invalid.last_height_validators_changed;
    if val_change_height > next_height + 1 {
        val_change_height = next_height + 1;
    }
    let mut params_change_height = invalid.last_height_consensus_params_changed;
    if params_change_height > rollback_height {
        params_change_height = rollback_height + 1;
    }
    let mut version = invalid.version;
    version.app = previous_params.version.app;
    let rolled = State {
        version,
        software: invalid.software.clone(),
        chain_id: invalid.chain_id.clone(),
        initial_height: invalid.initial_height,
        last_block_height: rollback_block.header.height,
        last_block_id: rollback_block.block_id.clone(),
        last_block_time: rollback_block.header.time,
        next_validators: invalid.validators.clone(),
        validators: invalid.last_validators.clone(),
        last_validators: previous_last_validators,
        last_height_validators_changed: val_change_height,
        consensus_params: previous_params,
        last_height_consensus_params_changed: params_change_height,
        last_results_hash: latest_block.header.last_results_hash.clone(),
        app_hash: latest_block.header.app_hash.clone(),
        timeouts: invalid.timeouts,
    };
    ss.save(&rolled)
        .map_err(|e| anyhow!("failed to save rolled back state: {e}"))?;
    if remove_block {
        bs.delete_latest_block()
            .map_err(|e| anyhow!("failed to remove final block from blockstore: {e}"))?;
    }
    Ok((rolled.last_block_height, rolled.app_hash))
}

/// Rolls consensus state to `target`, deleting blocks above `target + 1`
/// (`target + 1` too for `remove_block`). Safe to resume after any completed
/// state-save or block-delete step.
pub fn rollback_state_to_height(
    bs: &BlockStore,
    ss: &StateStore,
    target: i64,
    remove_block: bool,
) -> anyhow::Result<(i64, Vec<u8>)> {
    let info = check_rollback_to_height(bs, ss, target, remove_block)?;
    loop {
        let state = ss
            .load()
            .map_err(|e| anyhow!("{e}"))?
            .filter(|state| !state.is_empty())
            .ok_or_else(|| anyhow!("no state found"))?;
        let block_height = bs.height();
        if block_height == state.last_block_height + 1
            && (block_height > target + 1 || (remove_block && block_height == target + 1))
        {
            bs.delete_latest_block()
                .map_err(|e| anyhow!("failed to remove block {block_height}: {e}"))?;
            if bs.height() != block_height - 1 {
                bail!(
                    "blockstore height is {} after removing block {block_height}",
                    bs.height()
                );
            }
        } else if state.last_block_height > target {
            let old_height = state.last_block_height;
            let delete_step_block = old_height - 1 > target || remove_block;
            let (new_height, _) = rollback_state(bs, ss, delete_step_block)?;
            if new_height != old_height - 1 {
                bail!("rollback from height {old_height} returned height {new_height}");
            }
        } else {
            if state.last_block_height != target {
                bail!(
                    "rollback passed target height {target}: state height is {}",
                    state.last_block_height
                );
            }
            if state.app_hash != info.target_app_hash {
                bail!(
                    "app hash mismatch at height {}: got {}, want {}",
                    state.last_block_height,
                    hex::encode_upper(&state.app_hash),
                    hex::encode_upper(&info.target_app_hash)
                );
            }
            return Ok((state.last_block_height, state.app_hash));
        }
    }
}

/// Refuses an unexpectedly deep operator rollback unless `force` is set.
pub fn check_rollback_depth(info: &RollbackInfo, force: bool) -> anyhow::Result<()> {
    let depth = info.state_height - info.target_height;
    if depth > MAX_ROLLBACK_DEPTH && !force {
        bail!("rollback depth {depth} exceeds {MAX_ROLLBACK_DEPTH}; use --force");
    }
    Ok(())
}

/// `rollback`: consensus state one height back, app state to the same
/// height. Returns `(height, app hash)`.
pub fn rollback_to_height(
    home: &Path,
    requested_height: i64,
    remove_block: bool,
    force: bool,
    app_options: app::AppOptions,
) -> anyhow::Result<(i64, Vec<u8>)> {
    let (bs, ss) = crate::node::open_stores(home)?;
    let target = if requested_height == 0 {
        default_rollback_height(&bs, &ss)?
    } else {
        requested_height
    };
    let checked = check_rollback_to_height(&bs, &ss, target, remove_block)?;
    check_rollback_depth(&checked, force)?;

    // Establish that the target app version is retained and has the exact
    // consensus hash before changing either consensus or application state.
    let mut app =
        app::App::open_with(home, app_options).map_err(|e| anyhow!("open app state: {}", e.log))?;
    let app_height = app.info().last_block_height;
    if app_height < target {
        bail!("app state height {app_height} is below rollback target {target}");
    }
    let app_view = app
        .multistore()
        .view_at(target)
        .map_err(|e| anyhow!("app state at height {target}: {e}"))?;
    let app_commit = app_view
        .root()
        .last_commit_id()
        .map_err(|e| anyhow!("app commit at height {target}: {e}"))?;
    if app_commit.hash.as_slice() != checked.target_app_hash.as_slice() {
        bail!(
            "app state at height {target} has hash {}, expected {}",
            hex::encode_upper(app_commit.hash),
            hex::encode_upper(&checked.target_app_hash)
        );
    }
    let historical_app_version = app_version_at(&app_view)?;
    if historical_app_version != checked.target_app_version {
        bail!(
            "app version at height {target} is {historical_app_version}, expected {}",
            checked.target_app_version
        );
    }
    drop(app_view);

    let (height, hash) = rollback_state_to_height(&bs, &ss, target, remove_block)?;
    drop((bs, ss));
    if app_height > height {
        app.rollback_to(height)
            .map_err(|e| anyhow!("failed to rollback to version: {}", e.log))?;
    }
    let info = app.info();
    if info.last_block_height != height || info.last_block_app_hash != hash {
        bail!(
            "app state at height {} (hash {}) does not match the rolled-back consensus state at {height} (hash {})",
            info.last_block_height,
            hex::encode_upper(&info.last_block_app_hash),
            hex::encode_upper(&hash)
        );
    }
    if info.app_version != checked.target_app_version {
        bail!(
            "app version at height {height} is {}, expected {}",
            info.app_version,
            checked.target_app_version
        );
    }
    Ok((height, hash))
}

/// Backward-compatible one-height rollback.
pub fn rollback(
    home: &Path,
    remove_block: bool,
    app_options: app::AppOptions,
) -> anyhow::Result<(i64, Vec<u8>)> {
    rollback_to_height(home, 0, remove_block, false, app_options)
}
