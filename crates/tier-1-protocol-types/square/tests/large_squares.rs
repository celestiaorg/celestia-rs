//! Data roots of large squares against Go celestia-app v10 (`pkg/da`:
//! `ExtendShares` + `NewDataAvailabilityHeader`, rsmt2d `LeoRSCodec` on
//! klauspost/reedsolomon v1.14.2).
//!
//! Squares above 128x128 extend rows of more than 256 shards, which
//! klauspost encodes with leopard over GF(2^16). The Rust codec used to
//! support GF(2^8) only, so 256x256 squares (v10 `gov_max_square_size`)
//! could not be built or verified: a mixed Go / Rust devnet halted on the
//! first near-max PFB round (soak, 2026-10-14).
//!
//! Vectors: a `w`x`w` square whose share `i` is namespace `0x00..01`
//! (version 0) followed by `sha256("{w}-{i}")[j % 32] ^ j` for bytes
//! `j = 29..512`, generated with
//!
//! ```go
//! eds, _ := da.ExtendShares(shares)
//! dah, _ := da.NewDataAvailabilityHeader(eds)
//! fmt.Println(w, hex.EncodeToString(dah.Hash()), hex.EncodeToString(dah.RowRoots[w]))
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use shares::{SHARE_SIZE, Share};

fn square(w: usize) -> Vec<Share> {
    (0..w * w)
        .map(|i| {
            let mut s = [0u8; SHARE_SIZE];
            s[28] = 1;
            let seed = hash::sha256(format!("{w}-{i}").as_bytes());
            for (j, b) in s.iter_mut().enumerate().skip(29) {
                #[allow(clippy::cast_possible_truncation)]
                let x = j as u8;
                *b = seed[j % 32] ^ x;
            }
            Share::from_bytes(s).unwrap()
        })
        .collect()
}

fn check(w: usize, want_root: &str, want_first_parity_row_root: &str) {
    let eds = square::extend_shares(&square(w)).unwrap();
    assert_eq!(eds.width(), 2 * w);
    let dah = square::new_data_availability_header(&eds).unwrap();
    assert_eq!(hex::encode(dah.hash()), want_root, "{w}x{w} data root");
    assert_eq!(
        hex::encode(dah.row_roots[w]),
        want_first_parity_row_root,
        "{w}x{w} first parity row root"
    );
}

#[test]
fn data_root_64() {
    check(
        64,
        "bde18ccd94a8441bac417f6452e73419190b6985c10133e72fc85c9eb61247cf",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff5cea10a29733f3bf5d2c4ee4a3653e4771b3bc421f65c89eda8cf5704a3815cf",
    );
}

#[test]
fn data_root_128() {
    check(
        128,
        "6be0e7e5a4652c769ab7b877a0808e35bee7a1bf378eeac86aef631b03f35867",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe3f7df6cd070ae9c0cb384b5e56acb300c1e65caa80a06d87ff68f695dd8790f",
    );
}

/// GF(2^16) leopard rows (512 shards).
#[test]
fn data_root_256() {
    check(
        256,
        "4901f2617fac71cf98067fb1d03ecc060768f3043a5cec2ae434d90a1394ea53",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffcff38f7408911ee85b693af59a14a516c455602b9e802db452af34a7bb132d82",
    );
}
