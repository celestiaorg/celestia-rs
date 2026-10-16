//! Leopard Reed-Solomon erasure coding over GF(2^8), byte-identical to
//! klauspost/reedsolomon's `leopard8` (rsmt2d's `LeoRSCodec` for squares
//! up to 128x128), with SIMD kernels.
//!
//! The algorithm is leopard-codec 0.2.0 (Eiger, Apache-2.0; a port of
//! klauspost/reedsolomon, itself a port of catid/leopard), restructured
//! only where the bytes are touched: the 2- and 4-way FFT butterflies run
//! through [`Kernel`], which multiplies with split-nibble tables and
//! `pshufb` (SSSE3 / AVX2) or `tbl` (NEON), selected at runtime, with the
//! original table lookups as the portable fallback. Every kernel is
//! element-wise, so the output does not depend on the kernel; the tests
//! check every kernel against the scalar one and against leopard-codec.
//!
//! The GF(2^16) codec used for larger squares is `reed-solomon-simd`,
//! which already has AVX2 / SSSE3 / NEON kernels.

mod kernel;
mod tables;

pub use kernel::Kernel;
pub use tables::{MODULUS, ORDER};

use tables::{FFT_SKEW, LOG_WALSH, fwht};

/// Errors of [`encode`] and [`reconstruct`] (leopard-codec's `LeopardError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// More than [`ORDER`] shards.
    MaxShardNumberExceeded(usize),
    /// More parity shards (first) than data shards (second).
    MaxParityShardNumberExceeded(usize, usize),
    /// A (data, parity) shard count leopard cannot encode (it would
    /// overflow the FFT skew table).
    UnsupportedShardsAmounts(usize, usize),
    /// Every shard is empty.
    EmptyShards,
    /// Shards of different lengths.
    UnequalShardsLengths,
    /// The shard size is not a multiple of 64.
    InvalidShardSize(usize),
    /// Fewer shards present (first) than data shards (second).
    TooFewShards(usize, usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxShardNumberExceeded(n) => {
                write!(f, "Maximum shard number ({ORDER}) exceeded: {n}")
            }
            Self::MaxParityShardNumberExceeded(p, d) => {
                write!(f, "Maximum parity shard number ({p}) exceeded: {d}")
            }
            Self::UnsupportedShardsAmounts(d, p) => {
                write!(
                    f,
                    "Unsupported number of data ({d}) and parity ({p}) shards"
                )
            }
            Self::EmptyShards => f.write_str("Shards contain no data"),
            Self::UnequalShardsLengths => f.write_str("Shards of different lengths found"),
            Self::InvalidShardSize(s) => write!(f, "Shard size ({s}) should be a multiple of 64"),
            Self::TooFewShards(have, need) => write!(
                f,
                "Too few shards ({have}) to reconstruct data, at least {need} needed"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A result with [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Encode the parity shards: the first `data_shards` shards are data, the
/// rest are overwritten with parity. Uses [`Kernel::detect`].
pub fn encode(shards: &mut [impl AsMut<[u8]>], data_shards: usize) -> Result<()> {
    encode_with(Kernel::detect(), shards, data_shards)
}

/// [`encode`] with an explicit kernel (an unsupported one falls back to
/// [`Kernel::Scalar`]).
pub fn encode_with(
    kernel: Kernel,
    shards: &mut [impl AsMut<[u8]>],
    data_shards: usize,
) -> Result<()> {
    if shards.len() > ORDER {
        return Err(Error::MaxShardNumberExceeded(shards.len()));
    }
    let parity_shards = shards.len().saturating_sub(data_shards);
    if parity_shards > data_shards {
        return Err(Error::MaxParityShardNumberExceeded(
            parity_shards,
            data_shards,
        ));
    }
    if parity_shards == 0 || is_encode_buf_overflow(data_shards, parity_shards) {
        return Err(Error::UnsupportedShardsAmounts(data_shards, parity_shards));
    }
    let mut shards: Vec<&mut [u8]> = shards.iter_mut().map(AsMut::as_mut).collect();
    let shard_size = check_shards(shards.iter().map(|s| s.len()), false)?;
    if !shard_size.is_multiple_of(64) {
        return Err(Error::InvalidShardSize(shard_size));
    }
    let kernel = effective(kernel);
    encode_inner(kernel, &mut shards, data_shards, shard_size);
    Ok(())
}

/// The kernel to run: `kernel` if the CPU supports it, else scalar.
fn effective(kernel: Kernel) -> Kernel {
    if kernel.is_supported() {
        kernel
    } else {
        Kernel::Scalar
    }
}

fn encode_inner(k: Kernel, shards: &mut [&mut [u8]], data_shards: usize, shard_size: usize) {
    let parity_shards = shards.len() - data_shards;
    let m = ceil_pow2(parity_shards);
    let mtrunc = m.min(data_shards);

    // The first half of `work` receives the parity shards.
    let mut work_mem = vec![0; 2 * m * shard_size];
    let mut work: Vec<&mut [u8]> = work_mem.chunks_exact_mut(shard_size).collect();
    let skew_lut = &FFT_SKEW[m - 1..];

    for (shard, work) in shards[data_shards..].iter().zip(work.iter_mut()) {
        work.copy_from_slice(shard);
    }

    ifft_dit_encoder(
        k,
        &shards[..data_shards],
        mtrunc,
        &mut work,
        None,
        m,
        skew_lut,
    );

    let last_count = data_shards % m;
    if m < data_shards {
        let (xor_out, work) = work.split_at_mut(m);
        let mut n = m;
        while n <= data_shards - m {
            ifft_dit_encoder(k, &shards[n..], m, work, Some(xor_out), m, &skew_lut[n..]);
            n += m;
        }
        if last_count != 0 {
            ifft_dit_encoder(
                k,
                &shards[n..],
                last_count,
                work,
                Some(xor_out),
                m,
                &skew_lut[n..],
            );
        }
    }

    fft_dit(k, &mut work, parity_shards, m, &FFT_SKEW);

    for (shard, work) in shards[data_shards..].iter_mut().zip(work.iter()) {
        shard.copy_from_slice(work);
    }
}

/// Whether leopard's encoder would overflow the FFT skew table for these
/// counts (the last pass is partial and too far out).
fn is_encode_buf_overflow(data_shards: usize, parity_shards: usize) -> bool {
    let m = ceil_pow2(parity_shards);
    let last_count = data_shards % m;
    if m >= data_shards || last_count == 0 {
        return false;
    }
    let full_passes = data_shards / m;
    (full_passes + 1) * m + 1 > MODULUS as usize
}

/// Reconstruct the missing (empty) shards in place. At least
/// `data_shards` shards must be present. Uses [`Kernel::detect`].
pub fn reconstruct(shards: &mut [impl AsMut<Vec<u8>>], data_shards: usize) -> Result<()> {
    reconstruct_with(Kernel::detect(), shards, data_shards)
}

/// [`reconstruct`] with an explicit kernel.
pub fn reconstruct_with(
    kernel: Kernel,
    shards: &mut [impl AsMut<Vec<u8>>],
    data_shards: usize,
) -> Result<()> {
    if shards.len() > ORDER {
        return Err(Error::MaxShardNumberExceeded(shards.len()));
    }
    let parity_shards = shards.len().saturating_sub(data_shards);
    if parity_shards > data_shards {
        return Err(Error::MaxParityShardNumberExceeded(
            parity_shards,
            data_shards,
        ));
    }
    let mut shards: Vec<&mut Vec<u8>> = shards.iter_mut().map(AsMut::as_mut).collect();
    let shard_size = check_shards(shards.iter().map(|s| s.len()), true)?;
    let present = shards.iter().filter(|s| !s.is_empty()).count();
    if present == shards.len() {
        return Ok(());
    }
    if present < data_shards {
        return Err(Error::TooFewShards(present, data_shards));
    }
    if !shard_size.is_multiple_of(64) {
        return Err(Error::InvalidShardSize(shard_size));
    }
    reconstruct_inner(effective(kernel), &mut shards, data_shards, shard_size);
    Ok(())
}

fn reconstruct_inner(
    k: Kernel,
    shards: &mut [&mut Vec<u8>],
    data_shards: usize,
    shard_size: usize,
) {
    let parity_shards = shards.len() - data_shards;
    let m = ceil_pow2(parity_shards);
    let n = ceil_pow2(m + data_shards);

    let empty: Vec<bool> = shards.iter().map(|s| s.is_empty()).collect();
    for shard in shards.iter_mut().filter(|s| s.is_empty()) {
        shard.resize(shard_size, 0);
    }

    // Error locator: the missing parity shards, the padding up to m, the
    // missing data shards.
    let mut err_locs = [0u8; ORDER];
    for (&e, loc) in empty.iter().skip(data_shards).zip(err_locs.iter_mut()) {
        if e {
            *loc = 1;
        }
    }
    for loc in &mut err_locs[parity_shards..m] {
        *loc = 1;
    }
    for (&e, loc) in empty.iter().take(data_shards).zip(err_locs[m..].iter_mut()) {
        if e {
            *loc = 1;
        }
    }
    fwht(&mut err_locs, ORDER, m + data_shards);
    for (err, &log_walsh) in err_locs.iter_mut().zip(LOG_WALSH.iter()) {
        let product = usize::from(*err) * usize::from(log_walsh);
        #[allow(clippy::cast_possible_truncation)]
        let reduced = (product % MODULUS as usize) as u8;
        *err = reduced;
    }
    fwht(&mut err_locs, ORDER, ORDER);

    let mut work_mem = vec![0u8; shard_size * n];
    let mut work: Vec<&mut [u8]> = work_mem.chunks_exact_mut(shard_size).collect();
    for i in 0..parity_shards {
        if !empty[i + data_shards] {
            k.mul(work[i], shards[i + data_shards], err_locs[i]);
        }
    }
    for i in 0..data_shards {
        if !empty[i] {
            k.mul(work[m + i], shards[i], err_locs[m + i]);
        }
    }

    ifft_dit_decoder(k, m + data_shards, &mut work, n, &FFT_SKEW);

    // Formal derivative.
    for i in 1..n {
        let width = ((i ^ (i - 1)) + 1) >> 1;
        let (output, input) = work.split_at_mut(i);
        for (out, inp) in output[i - width..].iter_mut().zip(input.iter()) {
            k.xor(out, inp);
        }
    }

    fft_dit(k, &mut work, m + data_shards, n, &FFT_SKEW);

    // Original = -ErrLocator * FFT(Derivative(IFFT(ErrLocator * Received))).
    for (i, shard) in shards.iter_mut().enumerate() {
        if !empty[i] {
            continue;
        }
        if i >= data_shards {
            let j = i - data_shards;
            k.mul(shard, work[j], MODULUS - err_locs[j]);
        } else {
            k.mul(shard, work[i + m], MODULUS - err_locs[i + m]);
        }
    }
}

/// The common length of the shards (empty ones allowed if `allow_empty`).
fn check_shards(lens: impl Iterator<Item = usize> + Clone, allow_empty: bool) -> Result<usize> {
    let size = lens.clone().find(|&l| l != 0).unwrap_or(0);
    if size == 0 {
        return if allow_empty {
            Ok(0)
        } else {
            Err(Error::EmptyShards)
        };
    }
    if lens
        .into_iter()
        .all(|l| l == size || (allow_empty && l == 0))
    {
        Ok(size)
    } else {
        Err(Error::UnequalShardsLengths)
    }
}

const fn ceil_pow2(x: usize) -> usize {
    1 << (usize::BITS - (x - 1).leading_zeros())
}

/// `work[i]`, `work[i + dist]`, `work[i + 2 dist]`, `work[i + 3 dist]`.
fn four<'a>(work: &'a mut [&mut [u8]], i: usize, dist: usize) -> [&'a mut [u8]; 4] {
    let (w0, rest) = work[i..].split_at_mut(dist);
    let (w1, rest) = rest.split_at_mut(dist);
    let (w2, w3) = rest.split_at_mut(dist);
    [&mut *w0[0], &mut *w1[0], &mut *w2[0], &mut *w3[0]]
}

/// `x`, `y` = `work[i]`, `work[j]` with `i < j`.
fn two<'a>(work: &'a mut [&mut [u8]], i: usize, j: usize) -> (&'a mut [u8], &'a mut [u8]) {
    let (lo, hi) = work.split_at_mut(j);
    (&mut *lo[i], &mut *hi[0])
}

/// The encoder's IFFT of `data` into `work` (and XORed into `xor_output`).
fn ifft_dit_encoder(
    k: Kernel,
    data: &[&mut [u8]],
    mtrunc: usize,
    work: &mut [&mut [u8]],
    xor_output: Option<&mut [&mut [u8]]>,
    m: usize,
    skew_lut: &[u8],
) {
    for i in 0..mtrunc {
        work[i].copy_from_slice(data[i]);
    }
    for row in &mut work[mtrunc..m] {
        row.fill(0);
    }

    let mut dist = 1;
    let mut dist4 = 4;
    while dist4 <= m {
        for r in (0..mtrunc).step_by(dist4) {
            let iend = r + dist;
            let log_m01 = skew_lut[iend];
            let log_m02 = skew_lut[iend + dist];
            let log_m23 = skew_lut[iend + dist * 2];
            // klauspost/reedsolomon's order (differs from catid/leopard).
            for i in r..iend {
                k.ifft_dit4(four(work, i, dist), log_m01, log_m23, log_m02);
            }
        }
        dist = dist4;
        dist4 <<= 2;
    }

    if dist < m {
        let log_m = skew_lut[dist];
        for i in 0..dist {
            let (x, y) = two(work, i, i + dist);
            k.ifft_dit2(x, y, log_m);
        }
    }

    if let Some(xor_output) = xor_output {
        for (out, w) in xor_output[..m].iter_mut().zip(work[..m].iter()) {
            k.xor(out, w);
        }
    }
}

/// The decoder's IFFT, in place.
fn ifft_dit_decoder(k: Kernel, mtrunc: usize, work: &mut [&mut [u8]], m: usize, skew_lut: &[u8]) {
    let mut dist = 1;
    let mut dist4 = 4;
    while dist4 <= m {
        for r in (0..mtrunc).step_by(dist4) {
            let iend = r + dist;
            let log_m01 = skew_lut[iend - 1];
            let log_m02 = skew_lut[iend + dist - 1];
            let log_m23 = skew_lut[iend + 2 * dist - 1];
            for i in r..iend {
                k.ifft_dit4(four(work, i, dist), log_m01, log_m23, log_m02);
            }
        }
        dist = dist4;
        dist4 <<= 2;
    }

    if dist < m {
        let log_m = skew_lut[dist - 1];
        for i in 0..dist {
            let (x, y) = two(work, i, i + dist);
            k.ifft_dit2(x, y, log_m);
        }
    }
}

/// In-place FFT (encoder and decoder), truncated to `mtrunc` outputs.
fn fft_dit(k: Kernel, work: &mut [&mut [u8]], mtrunc: usize, m: usize, skew_lut: &[u8]) {
    let mut dist4 = m;
    let mut dist = m >> 2;
    while dist != 0 {
        for r in (0..mtrunc).step_by(dist4) {
            let iend = r + dist;
            let log_m01 = skew_lut[iend - 1];
            let log_m02 = skew_lut[iend + dist - 1];
            let log_m23 = skew_lut[iend + 2 * dist - 1];
            for i in r..iend {
                k.fft_dit4(four(work, i, dist), log_m01, log_m23, log_m02);
            }
        }
        dist4 = dist;
        dist >>= 2;
    }

    if dist4 == 2 {
        for r in (0..mtrunc).step_by(2) {
            let log_m = skew_lut[r];
            let (x, y) = two(work, r, r + 1);
            k.fft_dit2(x, y, log_m);
        }
    }
}
