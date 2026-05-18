//! Erasure-coded square helpers.

use shares::{NAMESPACE_SIZE, PARITY_SHARES_NAMESPACE, SHARE_SIZE, Share};

use crate::commitment::namespaced_merkle_root;
use crate::dah::DAH_ROOT_SIZE;
use crate::inclusion::{ceil_sqrt, is_power_of_two};
use crate::square::Square;
use crate::{Error, Result};

/// Erasure codec used by extended data square construction.
pub trait ErasureCodec {
    /// Encode data shards and return parity shards.
    fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>>;

    /// Reconstruct missing data or parity shards in place and return all shards.
    fn decode(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>>;

    /// Return the maximum original data-square share count supported by this codec.
    fn max_chunks(&self) -> usize;

    /// Return the stable codec name.
    fn name(&self) -> &'static str;

    /// Validate a shard byte length.
    fn validate_chunk_size(&self, chunk_size: usize) -> Result<()>;
}

/// Leopard Reed-Solomon codec compatible with `rsmt2d` for 8-bit Leopard widths.
#[derive(Debug, Default, Clone)]
pub struct LeoRsCodec;

impl LeoRsCodec {
    /// Construct a Leopard Reed-Solomon codec.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ErasureCodec for LeoRsCodec {
    fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
        validate_codec_data(self.name(), data)?;
        self.validate_chunk_size(data[0].len())?;
        let data_shards = data.len();
        let mut shards = Vec::with_capacity(data_shards * 2);
        shards.extend(data.iter().cloned());
        shards.extend((0..data_shards).map(|_| vec![0; data[0].len()]));
        leopard_codec::encode(&mut shards, data_shards).map_err(|error| Error::ErasureCodec {
            codec: self.name(),
            message: error.to_string(),
        })?;
        Ok(shards[data_shards..].to_vec())
    }

    fn decode(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>> {
        if data.is_empty() || data.len() % 2 != 0 {
            return Err(Error::InvalidErasureShardCount { actual: data.len() });
        }
        let data_shards = data.len() / 2;
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
        let max_original_width = leopard_codec::ORDER / 2;
        max_original_width * max_original_width
    }

    fn name(&self) -> &'static str {
        "Leopard"
    }

    fn validate_chunk_size(&self, chunk_size: usize) -> Result<()> {
        if chunk_size == 0 || chunk_size % 64 != 0 {
            return Err(Error::InvalidErasureChunkSize { actual: chunk_size });
        }
        Ok(())
    }
}

/// Row-major extended data square.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedDataSquare {
    original_width: usize,
    width: usize,
    shares: Vec<Vec<u8>>,
    codec_name: &'static str,
}

impl ExtendedDataSquare {
    /// Construct an empty extended data square with zero-filled cells.
    pub fn new(codec: &dyn ErasureCodec, width: usize, share_size: usize) -> Result<Self> {
        validate_extended_width(width)?;
        codec.validate_chunk_size(share_size)?;
        let total = width.checked_mul(width).ok_or(Error::ArithmeticOverflow)?;
        Ok(Self {
            original_width: width / 2,
            width,
            shares: vec![vec![0; share_size]; total],
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
        codec.validate_chunk_size(shares[0].len())?;
        Ok(Self {
            original_width: width / 2,
            width,
            shares,
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

    /// Return one row by index.
    pub fn row(&self, row_index: usize) -> Result<Vec<Vec<u8>>> {
        self.check_axis_index("row", row_index)?;
        let start = row_index
            .checked_mul(self.width)
            .ok_or(Error::ArithmeticOverflow)?;
        let end = start
            .checked_add(self.width)
            .ok_or(Error::ArithmeticOverflow)?;
        Ok(self.shares[start..end].to_vec())
    }

    /// Return one column by index.
    pub fn column(&self, column_index: usize) -> Result<Vec<Vec<u8>>> {
        self.check_axis_index("column", column_index)?;
        (0..self.width)
            .map(|row| {
                let index = row
                    .checked_mul(self.width)
                    .and_then(|start| start.checked_add(column_index))
                    .ok_or(Error::ArithmeticOverflow)?;
                Ok(self.shares[index].clone())
            })
            .collect()
    }

    /// Return row-major EDS bytes.
    #[must_use]
    pub fn flattened(&self) -> Vec<Vec<u8>> {
        self.shares.clone()
    }

    /// Return row-major original data-square bytes from Q0.
    #[must_use]
    pub fn flattened_ods(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(self.original_width * self.original_width);
        for row in 0..self.original_width {
            let start = row * self.width;
            out.extend_from_slice(&self.shares[start..start + self.original_width]);
        }
        out
    }

    /// Compute all row roots.
    pub fn row_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        (0..self.width)
            .map(|row| self.axis_root(row, Axis::Row))
            .collect()
    }

    /// Compute all column roots.
    pub fn column_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        (0..self.width)
            .map(|column| self.axis_root(column, Axis::Column))
            .collect()
    }

    /// Compute row roots followed by column roots.
    pub fn roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
        let mut roots = self.row_roots()?;
        roots.extend(self.column_roots()?);
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

    fn axis_root(&self, axis_index: usize, axis: Axis) -> Result<[u8; DAH_ROOT_SIZE]> {
        let cells = match axis {
            Axis::Row => self.row(axis_index)?,
            Axis::Column => self.column(axis_index)?,
        };
        let mut leaves = Vec::with_capacity(cells.len());
        for (share_index, cell) in cells.iter().enumerate() {
            let mut leaf = Vec::with_capacity(NAMESPACE_SIZE + cell.len());
            if axis_index < self.original_width && share_index < self.original_width {
                leaf.extend_from_slice(&cell[..NAMESPACE_SIZE]);
            } else {
                leaf.extend_from_slice(PARITY_SHARES_NAMESPACE.as_bytes());
            }
            leaf.extend_from_slice(cell);
            leaves.push(leaf);
        }
        namespaced_merkle_root(&leaves)
    }
}

/// Compute an extended data square from original square shares.
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

    let square = Square::new(shares.to_vec())?;
    codec.validate_chunk_size(SHARE_SIZE)?;
    let original_width = square.size();
    let width = original_width
        .checked_mul(2)
        .ok_or(Error::ArithmeticOverflow)?;
    let total = width.checked_mul(width).ok_or(Error::ArithmeticOverflow)?;
    let mut eds_shares = vec![vec![0; SHARE_SIZE]; total];

    for row in 0..original_width {
        for column in 0..original_width {
            let ods_index = row
                .checked_mul(original_width)
                .and_then(|start| start.checked_add(column))
                .ok_or(Error::ArithmeticOverflow)?;
            let eds_index = row
                .checked_mul(width)
                .and_then(|start| start.checked_add(column))
                .ok_or(Error::ArithmeticOverflow)?;
            eds_shares[eds_index] = square.as_shares()[ods_index].as_bytes().to_vec();
        }
    }

    for row in 0..original_width {
        let row_data = row_prefix(&eds_shares, width, row, original_width)?;
        let parity = codec.encode(&row_data)?;
        set_row_suffix(&mut eds_shares, width, row, original_width, &parity)?;

        let column_data = column_prefix(&eds_shares, width, row, original_width)?;
        let parity = codec.encode(&column_data)?;
        set_column_suffix(&mut eds_shares, width, row, original_width, &parity)?;
    }

    for row in original_width..width {
        let row_data = row_prefix(&eds_shares, width, row, original_width)?;
        let parity = codec.encode(&row_data)?;
        set_row_suffix(&mut eds_shares, width, row, original_width, &parity)?;
    }

    Ok(ExtendedDataSquare {
        original_width,
        width,
        shares: eds_shares,
        codec_name: codec.name(),
    })
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

fn validate_extended_width(width: usize) -> Result<()> {
    if width == 0 || width % 2 != 0 || !is_power_of_two(width / 2) {
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

fn validate_codec_data(codec: &'static str, data: &[Vec<u8>]) -> Result<()> {
    validate_shard_count(data.len())?;
    let chunk_size = data[0].len();
    if chunk_size == 0 {
        return Err(Error::InvalidErasureChunkSize { actual: 0 });
    }
    for shard in data {
        if shard.len() != chunk_size {
            return Err(Error::ErasureCodec {
                codec,
                message: "shards of different lengths found".to_owned(),
            });
        }
    }
    Ok(())
}

fn row_prefix(
    shares: &[Vec<u8>],
    width: usize,
    row: usize,
    original_width: usize,
) -> Result<Vec<Vec<u8>>> {
    let start = row.checked_mul(width).ok_or(Error::ArithmeticOverflow)?;
    Ok(shares[start..start + original_width].to_vec())
}

fn column_prefix(
    shares: &[Vec<u8>],
    width: usize,
    column: usize,
    original_width: usize,
) -> Result<Vec<Vec<u8>>> {
    (0..original_width)
        .map(|row| {
            let index = row
                .checked_mul(width)
                .and_then(|start| start.checked_add(column))
                .ok_or(Error::ArithmeticOverflow)?;
            Ok(shares[index].clone())
        })
        .collect()
}

fn set_row_suffix(
    shares: &mut [Vec<u8>],
    width: usize,
    row: usize,
    original_width: usize,
    parity: &[Vec<u8>],
) -> Result<()> {
    if parity.len() != original_width {
        return Err(Error::InvalidErasureShardCount {
            actual: parity.len(),
        });
    }
    let start = row
        .checked_mul(width)
        .and_then(|row_start| row_start.checked_add(original_width))
        .ok_or(Error::ArithmeticOverflow)?;
    shares[start..start + original_width].clone_from_slice(parity);
    Ok(())
}

fn set_column_suffix(
    shares: &mut [Vec<u8>],
    width: usize,
    column: usize,
    original_width: usize,
    parity: &[Vec<u8>],
) -> Result<()> {
    if parity.len() != original_width {
        return Err(Error::InvalidErasureShardCount {
            actual: parity.len(),
        });
    }
    for (offset, share) in parity.iter().enumerate() {
        let row = original_width
            .checked_add(offset)
            .ok_or(Error::ArithmeticOverflow)?;
        let index = row
            .checked_mul(width)
            .and_then(|start| start.checked_add(column))
            .ok_or(Error::ArithmeticOverflow)?;
        shares[index].clone_from(share);
    }
    Ok(())
}
