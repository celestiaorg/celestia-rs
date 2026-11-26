//! The `v6` handler (the v5 -> v6 upgrade, celestia-app v6.4.x).

use celestia_proto::celestia_core_proto::tendermint::types::EvidenceParams;
use math::LegacyDec;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

/// `appconsts.UnbondingTime` (337 hours).
const UNBONDING_TIME_SECONDS: i64 = 337 * 3600;
/// `appconsts.MaxAgeDuration` (337 hours, CIP-37).
const MAX_AGE_DURATION_SECONDS: i64 = 337 * 3600;
/// `appconsts.MaxAgeNumBlocks` (v6.4.x, CIP-37).
const MAX_AGE_NUM_BLOCKS: i64 = 242_640;

/// celestia-app `IcaAllowMessages`: the messages an interchain account may
/// execute (`app/ica_host.go`).
pub const ICA_ALLOW_MESSAGES: [&str; 12] = [
    "/ibc.applications.transfer.v1.MsgTransfer",
    "/cosmos.bank.v1beta1.MsgSend",
    "/cosmos.staking.v1beta1.MsgDelegate",
    "/cosmos.staking.v1beta1.MsgBeginRedelegate",
    "/cosmos.staking.v1beta1.MsgUndelegate",
    "/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation",
    "/cosmos.distribution.v1beta1.MsgSetWithdrawAddress",
    "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward",
    "/cosmos.distribution.v1beta1.MsgFundCommunityPool",
    "/cosmos.gov.v1.MsgVote",
    "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
    "/cosmos.feegrant.v1beta1.MsgRevokeAllowance",
];

/// celestia-app's ICA host params (`HostEnabled`, `IcaAllowMessages`): the
/// v6 handler's `setICAHostParams` and the `icaModule` default genesis.
#[must_use]
pub fn ica_host_params() -> celestia_proto::ibc::applications::interchain_accounts::host::v1::Params
{
    celestia_proto::ibc::applications::interchain_accounts::host::v1::Params {
        host_enabled: true,
        allow_messages: ICA_ALLOW_MESSAGES.iter().map(|m| (*m).to_owned()).collect(),
    }
}

/// `appconsts.MinCommissionRate` (10%, v6.4.x).
fn min_commission_rate() -> LegacyDec {
    LegacyDec::new_with_prec(10, 2).unwrap_or_default()
}

/// The `v6` upgrade handler: `SetUnbondingTime`, `SetEvidenceParams`,
/// `setICAHostParams`, `SetMinCommissionRate`,
/// `UpdateValidatorCommissionRates`, then `RunMigrations` (which adds the
/// interchain accounts module).
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    let mut params = x_staking::store::get_params(ctx)?;
    params.unbonding_time = pbjson_types::Duration {
        seconds: UNBONDING_TIME_SECONDS,
        nanos: 0,
    };
    x_staking::store::set_params(ctx, &params)?;

    let mut cp = x_consensus::get_params(ctx)?.ok_or_else(|| {
        sdk::error::internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        )
    })?;
    let ev = cp.evidence.get_or_insert_with(EvidenceParams::default);
    ev.max_age_duration = Some(pbjson_types::Duration {
        seconds: MAX_AGE_DURATION_SECONDS,
        nanos: 0,
    });
    ev.max_age_num_blocks = MAX_AGE_NUM_BLOCKS;
    x_consensus::set_params(ctx, &cp)?;

    x_icahost::set_params(ctx, &ica_host_params())?;

    let min = min_commission_rate();
    let mut params = x_staking::store::get_params(ctx)?;
    params.min_commission_rate = min.clone();
    x_staking::store::set_params(ctx, &params)?;

    // `UpdateValidatorCommissionRates`: x/distribution's
    // `BeforeValidatorModified` hook writes nothing.
    for mut v in x_staking::store::get_all_validators(ctx)? {
        if v.commission.rate >= min && v.commission.max_rate >= min {
            continue;
        }
        if v.commission.rate < min {
            v.commission.rate = min.clone();
        }
        if v.commission.max_rate < min {
            v.commission.max_rate = min.clone();
        }
        v.commission.update_time = ctx.header.time;
        x_staking::store::set_validator(ctx, &v)?;
    }

    super::run_migrations_to(
        ctx,
        &from,
        super::release_version_map(6).unwrap_or_default(),
    )
}
