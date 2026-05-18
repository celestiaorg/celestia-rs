//! Share padding constructors.

use crate::share::{DEFAULT_SHARE_VERSION, FIRST_SPARSE_SHARE_CONTENT_SIZE};
use crate::{
    Namespace, PRIMARY_RESERVED_PADDING_NAMESPACE, Result, Share, ShareBuilder,
    TAIL_PADDING_NAMESPACE,
};

/// Construct a namespace padding share.
pub fn namespace_padding_share(namespace: Namespace, share_version: u8) -> Result<Share> {
    let mut builder = ShareBuilder::new(namespace, share_version, true)?;
    builder.write_sequence_len(0)?;
    let padding = vec![0; FIRST_SPARSE_SHARE_CONTENT_SIZE];
    let leftover = builder.add_data(&padding);
    debug_assert!(leftover.is_none());
    builder.build()
}

/// Construct `count` namespace padding shares.
pub fn namespace_padding_shares(
    namespace: Namespace,
    share_version: u8,
    count: usize,
) -> Result<Vec<Share>> {
    (0..count)
        .map(|_| namespace_padding_share(namespace, share_version))
        .collect()
}

/// Construct a primary reserved padding share.
#[must_use]
pub fn reserved_padding_share() -> Share {
    namespace_padding_share(PRIMARY_RESERVED_PADDING_NAMESPACE, DEFAULT_SHARE_VERSION)
        .unwrap_or_else(|_| unreachable!("reserved padding namespace is valid"))
}

/// Construct primary reserved padding shares.
#[must_use]
pub fn reserved_padding_shares(count: usize) -> Vec<Share> {
    (0..count).map(|_| reserved_padding_share()).collect()
}

/// Construct a tail padding share.
#[must_use]
pub fn tail_padding_share() -> Share {
    namespace_padding_share(TAIL_PADDING_NAMESPACE, DEFAULT_SHARE_VERSION)
        .unwrap_or_else(|_| unreachable!("tail padding namespace is valid"))
}

/// Construct tail padding shares.
#[must_use]
pub fn tail_padding_shares(count: usize) -> Vec<Share> {
    (0..count).map(|_| tail_padding_share()).collect()
}
