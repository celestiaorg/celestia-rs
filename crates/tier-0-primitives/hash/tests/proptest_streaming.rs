//! Property tests for the hash crate — REQ-9.
//!
//! For any `Vec<u8>` up to 64 KB, feeding it to `Sha256Hasher` in arbitrary
//! chunking patterns must produce the same digest as `sha256(whole_input)`.

// Tests use proptest macros and intentionally unwrap/expect.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use hash::{sha256, Sha256Hasher};
use proptest::prelude::*;

/// Strategy: any byte-vector up to 64 KiB.
fn arb_bytes_64k() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..=65_536)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// REQ-9: streaming Sha256Hasher is chunk-invariant.
    #[test]
    fn sha256_hasher_chunk_invariant(
        input in arb_bytes_64k(),
        split_pts in prop::collection::vec(0usize..=65_536usize, 0..=16),
    ) {
        let expected = sha256(&input);

        // Build chunks using the split points.
        let mut sorted_pts: Vec<usize> = split_pts
            .into_iter()
            .map(|p| p.min(input.len()))  // clamp to valid range
            .collect();
        sorted_pts.sort_unstable();
        sorted_pts.dedup();

        let mut hasher = Sha256Hasher::new();
        let mut prev = 0;
        for &pt in &sorted_pts {
            if pt > prev && pt <= input.len() {
                hasher.update(&input[prev..pt]);
                prev = pt;
            }
        }
        // Feed the remainder.
        if prev < input.len() {
            hasher.update(&input[prev..]);
        }

        prop_assert_eq!(hasher.finalize(), expected);
    }
}
