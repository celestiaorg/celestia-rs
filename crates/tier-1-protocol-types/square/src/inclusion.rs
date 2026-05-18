//! Blob inclusion and square-sizing helpers.

use crate::{Error, Result};

/// Return the number of shares used and aligned start indexes for blobs.
pub fn blob_shares_used_non_interactive_defaults(
    cursor: usize,
    subtree_root_threshold: usize,
    blob_share_lengths: &[usize],
) -> Result<(usize, Vec<u32>)> {
    let start = cursor;
    let mut cursor = cursor;
    let mut indexes = Vec::with_capacity(blob_share_lengths.len());

    for &blob_len in blob_share_lengths {
        cursor = next_share_index(cursor, blob_len, subtree_root_threshold)?;
        let share_index =
            u32::try_from(cursor).map_err(|_| Error::ShareIndexOverflow { actual: cursor })?;
        indexes.push(share_index);
        cursor = cursor
            .checked_add(blob_len)
            .ok_or(Error::ArithmeticOverflow)?;
    }

    let shares_used = cursor.checked_sub(start).ok_or(Error::ArithmeticOverflow)?;
    Ok((shares_used, indexes))
}

/// Return the next blob start index allowed by ADR-013 share commitment rules.
pub fn next_share_index(
    cursor: usize,
    blob_share_len: usize,
    subtree_root_threshold: usize,
) -> Result<usize> {
    let tree_width = sub_tree_width(blob_share_len, subtree_root_threshold)?;
    round_up_by_multiple_of(cursor, tree_width)
}

/// Round `cursor` up to the next multiple of `value`.
pub fn round_up_by_multiple_of(cursor: usize, value: usize) -> Result<usize> {
    if value == 0 {
        return Err(Error::ZeroDivisor);
    }
    if cursor % value == 0 {
        return Ok(cursor);
    }
    let quotient = cursor / value;
    quotient
        .checked_add(1)
        .and_then(|next| next.checked_mul(value))
        .ok_or(Error::ArithmeticOverflow)
}

/// Return the minimum power-of-two square side that can hold `share_count`.
pub fn blob_min_square_size(share_count: usize) -> Result<usize> {
    if share_count == 0 {
        return Err(Error::InvalidSquareSize { actual: 0 });
    }
    let root = ceil_sqrt(share_count);
    Ok(round_up_power_of_two(root))
}

/// Return the max leaves per subtree used for blob share commitments.
pub fn sub_tree_width(share_count: usize, subtree_root_threshold: usize) -> Result<usize> {
    if share_count == 0 {
        return Err(Error::InvalidSquareSize { actual: 0 });
    }
    if subtree_root_threshold == 0 {
        return Err(Error::ZeroDivisor);
    }

    let threshold_multiple = share_count.div_ceil(subtree_root_threshold);
    let rounded = round_up_power_of_two(threshold_multiple);
    Ok(rounded.min(blob_min_square_size(share_count)?))
}

/// Return the next power of two greater than or equal to `input`.
#[must_use]
pub fn round_up_power_of_two(input: usize) -> usize {
    let mut result = 1usize;
    while result < input {
        let Some(next) = result.checked_mul(2) else {
            return usize::MAX;
        };
        result = next;
    }
    result
}

/// Return the greatest power of two less than or equal to `input`.
pub fn round_down_power_of_two(input: usize) -> Result<usize> {
    if input == 0 {
        return Err(Error::InvalidSquareSize { actual: 0 });
    }
    let shift = usize::BITS - 1 - input.leading_zeros();
    Ok(1usize << shift)
}

/// Return the next power of two strictly greater than `input`.
#[must_use]
pub fn round_up_power_of_two_strict(input: usize) -> usize {
    let rounded = round_up_power_of_two(input);
    if rounded == input {
        return rounded.checked_mul(2).unwrap_or(usize::MAX);
    }
    rounded
}

/// Return true when `input` is a non-zero power of two.
#[must_use]
pub const fn is_power_of_two(input: usize) -> bool {
    input.is_power_of_two()
}

pub(crate) fn ceil_sqrt(input: usize) -> usize {
    if input <= 1 {
        return input;
    }

    let mut low = 1usize;
    let mut high = input;
    while low < high {
        let mid = low + ((high - low) / 2);
        if mid >= input.div_ceil(mid) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}
