//! Share range helpers.

use crate::{Error, Namespace, Result, Share};

/// End-exclusive range of share indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareRange {
    /// Index of the first share in the range.
    pub start: usize,
    /// Index one past the last share in the range.
    pub end: usize,
}

impl ShareRange {
    /// Construct an end-exclusive share range.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Construct an empty share range.
    #[must_use]
    pub const fn empty() -> Self {
        Self { start: 0, end: 0 }
    }

    /// Return true when this range is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == 0 && self.end == 0
    }

    /// Offset the range by `value`.
    ///
    /// If either endpoint would overflow, returns an error and leaves the range unchanged.
    pub fn add(&mut self, value: usize) -> Result<()> {
        let start = self
            .start
            .checked_add(value)
            .ok_or(Error::InvalidShareRange)?;
        let end = self
            .end
            .checked_add(value)
            .ok_or(Error::InvalidShareRange)?;
        *self = Self::new(start, end);
        Ok(())
    }
}

/// Return the contiguous range of shares for `namespace`.
pub fn get_share_range_for_namespace(shares: &[Share], namespace: Namespace) -> Result<ShareRange> {
    if shares.is_empty() {
        return Ok(ShareRange::empty());
    }
    if namespace < shares[0].namespace()? || namespace > shares[shares.len() - 1].namespace()? {
        return Ok(ShareRange::empty());
    }

    let mut start = None;
    for (index, share) in shares.iter().enumerate() {
        let share_namespace = share.namespace()?;
        if share_namespace > namespace && start.is_some() {
            return Ok(ShareRange::new(
                start.ok_or(Error::InvalidShareRange)?,
                index,
            ));
        }
        if share_namespace == namespace && start.is_none() {
            start = Some(index);
        }
    }

    Ok(start.map_or_else(ShareRange::empty, |start| {
        ShareRange::new(start, shares.len())
    }))
}

#[cfg(kani)]
mod proofs {
    use super::{Error, ShareRange};

    #[kani::proof]
    fn offset_updates_both_endpoints_or_leaves_range_unchanged() {
        let original = ShareRange::new(kani::any(), kani::any());
        let offset: usize = kani::any();
        let (start, start_overflow) = original.start.overflowing_add(offset);
        let (end, end_overflow) = original.end.overflowing_add(offset);

        let mut range = original;
        let result = range.add(offset);

        if start_overflow || end_overflow {
            assert_eq!(result, Err(Error::InvalidShareRange));
            assert_eq!(range, original);
        } else {
            assert_eq!(result, Ok(()));
            assert_eq!(range, ShareRange::new(start, end));
        }
    }
}
