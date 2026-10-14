//! `celestia-rs rollback [--hard]`: Go `celestia-appd rollback` — rolls
//! the consensus state back one height (celestia-core `state.Rollback`)
//! and the application state to the same height (`RollbackToVersion`).
//! `--hard` also deletes the last block from the block store.
//!
//! The privval state is untouched: a validator that signed height `n`
//! refuses to sign a different block at `n` after the rollback, which is
//! the double-sign protection working as intended. Re-executing the same
//! block reproduces the same votes.

use std::path::Path;

use anyhow::{anyhow, bail};
use nodestore::{BlockStore, State, StateStore};

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

/// `rollback`: consensus state one height back, app state to the same
/// height. Returns `(height, app hash)`.
pub fn rollback(
    home: &Path,
    remove_block: bool,
    app_options: app::AppOptions,
) -> anyhow::Result<(i64, Vec<u8>)> {
    let (bs, ss) = crate::node::open_stores(home)?;
    let (height, hash) = rollback_state(&bs, &ss, remove_block)?;
    drop((bs, ss));
    let mut app =
        app::App::open_with(home, app_options).map_err(|e| anyhow!("open app state: {}", e.log))?;
    let app_height = app.info().last_block_height;
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
    Ok((height, hash))
}
