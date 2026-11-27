//! Per-phase profile of block execution (`CELESTIA_PHASE_PROFILE=1`): time,
//! committed IAVL reads (of which tree reads, the rest served by the
//! block's read cache) and node-cache lookups, summed per named phase. For
//! benchmarks (`replay_bench`); off, a span is one branch.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static READS: AtomicU64 = AtomicU64::new(0);
static TREE_READS: AtomicU64 = AtomicU64::new(0);
static TOTALS: Mutex<BTreeMap<&'static str, Totals>> = Mutex::new(BTreeMap::new());

/// Whether profiling is on (`CELESTIA_PHASE_PROFILE=1`).
#[must_use]
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CELESTIA_PHASE_PROFILE").is_ok_and(|v| v == "1"))
}

/// Counts a committed IAVL read (`tree`: it walked the tree).
pub(crate) fn count_read(tree: bool) {
    READS.fetch_add(1, Ordering::Relaxed);
    if tree {
        TREE_READS.fetch_add(1, Ordering::Relaxed);
    }
}

fn counters() -> [u64; 3] {
    [
        READS.load(Ordering::Relaxed),
        TREE_READS.load(Ordering::Relaxed),
        multistore::node_lookups(),
    ]
}

/// Sums of one phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Spans recorded.
    pub count: u64,
    /// Time inside them.
    pub time: Duration,
    /// Committed IAVL reads.
    pub reads: u64,
    /// Of those, reads that walked the tree.
    pub tree_reads: u64,
    /// Node-cache lookups.
    pub lookups: u64,
}

/// A running phase; recorded when dropped.
#[derive(Debug)]
pub struct Span {
    name: &'static str,
    start: Instant,
    at: [u64; 3],
}

/// Starts phase `name` (`None` when profiling is off).
#[must_use]
pub fn span(name: &'static str) -> Option<Span> {
    enabled().then(|| Span {
        name,
        start: Instant::now(),
        at: counters(),
    })
}

impl Drop for Span {
    fn drop(&mut self) {
        let now = counters();
        let mut totals = TOTALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let t = totals.entry(self.name).or_default();
        t.count += 1;
        t.time += self.start.elapsed();
        t.reads += now[0] - self.at[0];
        t.tree_reads += now[1] - self.at[1];
        t.lookups += now[2] - self.at[2];
    }
}

/// The totals so far, by phase name, and resets them.
#[must_use]
pub fn take() -> Vec<(&'static str, Totals)> {
    std::mem::take(
        &mut *TOTALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
    .into_iter()
    .collect()
}
