//! The mempool's ABCI connection (`proxy.AppConnMempool`).
//!
//! The node calls the application in process (ADR 0003), so this is a
//! plain trait: the node implements it over its `app::App` (keeping ABCI's
//! connection separation: the mempool connection only issues `CheckTx`,
//! `QuerySequence` and `Flush`). An `Err` is a client-level error (Go's
//! `err` from the ABCI client, e.g. a `ResponseException`), not a
//! non-zero CheckTx code.

use celestia_core_proto::abci::{RequestCheckTx, ResponseCheckTx};

/// The application's mempool connection.
pub trait AppConnMempool: Send + Sync {
    /// `CheckTx` (`type` New or Recheck).
    fn check_tx(&self, req: RequestCheckTx) -> Result<ResponseCheckTx, String>;

    /// `QuerySequence`: the next sequence the app expects from `signer`.
    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String>;

    /// `Flush`: waits for outstanding requests (a no-op in process).
    fn flush(&self) -> Result<(), String> {
        Ok(())
    }

    /// `Error`: the connection's sticky error, if any.
    fn error(&self) -> Result<(), String> {
        Ok(())
    }
}
