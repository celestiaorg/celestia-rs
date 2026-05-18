//! Celestia data-square layout and data-availability header primitives.
//!
//! See `docs/superpowers/specs/m1/square.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod blob_tx;
pub mod builder;
pub mod commitment;
pub mod dah;
pub mod erasure;
pub mod error;
pub mod inclusion;
pub mod square;

pub use blob_tx::{BlobTx, PROTO_BLOB_TX_TYPE_ID};
pub use builder::{
    BuildResult, Builder, Element, blob_share_range, build, construct, deconstruct, tx_share_range,
    worst_case_share_indexes,
};
pub use commitment::{
    COMMITMENT_HASH_SIZE, MerkleRootFn, NMT_NODE_SIZE, create_commitment, create_commitments,
    merkle_mountain_range_sizes,
};
pub use dah::{
    DAH_HASH_SIZE, DAH_ROOT_SIZE, DataAvailabilityHeader, data_availability_header_from_proto,
    min_data_availability_header, new_data_availability_header,
};
pub use erasure::{
    ErasureCodec, ExtendedDataSquare, LeoRsCodec, compute_extended_data_square, extend_shares,
    import_extended_data_square,
};
pub use error::{Error, Result};
pub use inclusion::{
    blob_min_square_size, blob_shares_used_non_interactive_defaults, is_power_of_two,
    next_share_index, round_down_power_of_two, round_up_by_multiple_of, round_up_power_of_two,
    round_up_power_of_two_strict, sub_tree_width,
};
pub use square::{
    DEFAULT_MAX_SQUARE_SIZE, DEFAULT_SUBTREE_ROOT_THRESHOLD, Square, empty_square, size,
    write_square,
};
