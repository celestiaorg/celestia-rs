//! Cost of creating a store iterator and reading its first entry, as IBC
//! `UpdateClient` does (prune-oldest: first entry of a client's consensus
//! states; previous / next consensus state: first entry in each
//! direction), against the committed entries in the range (`M`) and the
//! block's pending writes outside it (`N`). Neither should change the cost.
//!
//! ```sh
//! cargo run --release -p sdk --example range_bench
//! ```
#![allow(clippy::print_stdout)]

use std::time::Instant;

use sdk::store::{Kind, MultiStore, StoreError};

const ROUNDS: u32 = 2_000;

fn key(prefix: &[u8], i: u64) -> Vec<u8> {
    let mut k = prefix.to_vec();
    k.extend_from_slice(&i.to_be_bytes());
    k
}

fn store(m: u64, n: u64) -> Result<MultiStore, StoreError> {
    let mut ms = MultiStore::new();
    ms.mount("ibc", Kind::Iavl)?;
    for i in 0..m {
        ms.set("ibc", key(b"c/", i), vec![1; 64])?;
    }
    ms.flush()?;
    ms.commit()?;
    // Pending writes of the block (outside the range) and of the tx.
    for i in 0..n {
        ms.set("ibc", key(b"x/", i), vec![2; 64])?;
    }
    ms.branch();
    for i in 0..n / 10 {
        ms.set("ibc", key(b"y/", i), vec![3; 64])?;
    }
    Ok(ms)
}

fn first(ms: &MultiStore, rev: bool) -> Result<f64, StoreError> {
    let (lo, hi) = (b"c/".to_vec(), b"c0".to_vec());
    let t = Instant::now();
    for _ in 0..ROUNDS {
        let mut it = ms.lazy_range("ibc", Some(&lo), Some(&hi), rev)?;
        std::hint::black_box(it.next().transpose()?);
    }
    Ok(t.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS))
}

/// The eager `range` (what iterators did before they read lazily).
fn eager(ms: &MultiStore) -> Result<f64, StoreError> {
    let t = Instant::now();
    let rounds = ROUNDS / 100;
    for _ in 0..rounds {
        std::hint::black_box(ms.range("ibc", Some(b"c/"), Some(b"c0"), false)?.first());
    }
    Ok(t.elapsed().as_secs_f64() * 1e6 / f64::from(rounds))
}

fn main() -> Result<(), StoreError> {
    println!(
        "{:>8} {:>8} {:>12} {:>12} {:>12}",
        "M", "N", "forward_us", "reverse_us", "eager_us"
    );
    for m in [100, 10_000, 200_000] {
        for n in [0, 10_000, 100_000] {
            let ms = store(m, n)?;
            println!(
                "{m:>8} {n:>8} {:>12.2} {:>12.2} {:>12.0}",
                first(&ms, false)?,
                first(&ms, true)?,
                eager(&ms)?
            );
        }
    }
    Ok(())
}
