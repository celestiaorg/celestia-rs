//! CometBFT protobuf types used by this crate.

/// ABCI protobuf types.
pub mod abci {
    pub use tendermint_proto::v0_38::abci::*;
}

/// CometBFT crypto protobuf types.
pub mod crypto {
    pub use tendermint_proto::v0_38::crypto::*;
}

/// Google protobuf helper types.
pub mod google {
    pub use tendermint_proto::google::protobuf::Timestamp;
}

/// CometBFT core protobuf types.
pub mod types {
    pub use tendermint_proto::v0_38::types::*;
}
