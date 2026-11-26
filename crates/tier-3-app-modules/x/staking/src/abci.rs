//! Block hooks (`keeper/abci.go`, `keeper/historical_info.go`,
//! `BlockValidatorUpdates`).

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::celestia_core_proto::tendermint::types::{BlockId, Header, PartSetHeader};
use celestia_proto::celestia_core_proto::tendermint::version::Consensus;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use prost::Message;
use sdk::event::event;
use sdk::{Context, Result};

use crate::delegation::{complete_redelegation, complete_unbonding};
use crate::store;
use crate::validator::{apply_and_return_validator_set_updates, unbond_all_mature_validators};
use crate::{STORE_KEY, keys};

/// The block header as a CometBFT proto (`sdkCtx.BlockHeader()`), with the
/// fields baseapp sets in `FinalizeBlock`.
#[must_use]
pub fn cmt_header(ctx: &Context<'_>) -> Header {
    let h = &ctx.header;
    Header {
        version: Some(Consensus::default()),
        chain_id: h.chain_id.clone(),
        height: h.height,
        time: Some(h.time),
        last_block_id: Some(BlockId {
            hash: Vec::new(),
            part_set_header: Some(PartSetHeader::default()),
        }),
        next_validators_hash: h.next_validators_hash.clone(),
        app_hash: h.app_hash.clone(),
        proposer_address: h.proposer_address.clone(),
        ..Header::default()
    }
}

/// `TrackHistoricalInfo`, storing the header baseapp sets in
/// `FinalizeBlock`.
pub fn track_historical_info(ctx: &mut Context<'_>) -> Result<()> {
    let header = cmt_header(ctx);
    track_historical_info_with(ctx, header)
}

/// `TrackHistoricalInfo` with the given block header.
fn track_historical_info_with(ctx: &mut Context<'_>, header: Header) -> Result<()> {
    let entries = i64::from(store::historical_entries(ctx)?);
    let mut i = ctx.header.height - entries;
    while i >= 0 {
        if store::get_historical_info(ctx, i)?.is_none() {
            break;
        }
        let key = store::historical_info_key(ctx, i);
        ctx.delete(STORE_KEY, &key)?;
        i -= 1;
    }
    if entries == 0 {
        return Ok(());
    }
    let mut vals = store::get_last_validators(ctx)?;
    // ValidatorsByVotingPower: power descending, then consensus address.
    let mut keyed = Vec::with_capacity(vals.len());
    for v in vals.drain(..) {
        let p = v.consensus_power()?;
        let a = v.cons_addr().unwrap_or_default();
        keyed.push((p, a, v));
    }
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let hi = pb::HistoricalInfo {
        header: Some(header),
        valset: keyed.into_iter().map(|(_, _, v)| v.to_proto()).collect(),
    };
    let h = ctx.header.height;
    let key = store::historical_info_key(ctx, h);
    ctx.set(STORE_KEY, key, hi.encode_to_vec())
}

/// `BeginBlocker`. The v5 store migrations continue from their cursors
/// (a chain that never ran the v0.46 to v0.50 upgrade has none): 10,000
/// delegations into the by-validator index, then 1,000 historical info
/// keys. Go recovers from their panics and ignores their errors.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    if let Some(index) = ctx.get(STORE_KEY, keys::NEXT_MIGRATE_DELEGATIONS_BY_VALIDATOR_INDEX)?
        && let Err(e) = crate::migrations::index_delegations_by_validator(ctx, Some(&index), 10_000)
    {
        tracing::error!(error = %e.log, "x/staking delegation index migration");
    }
    if let Some(index) = ctx.get(STORE_KEY, keys::NEXT_MIGRATE_HISTORICAL_INFO)?
        && let Err(e) = crate::migrations::migrate_historical_info_keys(ctx, Some(&index), 1_000)
    {
        tracing::error!(error = %e.log, "x/staking historical info migration");
    }
    track_historical_info(ctx)
}

/// cosmos-sdk v0.46 `BeginBlocker`: `TrackHistoricalInfo` with the full
/// Tendermint v0.34 BeginBlock header (celestia-app v1 to v3).
pub fn begin_block_v046(ctx: &mut Context<'_>, header: &Header) -> Result<()> {
    track_historical_info_with(ctx, header.clone())
}

/// `EndBlocker` (`BlockValidatorUpdates`).
pub fn end_block(ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
    let updates = apply_and_return_validator_set_updates(ctx)?;
    unbond_all_mature_validators(ctx)?;
    let now = ctx.header.time;
    for pair in store::dequeue_all_mature_ubd_queue(ctx, &now)? {
        let val = crate::addr::val_from_string(&pair.validator_address)?;
        let del = crate::addr::acc_from_string(&pair.delegator_address)?;
        let Ok(balances) = complete_unbonding(ctx, &del, &val) else {
            continue;
        };
        ctx.events.emit(event(
            "complete_unbonding",
            [
                ("amount", balances.to_string()),
                ("validator", pair.validator_address.clone()),
                ("delegator", pair.delegator_address.clone()),
            ],
        ));
    }
    for t in store::dequeue_all_mature_red_queue(ctx, &now)? {
        let src = crate::addr::val_from_string(&t.validator_src_address)?;
        let dst = crate::addr::val_from_string(&t.validator_dst_address)?;
        let del = crate::addr::acc_from_string(&t.delegator_address)?;
        let Ok(balances) = complete_redelegation(ctx, &del, &src, &dst) else {
            continue;
        };
        ctx.events.emit(event(
            "complete_redelegation",
            [
                ("amount", balances.to_string()),
                ("delegator", t.delegator_address.clone()),
                ("source_validator", t.validator_src_address.clone()),
                ("destination_validator", t.validator_dst_address.clone()),
            ],
        ));
    }
    Ok(updates)
}
