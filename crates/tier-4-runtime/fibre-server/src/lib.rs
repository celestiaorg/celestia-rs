//! Fibre data availability server: the Rust port of celestia-app v11's
//! `fibre.Server`. A validator accepts the blob shards assigned to it,
//! verifies them, stores them until the payment promise expires, signs the
//! promise with its consensus key and serves the shards to download clients.

pub mod assignment;
pub mod occupancy;
pub mod params;
pub mod store;
pub mod tlsid;
pub mod verify;
