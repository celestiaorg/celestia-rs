//! Erasure-coded square helpers.

use shares::{NAMESPACE_SIZE, PARITY_SHARES_NAMESPACE, SHARE_SIZE, Share};

use std::cell::RefCell;

use crate::commitment::namespaced_merkle_root_of;
use crate::dah::DAH_ROOT_SIZE;
use crate::inclusion::{ceil_sqrt, is_power_of_two};
use crate::square::validate_square_shares;
use crate::{Error, Result, par};

/// Rows / columns per worker thread below which a pass stays serial.
const MIN_AXES_PER_THREAD: usize = 8;

/// Erasure codec used by extended data square construction.
pub trait ErasureCodec: Send + Sync {
    /// Encode data shards and return parity shards.
    fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>>;

    /// Encode the data shards `data` and write the parity shards into
    /// `parity` (one per data shard, of the shard length): [`Self::encode`]
    /// without owning the shards.
    fn encode_into(&self, data: &[&[u8]], parity: &mut [&mut [u8]]) -> Result<()> {
        let owned: Vec<Vec<u8>> = data.iter().map(|shard| shard.to_vec()).collect();
        copy_parity(&self.encode(&owned)?, parity)
    }

    /// Reconstruct missing data or parity shards in place and return all shards.
    fn decode(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>>;

    /// Return the maximum original data-square share count supported by this codec.
    fn max_chunks(&self) -> usize;

    /// Return the stable codec name.
    fn name(&self) -> &'static str;

    /// Validate a shard byte length.
    fn validate_chunk_size(&self, chunk_size: usize) -> Result<()>;
}

/// Shard count (original + parity) up to which klauspost/reedsolomon
/// (`reedsolomon.New(k, k, WithLeopardGF(true))`, rsmt2d's `LeoRSCodec`)
/// uses leopard over GF(2^8); above it, leopard over GF(2^16).
const MAX_GF8_SHARDS: usize = 256;

/// Leopard Reed-Solomon codec compatible with `rsmt2d`'s `LeoRSCodec`
/// (klauspost/reedsolomon v1.14.2): GF(2^8) leopard (`leopard-codec`) for
/// rows of up to 256 shards (squares up to 128x128) and GF(2^16) leopard
/// (`reed-solomon-simd`, byte-identical to klauspost's `leopardFF16`, as
/// the propagation vectors check) for larger squares (v10 allows up to
/// 512x512). Squares above 128x128 used to fail to extend (soak,
/// 2026-10-14).
#[derive(Debug, Default, Clone)]
pub struct LeoRsCodec;

impl LeoRsCodec {
    /// Construct a Leopard Reed-Solomon codec.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// GF(2^16) reconstruction: restores the missing originals, then
    /// recomputes the parity (encoding is deterministic).
    fn decode_gf16(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>> {
        let k = data.len() / 2;
        let err = |error: String| Error::ErasureCodec {
            codec: self.name(),
            message: error,
        };
        if data[..k].iter().any(Option::is_none) {
            let restored = reed_solomon_simd::decode(
                k,
                k,
                data[..k]
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_deref().map(|s| (i, s))),
                data[k..]
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_deref().map(|s| (i, s))),
            )
            .map_err(|e| err(e.to_string()))?;
            for (i, s) in restored {
                if let Some(slot) = data.get_mut(i) {
                    *slot = Some(s);
                }
            }
        }
        let originals: Vec<Vec<u8>> = data[..k]
            .iter()
            .map(|s| s.clone().ok_or_else(|| err("unrecoverable shard".into())))
            .collect::<Result<_>>()?;
        let parity = self.encode(&originals)?;
        for (slot, shard) in data[k..].iter_mut().zip(parity) {
            *slot = Some(shard);
        }
        Ok(data.iter().map(|s| s.clone().unwrap_or_default()).collect())
    }
}

impl ErasureCodec for LeoRsCodec {
    fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
        validate_codec_data(self.name(), data)?;
        self.validate_chunk_size(data[0].len())?;
        let data_shards = data.len();
        if 2 * data_shards > MAX_GF8_SHARDS {
            return reed_solomon_simd::encode(data_shards, data_shards, data).map_err(|error| {
                Error::ErasureCodec {
                    codec: self.name(),
                    message: error.to_string(),
                }
            });
        }
        let mut shards = Vec::with_capacity(data_shards * 2);
        shards.extend(data.iter().cloned());
        shards.extend((0..data_shards).map(|_| vec![0; data[0].len()]));
        leopard_codec::encode(&mut shards, data_shards).map_err(|error| Error::ErasureCodec {
            codec: self.name(),
            message: error.to_string(),
        })?;
        Ok(shards[data_shards..].to_vec())
    }

    fn encode_into(&self, data: &[&[u8]], parity: &mut [&mut [u8]]) -> Result<()> {
        validate_codec_data(self.name(), data)?;
        self.validate_chunk_size(data[0].len())?;
        let err = |error: String| Error::ErasureCodec {
            codec: self.name(),
            message: error,
        };
        let (data_shards, shard_size) = (data.len(), data[0].len());
        if parity.len() != data_shards {
            return Err(Error::InvalidErasureShardCount {
                actual: parity.len(),
            });
        }
        if 2 * data_shards > MAX_GF8_SHARDS {
            // One encoder per thread, reused across rows and columns.
            thread_local! {
                static GF16: RefCell<Option<reed_solomon_simd::ReedSolomonEncoder>> =
                    const { RefCell::new(None) };
            }
            return GF16.with_borrow_mut(|cached| {
                let encoder = match cached {
                    Some(encoder) => {
                        encoder
                            .reset(data_shards, data_shards, shard_size)
                            .map_err(|e| err(e.to_string()))?;
                        encoder
                    }
                    None => cached.insert(
                        reed_solomon_simd::ReedSolomonEncoder::new(
                            data_shards,
                            data_shards,
                            shard_size,
                        )
                        .map_err(|e| err(e.to_string()))?,
                    ),
                };
                for shard in data {
                    encoder
                        .add_original_shard(shard)
                        .map_err(|e| err(e.to_string()))?;
                }
                let result = encoder.encode().map_err(|e| err(e.to_string()))?;
                for (dst, src) in parity.iter_mut().zip(result.recovery_iter()) {
                    copy_shard(src, dst)?;
                }
                Ok(())
            });
        }
        let mut scratch = vec![0u8; 2 * data_shards * shard_size];
        for (dst, src) in scratch.chunks_exact_mut(shard_size).zip(data) {
            dst.copy_from_slice(src);
        }
        let mut shards: Vec<&mut [u8]> = scratch.chunks_exact_mut(shard_size).collect();
        leopard_codec::encode(&mut shards, data_shards).map_err(|e| err(e.to_string()))?;
        for (dst, src) in parity.iter_mut().zip(&shards[data_shards..]) {
            copy_shard(src, dst)?;
        }
        Ok(())
    }

    fn decode(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>> {
        if data.is_empty() || !data.len().is_multiple_of(2) {
            return Err(Error::InvalidErasureShardCount { actual: data.len() });
        }
        let data_shards = data.len() / 2;
        if data.len() > MAX_GF8_SHARDS {
            return self.decode_gf16(data);
        }
        let mut shards: Vec<Vec<u8>> = data
            .iter()
            .map(|shard| shard.clone().unwrap_or_default())
            .collect();
        leopard_codec::reconstruct(&mut shards, data_shards).map_err(|error| {
            Error::ErasureCodec {
                codec: self.name(),
                message: error.to_string(),
            }
        })?;
        for (slot, shard) in data.iter_mut().zip(shards.iter().cloned()) {
            *slot = Some(shard);
        }
        Ok(shards)
    }

    fn max_chunks(&self) -> usize {
        crate::v4::SQUARE_SIZE_UPPER_BOUND * crate::v4::SQUARE_SIZE_UPPER_BOUND
    }

    fn name(&self) -> &'static str {
        "Leopard"
    }

    fn validate_chunk_size(&self, chunk_size: usize) -> Result<()> {
        if chunk_size == 0 || !chunk_size.is_multiple_of(64) {
            return Err(Error::InvalidErasureChunkSize { actual: chunk_size });
        }
        Ok(())
    }
}

/// Row-major extended data square, stored as one contiguous buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedDataSquare {
    original_width: usize,
    width: usize,
    share_size: usize,
    /// `width * width` shares of `share_size` bytes, row-major.
    data: Vec<u8>,
    codec_name: &'static str,
}

impl ExtendedDataSquare {
    /// Construct an empty extended data square with zero-filled cells.
    pub fn new(codec: &dyn ErasureCodec, width: usize, share_size: usize) -> Result<Self> {
        validate_extended_width(width)?;
        codec.validate_chunk_size(share_size)?;
        let total = width.checked_mul(width).ok_or(Error::ArithmeticOverflow)?;
        let bytes = total
            .checked_mul(share_size)
            .ok_or(Error::ArithmeticOverflow)?;
        Ok(Self {
            original_width: width / 2,
            width,
            share_size,
            data: vec![0; bytes],
            codec_name: codec.name(),
        })
    }

    /// Import an already extended data square from row-major bytes.
    pub fn import(shares: Vec<Vec<u8>>, codec: &dyn ErasureCodec) -> Result<Self> {
        validate_shard_count(shares.len())?;
        validate_codec_data(codec.name(), &shares)?;
        let width = ceil_sqrt(shares.len());
        if width.checked_mul(width) != Some(shares.len()) {
            return Err(Error::InvalidErasureShardCount {
                actual: shares.len(),
            });
        }
        validate_extended_width(width)?;
        let share_size = shares[0].len();
        codec.validate_chunk_size(share_size)?;
        Ok(Self {
            original_width: width / 2,
            width,
            share_size,
            data: shares.concat(),
            codec_name: codec.name(),
        })
    }

    /// Return the original data-square width.
    #[must_use]
    pub const fn original_width(&self) -> usize {
        self.original_width
    }

    /// Return the extended data-square width.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Borrow the share at (`row`, `column`).
    pub fn share(&self, row: usize, column: usize) -> Result<&[u8]> {
        self.check_axis_index("row", row)?;
        self.check_axis_index("column", column)?;
        Ok(self.cell(row, column))
    }

    /// Return one row by index.
    pub fn row(&self, row_index: usize) -> Result<Vec<Vec<u8>>> {
        self.check_axis_index("row", row_index)?;
        Ok((0..self.width)
            .map(|column| self.cell(row_index, column).to_vec())
            .collect())
    }

    /// Return one column by index.
    pub fn column(&self, column_index: usize) -> Result<Vec<Vec<u8>>> {
        self.check_axis_index("column", column_index)?;
        Ok((0..self.width)
            .map(|row| self.cell(row, column_index).to_vec())
            .collect())
    }

    /// Return row-major EDS bytes.
    #[must_use]
    pub fn flattened(&self) -> Vec<Vec<u8>> {
        self.data
            .chunks_exact(self.share_size)
            .map(<[u8]>::to_vec)
            .collect()
    }

    /// Return row-major original data-square bytes from Q0.
    #[must_use]
    pub fn flattened_ods(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(self.original_width * self.original_width);
        for row in 0..self.original_width {
            for column in 0..self.original_width {
                out.push(self.cell(row, column).to_vec());
            }
        }
        out
    }

    /// Compute all row roots.
    pub fn row_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        self.axis_roots(Some(Axis::Row))
    }

    /// Compute all column roots.
    pub fn column_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        self.axis_roots(Some(Axis::Column))
    }

    /// Compute row roots followed by column roots.
    pub fn roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        self.axis_roots(None)
    }

    /// The roots of every row (`Some(Row)`), every column (`Some(Column)`)
    /// or both, rows first (`None`), computed in parallel.
    fn axis_roots(&self, axis: Option<Axis>) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        let count = if axis.is_some() {
            self.width
        } else {
            2 * self.width
        };
        let mut roots = vec![[0u8; DAH_ROOT_SIZE]; count];
        par::for_each_mut(&mut roots, MIN_AXES_PER_THREAD, |i, root| {
            *root = match axis {
                Some(axis) => self.axis_root(i, axis)?,
                None if i < self.width => self.axis_root(i, Axis::Row)?,
                None => self.axis_root(i - self.width, Axis::Column)?,
            };
            Ok(())
        })?;
        Ok(roots)
    }

    fn check_axis_index(&self, axis: &'static str, index: usize) -> Result<()> {
        if index >= self.width {
            return Err(Error::ExtendedDataSquareIndexOutOfRange {
                axis,
                index,
                width: self.width,
            });
        }
        Ok(())
    }

    /// The share at (`row`, `column`); both must be below `width`.
    fn cell(&self, row: usize, column: usize) -> &[u8] {
        let start = (row * self.width + column) * self.share_size;
        &self.data[start..start + self.share_size]
    }

    /// The NMT root of one axis (celestia-app's `ErasuredNamespacedMerkleTree`:
    /// Q0 leaves keep their namespace, every other leaf gets the parity
    /// namespace).
    fn axis_root(&self, axis_index: usize, axis: Axis) -> Result<[u8; DAH_ROOT_SIZE]> {
        self.check_axis_index(axis.name(), axis_index)?;
        let parity: &[u8] = PARITY_SHARES_NAMESPACE.as_bytes();
        namespaced_merkle_root_of(self.width, |share_index| {
            let cell = match axis {
                Axis::Row => self.cell(axis_index, share_index),
                Axis::Column => self.cell(share_index, axis_index),
            };
            if axis_index < self.original_width && share_index < self.original_width {
                (&cell[..NAMESPACE_SIZE], cell)
            } else {
                (parity, cell)
            }
        })
    }
}

/// Compute an extended data square from original square shares.
///
/// Like rsmt2d: Q1 extends the rows of Q0, Q2 the columns of Q0 and Q3 the
/// rows of Q2. Rows and columns are encoded in parallel.
pub fn compute_extended_data_square(
    shares: &[Share],
    codec: &dyn ErasureCodec,
) -> Result<ExtendedDataSquare> {
    if shares.len() > codec.max_chunks() {
        return Err(Error::InvalidErasureShardCount {
            actual: shares.len(),
        });
    }
    if !is_power_of_two(shares.len()) {
        return Err(Error::InvalidErasureShardCount {
            actual: shares.len(),
        });
    }

    validate_square_shares(shares)?;
    codec.validate_chunk_size(SHARE_SIZE)?;
    let original_width = crate::square::size(shares.len());
    let width = original_width
        .checked_mul(2)
        .ok_or(Error::ArithmeticOverflow)?;
    let total = width.checked_mul(width).ok_or(Error::ArithmeticOverflow)?;
    let row_bytes = width * SHARE_SIZE;
    let half_row = original_width * SHARE_SIZE;
    let mut data = vec![0u8; total * SHARE_SIZE];
    let (top, bottom) = data.split_at_mut(original_width * row_bytes);

    // Q0 and Q1: copy each original row and extend it.
    let mut rows: Vec<&mut [u8]> = top.chunks_exact_mut(row_bytes).collect();
    par::for_each_mut(&mut rows, MIN_AXES_PER_THREAD, |row, cells| {
        let (q0, q1) = cells.split_at_mut(half_row);
        let originals = &shares[row * original_width..(row + 1) * original_width];
        for (dst, share) in q0.as_chunks_mut::<SHARE_SIZE>().0.iter_mut().zip(originals) {
            *dst = *share.as_bytes();
        }
        encode_axis(codec, q0, q1)
    })?;
    drop(rows);
    let top: &[u8] = top;

    // Q2: extend each column of Q0 (column-major scratch).
    let mut q2 = vec![0u8; original_width * half_row];
    let mut columns: Vec<&mut [u8]> = q2.chunks_exact_mut(half_row).collect();
    par::for_each_mut(&mut columns, MIN_AXES_PER_THREAD, |column, parity| {
        let originals: Vec<&[u8]> = (0..original_width)
            .map(|row| {
                let start = row * row_bytes + column * SHARE_SIZE;
                &top[start..start + SHARE_SIZE]
            })
            .collect();
        let mut parity = shard_slices_mut(parity);
        codec.encode_into(&originals, &mut parity)
    })?;
    drop(columns);

    // Q2 into place, and Q3: extend each row of Q2.
    let mut rows: Vec<&mut [u8]> = bottom.chunks_exact_mut(row_bytes).collect();
    par::for_each_mut(&mut rows, MIN_AXES_PER_THREAD, |row, cells| {
        let (q2_row, q3) = cells.split_at_mut(half_row);
        for (column, dst) in q2_row
            .as_chunks_mut::<SHARE_SIZE>()
            .0
            .iter_mut()
            .enumerate()
        {
            let start = column * half_row + row * SHARE_SIZE;
            dst.copy_from_slice(&q2[start..start + SHARE_SIZE]);
        }
        encode_axis(codec, q2_row, q3)
    })?;
    drop(rows);

    Ok(ExtendedDataSquare {
        original_width,
        width,
        share_size: SHARE_SIZE,
        data,
        codec_name: codec.name(),
    })
}

/// Encode the contiguous shares `originals` into the contiguous `parity`.
fn encode_axis(codec: &dyn ErasureCodec, originals: &[u8], parity: &mut [u8]) -> Result<()> {
    let originals: Vec<&[u8]> = originals
        .as_chunks::<SHARE_SIZE>()
        .0
        .iter()
        .map(<[u8; SHARE_SIZE]>::as_slice)
        .collect();
    let mut parity = shard_slices_mut(parity);
    codec.encode_into(&originals, &mut parity)
}

/// `bytes` as consecutive mutable shares.
fn shard_slices_mut(bytes: &mut [u8]) -> Vec<&mut [u8]> {
    bytes
        .as_chunks_mut::<SHARE_SIZE>()
        .0
        .iter_mut()
        .map(<[u8; SHARE_SIZE]>::as_mut_slice)
        .collect()
}

fn copy_shard(src: &[u8], dst: &mut [u8]) -> Result<()> {
    if src.len() != dst.len() {
        return Err(Error::InvalidErasureChunkSize { actual: src.len() });
    }
    dst.copy_from_slice(src);
    Ok(())
}

fn copy_parity(parity: &[Vec<u8>], out: &mut [&mut [u8]]) -> Result<()> {
    if parity.len() != out.len() {
        return Err(Error::InvalidErasureShardCount {
            actual: parity.len(),
        });
    }
    for (dst, src) in out.iter_mut().zip(parity) {
        copy_shard(src, dst)?;
    }
    Ok(())
}

/// Compute an extended data square using the default Leopard codec.
pub fn extend_shares(shares: &[Share]) -> Result<ExtendedDataSquare> {
    compute_extended_data_square(shares, &LeoRsCodec::new())
}

/// Import an extended data square from row-major bytes.
pub fn import_extended_data_square(
    shares: Vec<Vec<u8>>,
    codec: &dyn ErasureCodec,
) -> Result<ExtendedDataSquare> {
    ExtendedDataSquare::import(shares, codec)
}

#[derive(Debug, Clone, Copy)]
enum Axis {
    Row,
    Column,
}

impl Axis {
    const fn name(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Column => "column",
        }
    }
}

fn validate_extended_width(width: usize) -> Result<()> {
    if width == 0 || !width.is_multiple_of(2) || !is_power_of_two(width / 2) {
        return Err(Error::InvalidErasureShardCount { actual: width });
    }
    Ok(())
}

fn validate_shard_count(count: usize) -> Result<()> {
    if count == 0 {
        return Err(Error::InvalidErasureShardCount { actual: 0 });
    }
    Ok(())
}

fn validate_codec_data<T: AsRef<[u8]>>(codec: &'static str, data: &[T]) -> Result<()> {
    validate_shard_count(data.len())?;
    let chunk_size = data[0].as_ref().len();
    if chunk_size == 0 {
        return Err(Error::InvalidErasureChunkSize { actual: 0 });
    }
    for shard in data {
        if shard.as_ref().len() != chunk_size {
            return Err(Error::ErasureCodec {
                codec,
                message: "shards of different lengths found".to_owned(),
            });
        }
    }
    Ok(())
}
