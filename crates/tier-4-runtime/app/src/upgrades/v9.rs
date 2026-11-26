//! The `v9` handler (the v8 -> v9 upgrade, celestia-app v9.0.8).

use celestia_proto::celestia_core_proto::tendermint::types::{BlockParams, EvidenceParams};
use sdk::{Context, Result};
use x_upgrade::VersionMap;

use super::run_migrations;

/// `appconsts.BlockMaxBytes` (32 MiB).
const BLOCK_MAX_BYTES: i64 = 32 * 1024 * 1024;
/// `appconsts.MaxAgeNumBlocks` (v9.0.8).
const MAX_AGE_NUM_BLOCKS: i64 = 559_940;
/// `appconsts.MaxSquareSize` (the blob `GovMaxSquareSize` the handler sets).
const MAX_SQUARE_SIZE: u64 = 256;
/// `appconsts.MaxExpectedTimePerBlock` (13 s), in nanoseconds.
const MAX_EXPECTED_TIME_PER_BLOCK_NS: u64 = 13_000_000_000;

/// The `v9` upgrade handler: `SetBlockMaxBytes`,
/// `SetEvidenceMaxAgeNumBlocks`, `SetGovMaxSquareSize`,
/// `SetMaxExpectedTimePerBlock`, then `RunMigrations`. Without the `fibre`
/// build tag v9.0.8 adds no store.
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    let mut cp = consensus_params(ctx)?;
    cp.block.get_or_insert_with(BlockParams::default).max_bytes = BLOCK_MAX_BYTES;
    x_consensus::set_params(ctx, &cp)?;
    let mut cp = consensus_params(ctx)?;
    cp.evidence
        .get_or_insert_with(EvidenceParams::default)
        .max_age_num_blocks = MAX_AGE_NUM_BLOCKS;
    x_consensus::set_params(ctx, &cp)?;
    let mut blob = x_blob::get_params(ctx)?;
    blob.gov_max_square_size = MAX_SQUARE_SIZE;
    x_blob::set_params(ctx, &blob)?;
    x_ibc::connection::set_params(
        ctx,
        &celestia_proto::ibc::core::connection::v1::Params {
            max_expected_time_per_block: MAX_EXPECTED_TIME_PER_BLOCK_NS,
        },
    )?;
    run_migrations(ctx, &from, 9)
}

/// `ConsensusKeeper.ParamsStore.Get`.
fn consensus_params(
    ctx: &mut Context<'_>,
) -> Result<celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams> {
    x_consensus::get_params(ctx)?.ok_or_else(|| {
        sdk::error::internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{
        BlockParams, ConsensusParams, EvidenceParams, VersionParams,
    };
    use sdk::{ExecMode, Header, MultiStore};

    /// celestia-app v9.0.8's handler: `BlockMaxBytes` 32 MiB, evidence
    /// `MaxAgeNumBlocks` 559,940 (the duration stays), blob
    /// `GovMaxSquareSize` 256 (gas per blob byte stays), IBC connection
    /// params `{MaxExpectedTimePerBlock: 13s}`, then `RunMigrations`, which
    /// changes no version from v8.0.8's map and adds no module.
    #[test]
    fn v9_handler_sets_the_v9_params() {
        let mut ms = MultiStore::new();
        crate::stores::mount_release(&mut ms, sdk::version::Release::for_app_version(9).unwrap())
            .unwrap();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let before = ConsensusParams {
            block: Some(BlockParams {
                max_bytes: 8 * 1024 * 1024,
                max_gas: -1,
            }),
            evidence: Some(EvidenceParams {
                max_age_num_blocks: 242_640,
                max_age_duration: Some(pbjson_types::Duration {
                    seconds: 337 * 3600,
                    nanos: 0,
                }),
                max_bytes: 1_048_576,
            }),
            version: Some(VersionParams { app: 9 }),
            ..Default::default()
        };
        x_consensus::set_params(&mut ctx, &before).unwrap();
        x_blob::set_params(
            &mut ctx,
            &celestia_proto::celestia::blob::v1::Params {
                gas_per_blob_byte: 8,
                gov_max_square_size: 128,
            },
        )
        .unwrap();
        x_ibc::connection::set_params(
            &mut ctx,
            &celestia_proto::ibc::core::connection::v1::Params {
                max_expected_time_per_block: 75_000_000_000,
            },
        )
        .unwrap();
        let from: VersionMap = crate::modules::version_map(9)
            .filter(|(m, _)| crate::stores::exists_at(m, 9))
            .map(|(m, v)| (m.to_owned(), v))
            .collect();
        let out = handler(&mut ctx, from.clone()).unwrap();
        assert_eq!(out, from);

        let cp = x_consensus::get_params(&mut ctx).unwrap().unwrap();
        let mut want = before;
        want.block.as_mut().unwrap().max_bytes = 32 * 1024 * 1024;
        want.evidence.as_mut().unwrap().max_age_num_blocks = 559_940;
        assert_eq!(cp, want);
        let blob = x_blob::get_params(&mut ctx).unwrap();
        assert_eq!((blob.gas_per_blob_byte, blob.gov_max_square_size), (8, 256));
        assert_eq!(
            x_ibc::connection::get_params(&mut ctx)
                .unwrap()
                .max_expected_time_per_block,
            13_000_000_000
        );
    }
}
