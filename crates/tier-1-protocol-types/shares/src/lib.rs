//! Celestia share encoding and decoding primitives.
//!
//! See `docs/superpowers/specs/m1/shares.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod blob;
pub mod builder;
pub mod compact;
pub mod error;
pub mod info_byte;
pub mod namespace;
pub mod padding;
pub mod parse;
pub mod range;
pub mod reserved;
pub mod share;
pub mod sparse;

pub use blob::{Blob, IndexWrapper};
pub use builder::ShareBuilder;
pub use compact::{CompactShareSplitter, SplitTxsResult, marshal_delimited_tx, split_txs};
pub use error::{Error, Result};
pub use info_byte::InfoByte;
pub use namespace::{
    INTERMEDIATE_STATE_ROOTS_NAMESPACE, MAX_PRIMARY_RESERVED_NAMESPACE,
    MIN_SECONDARY_RESERVED_NAMESPACE, NAMESPACE_ID_SIZE, NAMESPACE_SIZE, NAMESPACE_VERSION_MAX,
    NAMESPACE_VERSION_SIZE, NAMESPACE_VERSION_ZERO, NAMESPACE_VERSION_ZERO_ID_SIZE,
    NAMESPACE_VERSION_ZERO_PREFIX_SIZE, Namespace, PARITY_SHARES_NAMESPACE, PAY_FOR_BLOB_NAMESPACE,
    PAY_FOR_FIBRE_NAMESPACE, PRIMARY_RESERVED_PADDING_NAMESPACE, TAIL_PADDING_NAMESPACE,
    TX_NAMESPACE,
};
pub use padding::{
    namespace_padding_share, namespace_padding_shares, reserved_padding_share,
    reserved_padding_shares, tail_padding_share, tail_padding_shares,
};
pub use parse::{parse_blobs, parse_compact_shares, parse_sparse_shares, parse_txs};
pub use range::{ShareRange, get_share_range_for_namespace};
pub use reserved::{ReservedBytes, parse_reserved_bytes};
pub use share::{
    COMPACT_SHARE_RESERVED_BYTES, CONTINUATION_COMPACT_SHARE_CONTENT_SIZE,
    CONTINUATION_SPARSE_SHARE_CONTENT_SIZE, DEFAULT_SHARE_VERSION, FIBRE_BLOB_VERSION_SIZE,
    FIBRE_COMMITMENT_SIZE, FIRST_COMPACT_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER, MAX_SHARE_VERSION, MIN_SHARE_COUNT,
    MIN_SQUARE_SIZE, SEQUENCE_LEN_BYTES, SHARE_INFO_BYTES, SHARE_SIZE, SHARE_VERSION_ONE,
    SHARE_VERSION_TWO, SHARE_VERSION_ZERO, SIGNER_SIZE, SUPPORTED_SHARE_VERSIONS, Share,
    shares_from_bytes, shares_to_bytes,
};
pub use sparse::{SparseShareSplitter, split_blobs};
