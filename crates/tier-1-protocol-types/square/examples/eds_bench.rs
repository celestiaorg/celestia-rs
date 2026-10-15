//! Times erasure extension and DAH computation of synthetic squares.
//!
//! `cargo run --release -p square --example eds_bench -- [iterations] [widths...]`
//! prints, per original width, the median time to extend the square, to
//! compute the DAH from the EDS, and both together, plus the DAH hash (so
//! two builds can be checked to agree).
#![allow(clippy::print_stdout, clippy::expect_used, missing_docs)]

use std::time::{Duration, Instant};

use shares::{SHARE_SIZE, Share};

/// A deterministic square whose namespaces increase row-major (valid NMT
/// leaves along every row and column) with pseudo-random share bodies.
fn synthetic_square(width: usize) -> Vec<Share> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64 ^ width as u64;
    (0..width * width)
        .map(|i| {
            let mut bytes = [0u8; SHARE_SIZE];
            // Version-0 namespace: 18 zero bytes then a 10-byte ID.
            bytes[19..27].copy_from_slice(&((i as u64) + 1).to_be_bytes());
            bytes[29] = 1; // info byte: share version 0, sequence start
            for chunk in bytes[30..].chunks_mut(8) {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let n = chunk.len();
                chunk.copy_from_slice(&state.to_le_bytes()[..n]);
            }
            Share::from_bytes(bytes).expect("share")
        })
        .collect()
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: usize = args.next().map_or(5, |s| s.parse().expect("iterations"));
    let widths: Vec<usize> = {
        let w: Vec<usize> = args.map(|s| s.parse().expect("width")).collect();
        if w.is_empty() { vec![64, 128, 256] } else { w }
    };
    for width in widths {
        let shares = synthetic_square(width);
        let (mut ext, mut dah_t, mut total) = (Vec::new(), Vec::new(), Vec::new());
        let mut hash = [0u8; 32];
        for _ in 0..iterations {
            let t0 = Instant::now();
            let eds = square::extend_shares(&shares).expect("extend");
            let t1 = Instant::now();
            let dah = square::new_data_availability_header(&eds).expect("dah");
            // sha256 over every row root then every column root: the same
            // digest the Go counterpart (rsmt2d + nmt) prints.
            let mut roots = Vec::new();
            for root in dah.row_roots.iter().chain(&dah.column_roots) {
                roots.extend_from_slice(root);
            }
            hash = hash::sha256(&roots);
            let t2 = Instant::now();
            ext.push(t1 - t0);
            dah_t.push(t2 - t1);
            total.push(t2 - t0);
        }
        println!(
            "width {width:>3}: extend {:>8.1} ms  dah {:>8.1} ms  total {:>8.1} ms  roots-digest {}",
            median(ext).as_secs_f64() * 1e3,
            median(dah_t).as_secs_f64() * 1e3,
            median(total).as_secs_f64() * 1e3,
            hex::encode(hash)
        );
    }
}
