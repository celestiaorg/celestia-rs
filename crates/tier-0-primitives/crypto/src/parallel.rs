//! Order-preserving parallel checks for signature batches.

/// Below this many items, checking them on one thread is cheaper than
/// starting threads.
pub const PARALLEL_MIN: usize = 16;

/// The index of the first item of `items` (in order) for which `ok` is
/// false. With [`PARALLEL_MIN`] or more items and more than one CPU, the
/// items are split into contiguous chunks checked on the shared worker
/// pool (rayon's global pool, one thread per CPU, started once), and
/// the lowest failing index is returned: the same answer as a sequential
/// scan, for a pure `ok`. Workers share the lowest failure found so far
/// and stop before an item past it, so an early failure still ends the
/// work promptly (at most one more check per thread), as a sequential scan
/// does. A panic in a check is re-raised, never taken for success.
pub fn first_failure<T: Sync>(items: &[T], ok: impl Fn(&T) -> bool + Sync) -> Option<usize> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if items.len() < PARALLEL_MIN || threads < 2 {
        return items.iter().position(|item| !ok(item));
    }
    let chunks = threads.min(items.len() / (PARALLEL_MIN / 2)).max(1);
    let size = items.len().div_ceil(chunks);
    let first = AtomicUsize::new(usize::MAX);
    let (ok, first) = (&ok, &first);
    // `rayon::scope_fifo` returns when every chunk is done and re-raises a
    // chunk's panic. FIFO order starts the lowest chunk first (rayon's
    // default runs the last one spawned first), so an early failure is
    // found as soon as with one thread per chunk.
    rayon::scope_fifo(|scope| {
        for (c, part) in items.chunks(size).enumerate() {
            scope.spawn_fifo(move |_| {
                for (i, item) in part.iter().enumerate() {
                    let index = c * size + i;
                    // A lower item already failed: nothing here can win.
                    if index > first.load(Ordering::Relaxed) {
                        return;
                    }
                    if !ok(item) {
                        first.fetch_min(index, Ordering::Relaxed);
                        return;
                    }
                }
            });
        }
    });
    let found = first.load(Ordering::Relaxed);
    (found != usize::MAX).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_sequential_scan() {
        for len in [0usize, 1, 15, 16, 17, 40, 100, 1000] {
            let items: Vec<usize> = (0..len).collect();
            for bad in [
                vec![],
                vec![0],
                vec![len.saturating_sub(1)],
                vec![7, 3],
                vec![len / 2, len / 3 + 1],
            ] {
                let ok = |i: &usize| !bad.contains(i);
                assert_eq!(
                    first_failure(&items, ok),
                    items.iter().position(|i| !ok(i)),
                    "len {len} bad {bad:?}"
                );
            }
        }
    }

    /// An early failure stops the other workers: far fewer checks than
    /// items, as with a sequential scan that stops at the failure.
    #[test]
    fn an_early_failure_stops_the_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let items: Vec<usize> = (0..10_000).collect();
        let checks = AtomicUsize::new(0);
        let found = first_failure(&items, |i| {
            checks.fetch_add(1, Ordering::Relaxed);
            // Slow enough that the other workers see the failure.
            std::thread::sleep(std::time::Duration::from_micros(200));
            *i != 0
        });
        assert_eq!(found, Some(0));
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        assert!(
            checks.load(Ordering::Relaxed) <= 2 * threads + 2,
            "{checks:?}"
        );
    }

    /// Checks run on a fixed set of worker threads: calls do not start
    /// threads of their own (verifying every block's commit did, at a
    /// cost of one thread start per CPU per commit).
    #[test]
    fn calls_reuse_the_same_worker_threads() {
        use std::collections::HashSet;
        use std::sync::Mutex;
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        if threads < 2 {
            return;
        }
        let items: Vec<usize> = (0..256).collect();
        let seen = Mutex::new(HashSet::new());
        for _ in 0..50 {
            assert_eq!(
                first_failure(&items, |_| {
                    seen.lock().unwrap().insert(std::thread::current().id());
                    true
                }),
                None
            );
        }
        let seen = seen.into_inner().unwrap().len();
        assert!(seen <= threads + 1, "{seen} threads for {threads} CPUs");
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn a_panicking_check_is_re_raised() {
        let items: Vec<usize> = (0..64).collect();
        first_failure(&items, |i| {
            assert!(*i != 50, "boom");
            true
        });
    }
}
