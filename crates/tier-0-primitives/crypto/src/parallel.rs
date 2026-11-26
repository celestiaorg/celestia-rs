//! Order-preserving parallel checks for signature batches.

/// Below this many items, checking them on one thread is cheaper than
/// starting threads.
pub const PARALLEL_MIN: usize = 16;

/// The index of the first item of `items` (in order) for which `ok` is
/// false. With [`PARALLEL_MIN`] or more items and more than one CPU, the
/// items are split into contiguous chunks checked on scoped threads, and
/// the first failure of the lowest failing chunk is returned: the same
/// answer as a sequential scan, for a pure `ok`. A panic in a check is
/// re-raised, never taken for success.
pub fn first_failure<T: Sync>(items: &[T], ok: impl Fn(&T) -> bool + Sync) -> Option<usize> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if items.len() < PARALLEL_MIN || threads < 2 {
        return items.iter().position(|item| !ok(item));
    }
    let chunks = threads.min(items.len() / (PARALLEL_MIN / 2)).max(1);
    let size = items.len().div_ceil(chunks);
    let ok = &ok;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(size)
            .enumerate()
            .map(|(c, part)| {
                scope.spawn(move || part.iter().position(|item| !ok(item)).map(|i| c * size + i))
            })
            .collect();
        let found: Vec<Option<usize>> = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect();
        found.into_iter().flatten().next()
    })
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
