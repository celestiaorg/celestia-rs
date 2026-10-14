//! Port of celestia-core v0.42.3 `rpc/` (ADR 0003): the CometBFT JSON-RPC
//! server — URI and JSON-RPC 2.0 over HTTP, WebSocket with event
//! subscriptions — every route of `rpc/core/routes.go` including
//! celestia's (`prove_shares[_v2]`, `data_root_inclusion_proof`,
//! `data_commitment`, `signed_block`, `tx_status[_batch]`); the event bus
//! (`types/event_bus.go`, `libs/pubsub` + its query language); the kv tx
//! and block indexers (`state/txindex/kv`, `state/indexer/block/kv`); and
//! the celestia-core gRPC services (`rpc/grpc`: BlockAPI, BlobstreamAPI,
//! BroadcastAPI).
//!
//! Responses are encoded exactly as Go writes them ([`json`],
//! [`encode`]); the differential corpus in `conformance/rpccorpus`
//! (`tests/corpus.rs`) checks them byte for byte against a Go node.

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability.
#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::result_large_err,
    clippy::type_complexity,
    clippy::naive_bytecount,
    clippy::similar_names,
    clippy::assigning_clones,
    clippy::unnecessary_sort_by,
    clippy::redundant_guards,
    clippy::cast_possible_wrap,
    clippy::iter_not_returning_iterator,
    clippy::zero_sized_map_values,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::match_wildcard_for_single_variants
)]

pub mod encode;
pub mod env;
pub mod events;
pub mod grpc;
pub mod indexer;
pub mod json;
pub mod params;
pub mod query;
pub mod server;

pub use env::{
    AppQuery, Environment, EvidenceSink, GenesisData, NodeInfo, NodeStatus, PeerInfo, RpcConfig,
};
pub use events::{BlockEventPublisher, EventBus, EventData};
pub use indexer::{BlockIndexer, TxIndexer};
