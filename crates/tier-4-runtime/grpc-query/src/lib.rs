//! gRPC query services of celestia-app v10 (`baseapp.GRPCQueryRouter`).
//!
//! Every module query service Go registers through
//! `module.Manager.RegisterServices` is a set of handlers keyed by the full
//! gRPC method path (`/cosmos.bank.v1beta1.Query/Balance`). A handler takes
//! the protobuf request bytes and a query [`Context`] (a read-only branch of
//! a committed height, see `app::query`) and returns the protobuf response
//! bytes or a [`QueryError`] that keeps the Go error's shape, so both the
//! ABCI query path and the gRPC server can render it exactly like Go.

#![allow(clippy::similar_names)]

pub mod error;
pub mod pagination;

mod auth;
mod authz;
mod bank;
mod celestia;
mod circuit_evidence;
mod distribution;
mod feegrant;
mod gov;
mod hyperlane;
mod ibc;
mod reflection;
mod reflection_table;
mod sdk_misc;
mod slashing;
mod staking;
pub mod util;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use prost::Message;
use sdk::Context;

pub use error::{Code, QResult, QueryError};

/// A query handler over protobuf bytes.
pub type Handler = Box<dyn Fn(&mut Context<'_>, &[u8]) -> QResult<Vec<u8>> + Send + Sync>;

/// Handlers by full gRPC method path.
#[derive(Default)]
pub struct Router {
    routes: BTreeMap<&'static str, Handler>,
}

impl core::fmt::Debug for Router {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Router")
            .field("routes", &self.routes.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Router {
    /// Registers a typed handler (`Unmarshal` → handler → `Marshal`).
    pub fn add<Req, Resp>(
        &mut self,
        path: &'static str,
        f: fn(&mut Context<'_>, Req) -> QResult<Resp>,
    ) where
        Req: Message + Default + 'static,
        Resp: Message + 'static,
    {
        self.routes.insert(
            path,
            Box::new(move |ctx, data| {
                let req = Req::decode(data).map_err(|e| decode_error(&e))?;
                Ok(f(ctx, req)?.encode_to_vec())
            }),
        );
    }

    /// The handler of `path` (`GRPCQueryRouter.Route`).
    #[must_use]
    pub fn route(&self, path: &str) -> Option<&Handler> {
        self.routes.get(path)
    }

    /// Whether any method of `service` (full name, no slashes) is
    /// registered.
    #[must_use]
    pub fn has_service(&self, service: &str) -> bool {
        let prefix = format!("/{service}/");
        self.routes.keys().any(|k| k.starts_with(&prefix))
    }

    /// All registered method paths.
    pub fn paths(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.routes.keys().copied()
    }
}

/// A request decode failure as gogoproto's `Unmarshal` reports it (truncated
/// input is `io.ErrUnexpectedEOF`); other malformed input keeps prost's
/// description.
#[must_use]
pub fn decode_error(e: &prost::DecodeError) -> QueryError {
    let s = e.to_string();
    if s.contains("buffer underflow") || s.contains("invalid varint") {
        return QueryError::Plain("unexpected EOF".into());
    }
    QueryError::Plain(s)
}

/// The module query router (every service `RegisterServices` registers).
pub fn router() -> &'static Router {
    static ROUTER: OnceLock<Router> = OnceLock::new();
    ROUTER.get_or_init(|| {
        let mut r = Router::default();
        auth::register(&mut r);
        authz::register(&mut r);
        bank::register(&mut r);
        celestia::register(&mut r);
        circuit_evidence::register(&mut r);
        distribution::register(&mut r);
        feegrant::register(&mut r);
        gov::register(&mut r);
        hyperlane::register(&mut r);
        ibc::register(&mut r);
        reflection::register(&mut r);
        sdk_misc::register(&mut r);
        slashing::register(&mut r);
        staking::register(&mut r);
        r
    })
}
