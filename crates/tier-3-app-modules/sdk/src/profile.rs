//! Per-phase profile of block execution (`CELESTIA_PHASE_PROFILE=1`): time,
//! committed IAVL reads and node-cache lookups, summed per named phase. For
//! benchmarks (`replay_bench`); off, a span is one branch.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static READS: AtomicU64 = AtomicU64::new(0);
static TOTALS: Mutex<BTreeMap<&'static str, Totals>> = Mutex::new(BTreeMap::new());

/// 0: not decided yet (read the environment), 1: off, 2: on.
static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether profiling is on: [`set_enabled`], else `CELESTIA_PHASE_PROFILE=1`.
#[must_use]
pub fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var("CELESTIA_PHASE_PROFILE").is_ok_and(|v| v == "1");
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        s => s == 2,
    }
}

/// Turns profiling on or off (tests and benchmarks).
pub fn set_enabled(on: bool) {
    STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
}

/// Counts a committed IAVL read.
pub(crate) fn count_read() {
    READS.fetch_add(1, Ordering::Relaxed);
}

fn counters() -> [u64; 6] {
    let e = multistore::exp_counters();
    [READS.load(Ordering::Relaxed), multistore::node_lookups(), e[0], e[1], e[2], e[3]]
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
    /// Node-cache lookups.
    pub lookups: u64,
    /// EXPERIMENT: disk reads by load, by child ref, cold and evicted misses.
    pub exp: [u64; 4],
}

/// A running phase; recorded when dropped.
#[derive(Debug)]
pub struct Span {
    name: &'static str,
    start: Instant,
    at: [u64; 6],
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
        t.lookups += now[1] - self.at[1];
        for i in 0..4 {
            t.exp[i] += now[i + 2] - self.at[i + 2];
        }
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
