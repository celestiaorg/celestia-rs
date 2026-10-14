//! Regression (soak, 2026-10-14): splitting a blob into shares copied the
//! remaining tail for every share, quadratic in the blob size. A 7.3 MB
//! blob (max tx size is 8 MiB) took ~10 s in release builds, so blocks with
//! a few large PFBs (256x256 squares) made Rust proposers miss their round
//! and Rust validators prevote late: the mixed devnet stalled for minutes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use shares::{Blob, Namespace, parse_blobs, split_blobs};

#[test]
fn eight_mib_blob_splits_in_linear_time_and_round_trips() {
    let data: Vec<u8> = (0..8 * 1024 * 1024u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let blob = Blob::new_v0(Namespace::new_v0(&[7; 10]).unwrap(), data.clone()).unwrap();
    let t = Instant::now();
    let shares = split_blobs(std::slice::from_ref(&blob)).unwrap();
    let took = t.elapsed();
    // 1 + ceil((8 MiB - 478) / 482) shares.
    assert_eq!(shares.len(), 1 + (data.len() - 478).div_ceil(482));
    // Linear: well under a second even unoptimized (the quadratic version
    // copied ~70 GB here).
    assert!(took < Duration::from_secs(10), "split took {took:?}");
    let parsed = parse_blobs(&shares).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].data(), data.as_slice());
}
