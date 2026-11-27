//! Store steps the service runs around uploads: seeding occupancy at
//! startup (Go `Server.seedOccupancy`) and one prune pass (Go
//! `Server.prune`, which `startPruneLoop` runs every minute).

use std::time::{Instant, SystemTime};

use tokio_util::sync::CancellationToken;

use super::{Store, StoreError};
use crate::occupancy::Occupancy;

/// Outcome of one [`prune_expired`] pass, for logs and metrics.
#[derive(Debug, Default)]
pub struct PruneSummary {
    /// Entries deleted in the pass.
    pub pruned: usize,
    /// Bytes released from occupancy.
    pub freed: i64,
    /// First integrity error (corrupt markers were skipped and kept).
    pub integrity_error: Option<StoreError>,
    /// First payload deletion failure (its metadata is kept for retry).
    pub delete_error: Option<StoreError>,
    /// The error that stopped the pass early (database failure or
    /// cancellation); Go logs it at error level.
    pub error: Option<StoreError>,
}

/// Seeds `occupancy` from [`Store::size`]. An integrity error is logged
/// and the usable partial total is seeded; other errors are returned.
pub fn seed_occupancy(
    store: &Store,
    occupancy: &Occupancy,
    cancel: &CancellationToken,
) -> Result<(), StoreError> {
    let size = store.size(cancel)?;
    if let Some(e) = &size.integrity_error {
        tracing::warn!(error = %e, "store size may be incorrect due to corrupt shard marker");
    }
    occupancy.seed(size.bytes);
    Ok(())
}

/// One prune pass: prunes every entry whose prune time is before `now`, in
/// batches of [`super::MAX_PRUNE_BATCH_SIZE`], and releases the freed bytes
/// from `occupancy`. Integrity errors and payload deletion failures do not
/// stop the pass; database errors and cancellation do.
pub fn prune_expired(
    store: &Store,
    occupancy: &Occupancy,
    now: SystemTime,
    cancel: &CancellationToken,
) -> PruneSummary {
    let start = Instant::now();
    let mut summary = PruneSummary::default();
    let mut cursor: Option<Vec<u8>> = None;
    loop {
        let batch = store.prune_batch(now, cursor.as_deref(), cancel);
        summary.pruned += batch.pruned;
        if batch.freed > 0 {
            occupancy.release(batch.freed);
            summary.freed += batch.freed;
        }
        if let Some(err) = batch.error {
            // Split a partial deletion from the integrity errors it carries.
            let rest = match err {
                StoreError::PartialDelete {
                    deletion,
                    integrity,
                } => {
                    summary.delete_error.get_or_insert(*deletion);
                    integrity.map(|e| *e)
                }
                err if !err.is_integrity() || cancel.is_cancelled() => {
                    tracing::error!(error = %err, elapsed_ms = start.elapsed().as_millis(), "failed to prune store");
                    summary.error = Some(err);
                    return summary;
                }
                err => Some(err),
            };
            if let Some(e) = rest.filter(StoreError::is_integrity) {
                summary.integrity_error.get_or_insert(e);
            }
        }
        match batch.next {
            Some(next) if !cancel.is_cancelled() => cursor = Some(next),
            _ => break,
        }
    }
    let elapsed_ms = start.elapsed().as_millis();
    if let Some(e) = &summary.integrity_error {
        tracing::warn!(error = %e, elapsed_ms, "prune skipped corrupt shard markers");
    }
    if let Some(e) = &summary.delete_error {
        tracing::warn!(error = %e, elapsed_ms, "prune retained failed payload deletions");
    }
    if summary.pruned > 0 {
        tracing::info!(
            pruned = summary.pruned,
            elapsed_ms,
            "pruned expired entries"
        );
    }
    summary
}
