//! Original data square layout helpers.

use shares::{
    CompactShareSplitter, DEFAULT_SHARE_VERSION, PAY_FOR_BLOB_NAMESPACE, SHARE_SIZE, Share,
    SparseShareSplitter, TX_NAMESPACE, parse_txs, reserved_padding_shares, tail_padding_shares,
};

use crate::inclusion::{ceil_sqrt, is_power_of_two, round_up_power_of_two};
use crate::{Error, Result};

/// Current mainnet maximum original data square width.
pub const DEFAULT_MAX_SQUARE_SIZE: usize = 128;

/// Current mainnet subtree root threshold used by blob commitment rules.
pub const DEFAULT_SUBTREE_ROOT_THRESHOLD: usize = 64;

/// A power-of-two original data square of shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Square {
    shares: Vec<Share>,
}

impl Square {
    /// Construct a square from validated shares.
    pub fn new(shares: Vec<Share>) -> Result<Self> {
        validate_square_shares(&shares)?;
        Ok(Self { shares })
    }

    /// Construct a square without validating the provided shares.
    #[must_use]
    pub fn from_shares_unchecked(shares: Vec<Share>) -> Self {
        Self { shares }
    }

    /// Borrow the square shares.
    #[must_use]
    pub fn as_shares(&self) -> &[Share] {
        &self.shares
    }

    /// Return the square shares.
    #[must_use]
    pub fn into_shares(self) -> Vec<Share> {
        self.shares
    }

    /// Encode the square as fixed-size share byte arrays.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<[u8; SHARE_SIZE]> {
        self.shares.iter().cloned().map(Share::into_bytes).collect()
    }

    /// Return the square side length.
    #[must_use]
    pub fn size(&self) -> usize {
        size(self.shares.len())
    }

    /// Return the number of shares.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shares.len()
    }

    /// Return true when there are no shares.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shares.is_empty()
    }

    /// Return true when this is the canonical one-share empty square.
    #[must_use]
    pub fn is_empty_square(&self) -> bool {
        self.shares == empty_square().shares
    }

    /// Return wrapped PFB transaction bytes in this square.
    pub fn wrapped_pfbs(&self) -> Result<Vec<Vec<u8>>> {
        let range = shares::get_share_range_for_namespace(&self.shares, PAY_FOR_BLOB_NAMESPACE)?;
        Ok(parse_txs(&self.shares[range.start..range.end])?)
    }
}

impl AsRef<[Share]> for Square {
    fn as_ref(&self) -> &[Share] {
        self.as_shares()
    }
}

impl TryFrom<Vec<Share>> for Square {
    type Error = Error;

    fn try_from(value: Vec<Share>) -> Result<Self> {
        Self::new(value)
    }
}

/// Return the square side length for a share count.
#[must_use]
pub fn size(share_count: usize) -> usize {
    round_up_power_of_two(ceil_sqrt(share_count))
}

/// The checks of [`Square::new`] without taking ownership of the shares.
pub(crate) fn validate_square_shares(shares: &[Share]) -> Result<()> {
    if shares.is_empty() {
        return Err(Error::InvalidSquareSize { actual: 0 });
    }
    let side = size(shares.len());
    if !is_power_of_two(side) || side.checked_mul(side) != Some(shares.len()) {
        return Err(Error::InvalidSquareSize {
            actual: shares.len(),
        });
    }
    for share in shares {
        share.validate()?;
    }
    Ok(())
}

/// Return the canonical empty original data square.
#[must_use]
pub fn empty_square() -> Square {
    Square::from_shares_unchecked(tail_padding_shares(shares::MIN_SHARE_COUNT))
}

/// Lay out compact tx shares, compact PFB shares, sparse blob shares, and padding.
#[allow(clippy::needless_pass_by_value)]
pub fn write_square(
    mut tx_writer: CompactShareSplitter,
    mut pfb_writer: CompactShareSplitter,
    blob_writer: SparseShareSplitter,
    non_reserved_start: usize,
    square_size: usize,
) -> Result<Square> {
    if !is_power_of_two(square_size) {
        return Err(Error::NotPowerOfTwo {
            actual: square_size,
        });
    }

    let total_shares = square_size
        .checked_mul(square_size)
        .ok_or(Error::ArithmeticOverflow)?;
    let pfb_start_index = tx_writer.count();
    let padding_start_index = pfb_start_index
        .checked_add(pfb_writer.count())
        .ok_or(Error::ArithmeticOverflow)?;
    if non_reserved_start < padding_start_index {
        return Err(Error::InvalidSquareSize {
            actual: non_reserved_start,
        });
    }

    let blob_count = blob_writer.count();
    let end_of_last_blob = non_reserved_start
        .checked_add(blob_count)
        .ok_or(Error::ArithmeticOverflow)?;
    if total_shares < end_of_last_blob {
        return Err(Error::InvalidSquareSize {
            actual: square_size,
        });
    }

    let tx_shares = tx_writer.export()?;
    let pfb_shares = pfb_writer.export()?;
    let reserved_padding = reserved_padding_shares(non_reserved_start - padding_start_index);
    let blob_shares = blob_writer.export();
    let tail_padding = tail_padding_shares(total_shares - end_of_last_blob);

    let mut shares = Vec::with_capacity(total_shares);
    shares.extend(tx_shares);
    shares.extend(pfb_shares);
    shares.extend(reserved_padding);
    shares.extend(blob_shares);
    shares.extend(tail_padding);
    if shares.len() != total_shares {
        return Err(Error::InvalidSquareSize {
            actual: shares.len(),
        });
    }
    Ok(Square::from_shares_unchecked(shares))
}

#[allow(dead_code)]
const _: (u8, shares::Namespace, shares::Namespace) =
    (DEFAULT_SHARE_VERSION, TX_NAMESPACE, PAY_FOR_BLOB_NAMESPACE);
