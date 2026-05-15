//! CometBFT v0.38.12 protobuf types used by this crate.

/// Tendermint crypto protobuf types.
pub mod crypto {
    pub use tendermint_proto::v0_38::crypto::*;
}

/// Google protobuf helper types.
pub mod google {
    pub use tendermint_proto::google::protobuf::Timestamp;
}

/// Tendermint consensus protobuf types.
pub mod types {
    pub use tendermint_proto::v0_38::types::*;
}

/// Tendermint version protobuf types.
pub mod version {
    pub use tendermint_proto::v0_38::version::*;
}
