//! Data-parallel helpers on scoped std threads.
//!
//! The extended data square and its NMT roots are independent per row /
//! column, so they are spread over a few threads. The target machine has
//! 10 cores (`docs/hardware.md`), so at most [`MAX_THREADS`] are used.
//! Results never depend on the thread count: every item is computed by the
//! same function from the same inputs.

use std::num::NonZeroUsize;
use std::sync::OnceLock;

use crate::Result;

/// Upper bound on worker threads (the reference machine's core count).
pub const MAX_THREADS: usize = 10;

/// Worker threads used for one parallel pass: the available parallelism,
/// capped at [`MAX_THREADS`]. `CELESTIA_RS_SQUARE_THREADS` overrides it
/// (1 = serial), for benchmarks.
#[must_use]
pub fn threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    *THREADS.get_or_init(|| {
        std::env::var("CELESTIA_RS_SQUARE_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map_or(1, NonZeroUsize::get)
                    .min(MAX_THREADS)
            })
    })
}

/// Calls `f(index, item)` for every item, splitting `items` into contiguous
/// runs over up to [`threads`] scoped threads (the calling thread takes the
/// first run). Runs serially when there are fewer than `2 * min_per_thread`
/// items. Each run stops at its first error and the error of the
/// lowest-indexed failing item is returned, as a serial loop would.
pub(crate) fn for_each_mut<T, F>(items: &mut [T], min_per_thread: usize, f: F) -> Result<()>
where
    T: Send,
    F: Fn(usize, &mut T) -> Result<()> + Sync,
{
    let workers = threads().min(items.len() / min_per_thread.max(1)).max(1);
    let run = |offset: usize, chunk: &mut [T]| -> Result<()> {
        chunk
            .iter_mut()
            .enumerate()
            .try_for_each(|(i, item)| f(offset + i, item))
    };
    if workers == 1 {
        return run(0, items);
    }
    let per = items.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let mut chunks = items.chunks_mut(per).enumerate();
        let first = chunks.next();
        let handles: Vec<_> = chunks
            .map(|(n, chunk)| {
                let run = &run;
                scope.spawn(move || run(n * per, chunk))
            })
            .collect();
        let mut result = first.map_or(Ok(()), |(_, chunk)| run(0, chunk));
        for handle in handles {
            let outcome = handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
            if result.is_ok() {
                result = outcome;
            }
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn every_item_is_visited_once_with_its_index() {
        for len in [0, 1, 7, 64, 1000] {
            let mut items = vec![usize::MAX; len];
            for_each_mut(&mut items, 1, |i, slot| {
                *slot = i;
                Ok(())
            })
            .expect("no error");
            assert!(items.iter().enumerate().all(|(i, &v)| i == v));
        }
    }

    #[test]
    fn the_lowest_failing_item_wins() {
        let mut items = vec![0u8; 256];
        let err = for_each_mut(&mut items, 1, |i, _| {
            if i % 50 == 49 {
                Err(Error::InvalidErasureShardCount { actual: i })
            } else {
                Ok(())
            }
        })
        .expect_err("fails");
        assert!(matches!(
            err,
            Error::InvalidErasureShardCount { actual: 49 }
        ));
    }
}
