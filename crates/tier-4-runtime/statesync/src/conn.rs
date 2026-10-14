//! The ABCI connections state sync uses (`proxy.AppConnSnapshot` and
//! `proxy.AppConnQuery.Info`), and an adapter for in-process apps.

use std::sync::Arc;

use abci_server::Application;
use celestia_core_proto::tendermint::abci::{self, request, response};

/// Snapshot + info calls into the application.
pub trait AppConn: Send + Sync {
    /// `ListSnapshots`.
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String>;
    /// `LoadSnapshotChunk`.
    fn load_snapshot_chunk(
        &self,
        req: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String>;
    /// `OfferSnapshot`.
    fn offer_snapshot(
        &self,
        req: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String>;
    /// `ApplySnapshotChunk`.
    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String>;
    /// `Info` (query connection).
    fn info(&self, req: abci::RequestInfo) -> Result<abci::ResponseInfo, String>;
}

/// An in-process application behind the ABCI `Application` interface
/// (e.g. `app::AbciApp`).
pub struct InProcessConn<A: Application>(pub Arc<A>);

impl<A: Application> std::fmt::Debug for InProcessConn<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InProcessConn")
    }
}

fn unexpected(what: &str, got: &response::Value) -> String {
    match got {
        response::Value::Exception(e) => e.error.clone(),
        other => format!("unexpected {what} response {other:?}"),
    }
}

impl<A: Application> AppConn for InProcessConn<A> {
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
        match self
            .0
            .handle(request::Value::ListSnapshots(abci::RequestListSnapshots {}))
        {
            response::Value::ListSnapshots(r) => Ok(r),
            other => Err(unexpected("ListSnapshots", &other)),
        }
    }

    fn load_snapshot_chunk(
        &self,
        req: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
        match self.0.handle(request::Value::LoadSnapshotChunk(req)) {
            response::Value::LoadSnapshotChunk(r) => Ok(r),
            other => Err(unexpected("LoadSnapshotChunk", &other)),
        }
    }

    fn offer_snapshot(
        &self,
        req: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String> {
        match self.0.handle(request::Value::OfferSnapshot(req)) {
            response::Value::OfferSnapshot(r) => Ok(r),
            other => Err(unexpected("OfferSnapshot", &other)),
        }
    }

    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        match self.0.handle(request::Value::ApplySnapshotChunk(req)) {
            response::Value::ApplySnapshotChunk(r) => Ok(r),
            other => Err(unexpected("ApplySnapshotChunk", &other)),
        }
    }

    fn info(&self, req: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        match self.0.handle(request::Value::Info(req)) {
            response::Value::Info(r) => Ok(r),
            other => Err(unexpected("Info", &other)),
        }
    }
}
