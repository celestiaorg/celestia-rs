//! Every kernel against the scalar one and against leopard-codec 0.2.0 (the
//! scalar implementation the square used before, itself checked against
//! klauspost/reedsolomon).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use leopard::{Kernel, ORDER};
use proptest::prelude::*;

/// xorshift bytes, so cases are reproducible from their seed.
fn random_shards(count: usize, size: usize, mut seed: u64) -> Vec<Vec<u8>> {
    seed |= 1;
    (0..count)
        .map(|_| {
            (0..size)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed.to_le_bytes()[3]
                })
                .collect()
        })
        .collect()
}

/// leopard-codec's encoder supports these counts.
fn supported(data: usize, parity: usize) -> bool {
    let m = parity.next_power_of_two();
    let last = data % m;
    m >= data || last == 0 || (data / m + 1) * m < 255
}

fn case() -> impl Strategy<Value = (usize, usize, usize, u64)> {
    (1usize..ORDER - 1)
        .prop_flat_map(|data| (Just(data), 1..=(ORDER - data).min(data)))
        .prop_filter("leopard-codec overflow", |(d, p)| supported(*d, *p))
        .prop_flat_map(|(d, p)| (Just(d), Just(p), 1usize..24, any::<u64>()))
        .prop_map(|(d, p, s, seed)| (d, p, s * 64, seed))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn encode_matches_leopard_codec_for_every_kernel((data, parity, size, seed) in case()) {
        let input = random_shards(data + parity, size, seed);
        let mut expected = input.clone();
        leopard_codec::encode(&mut expected, data).unwrap();
        for kernel in Kernel::supported() {
            let mut shards = input.clone();
            leopard::encode_with(kernel, &mut shards, data).unwrap();
            prop_assert!(shards == expected, "{kernel:?} differs: {data}+{parity} x {size}");
        }
    }

    #[test]
    fn reconstruct_matches_leopard_codec_for_every_kernel(
        (data, parity, size, seed) in case(),
        missing_seed in any::<u64>(),
    ) {
        let mut full = random_shards(data + parity, size, seed);
        leopard_codec::encode(&mut full, data).unwrap();
        // Drop up to `parity` shards.
        let total = data + parity;
        let mut damaged = full.clone();
        let mut s = missing_seed | 1;
        let drop = 1 + usize::try_from(s % parity as u64).unwrap();
        for _ in 0..drop {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            damaged[usize::try_from(s % total as u64).unwrap()].clear();
        }
        let mut expected = damaged.clone();
        leopard_codec::reconstruct(&mut expected, data).unwrap();
        prop_assert!(expected == full);
        for kernel in Kernel::supported() {
            let mut shards = damaged.clone();
            leopard::reconstruct_with(kernel, &mut shards, data).unwrap();
            prop_assert!(shards == full, "{kernel:?} reconstruct differs");
        }
    }
}

/// The square's shapes: k data + k parity shards of 512 bytes.
#[test]
fn square_rows_match_leopard_codec() {
    for k in [1usize, 2, 4, 8, 16, 32, 64, 128] {
        for seed in 1..4u64 {
            let mut expected = random_shards(2 * k, 512, seed * 7919 + k as u64);
            let input = expected.clone();
            leopard_codec::encode(&mut expected, k).unwrap();
            for kernel in Kernel::supported() {
                let mut shards = input.clone();
                leopard::encode_with(kernel, &mut shards, k).unwrap();
                assert!(shards == expected, "{kernel:?} k={k}");
            }
        }
    }
}

#[test]
fn errors_match_leopard_codec() {
    let mut too_many = random_shards(ORDER + 1, 64, 1);
    assert!(leopard::encode(&mut too_many, ORDER / 2).is_err());
    let mut unequal = random_shards(4, 64, 2);
    unequal[3].push(0);
    assert_eq!(
        leopard::encode(&mut unequal, 2),
        Err(leopard::Error::UnequalShardsLengths)
    );
    let mut odd = random_shards(4, 65, 3);
    assert_eq!(
        leopard::encode(&mut odd, 2),
        Err(leopard::Error::InvalidShardSize(65))
    );
    let mut few = random_shards(4, 64, 4);
    few[0].clear();
    few[1].clear();
    few[2].clear();
    assert_eq!(
        leopard::reconstruct(&mut few, 2),
        Err(leopard::Error::TooFewShards(1, 2))
    );
}

#[test]
fn a_simd_kernel_is_detected_where_available() {
    let detected = Kernel::detect();
    #[cfg(target_arch = "aarch64")]
    assert_eq!(detected, Kernel::Neon);
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2")
        && std::env::var("CELESTIA_RS_LEOPARD_KERNEL").is_err()
    {
        assert_eq!(detected, Kernel::Avx2);
    }
    assert!(detected.is_supported());
}
