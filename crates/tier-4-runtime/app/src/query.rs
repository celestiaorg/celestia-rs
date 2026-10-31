//! ABCI `Query` (`baseapp.BaseApp.Query`): gRPC routes over committed
//! heights, `/store/<name>/{key,subspace}` with ICS-23 proofs, `/app/...`,
//! `/p2p/...` and `/custom/...`, with Go's height semantics and errors.

use celestia_proto::celestia_core_proto::abci::{RequestQuery, ResponseQuery};
use celestia_proto::celestia_core_proto::tendermint::crypto::{ProofOp, ProofOps};
use grpc_query::QueryError;
use sdk::error::sdk as errs;
use sdk::{Context, ExecMode, Header, MultiStore, SdkError};

use crate::{AbciApp, App};

/// `baseapp.QueryPathBroadcastTx`.
pub const QUERY_PATH_BROADCAST_TX: &str = "/cosmos.tx.v1beta1.Service/BroadcastTx";

/// celestia-app's `appconsts.AppName` (`app.Name()`).
pub const APP_NAME: &str = "celestia-app";

/// `sdkerrors.QueryResult(err)`.
#[must_use]
pub fn query_result(e: &SdkError) -> ResponseQuery {
    ResponseQuery {
        code: e.code,
        codespace: e.codespace.to_owned(),
        log: e.log.clone(),
        ..ResponseQuery::default()
    }
}

/// A query context: a read-only branch of a committed height.
#[derive(Debug)]
pub struct QueryState {
    /// The multistore view.
    pub ms: MultiStore,
    /// Header of the query context (check-state header at `height`).
    pub header: Header,
}

impl QueryState {
    /// A `Context` over the view (`isCheckTx = true`).
    pub fn context(&mut self) -> Context<'_> {
        Context::new(&mut self.ms, self.header.clone(), ExecMode::Check)
    }
}

/// Failure of a gRPC query (gRPC server path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrpcQueryError {
    /// No handler is registered for the path.
    Unimplemented(String),
    /// The path's service is not registered at the current app version
    /// (celestia-app v9.0.8 has no fibre / valaddr / `cosmos.reflection.v1`
    /// services): Go's `unknown service`.
    UnknownService(String),
    /// `CreateQueryContext` failed (a registered SDK error).
    Context(SdkError),
    /// The handler failed.
    Handler(QueryError),
}

/// Whether the query service of `path` is absent at `app_version`:
/// celestia-app v9.0.8 registers no fibre / valaddr query services and no
/// `cosmos.reflection.v1` service (ADR 0004).
fn absent_at(app_version: u64, path: &str) -> bool {
    sdk::version::is_v9(app_version)
        && (path.starts_with("/celestia.fibre.v1.")
            || path.starts_with("/celestia.valaddr.v1.")
            || path.starts_with("/cosmos.reflection.v1."))
}

impl GrpcQueryError {
    /// The gRPC status (code, message) Go's gRPC server would return.
    #[must_use]
    pub fn grpc_status(&self) -> (grpc_query::Code, String) {
        match self {
            Self::Unimplemented(path) => (
                grpc_query::Code::Unimplemented,
                format!("unknown method {path}"),
            ),
            Self::UnknownService(svc) => (
                grpc_query::Code::Unimplemented,
                format!("unknown service {svc}"),
            ),
            Self::Context(e) => QueryError::Sdk(e.clone()).grpc_status(),
            Self::Handler(e) => e.grpc_status(),
        }
    }
}

impl App {
    /// Replaces the pruning policy of the committed stores (e.g. keep every
    /// height of an in-memory app for historical queries).
    pub fn set_pruning(&mut self, pruning: multistore::PruningOptions) {
        self.ms.root_mut().set_pruning(pruning);
    }

    /// The latest committed height (`LastBlockHeight`).
    #[must_use]
    pub fn last_block_height(&self) -> i64 {
        self.last_height
    }

    /// `CreateQueryContext(height, prove)`.
    pub fn create_query_context(&self, height: i64, prove: bool) -> Result<QueryState, SdkError> {
        if height < 0 {
            return Err(errs::invalid_request(
                "cannot query with height < 0; please provide a valid height",
            ));
        }
        let last = self.ms.version();
        if last == 0 {
            return Err(errs::invalid_height(format!(
                "{APP_NAME} is not ready; please wait for first block"
            )));
        }
        if height > last {
            return Err(errs::invalid_height(
                "cannot query with height in the future; please provide a valid height",
            ));
        }
        let height = if height == 0 { last } else { height };
        if height <= 1 && prove {
            return Err(errs::invalid_request(
                "cannot query with proof when height <= 1; please provide a valid height",
            ));
        }
        let ms = self.ms.view_at(height).map_err(|_| {
            errs::invalid_request(format!(
                "failed to load state at height {height}; version mismatch on immutable IAVL tree; version does not exist. Version has either been pruned, or is for a future block height (latest height: {last})"
            ))
        })?;
        let mut header = self.check_header.clone();
        header.height = height;
        if height != last
            && let Some(ts) = self
                .ms
                .root()
                .get_commit_info(height)
                .ok()
                .and_then(|ci| ci.timestamp)
        {
            header.time = pbjson_types::Timestamp {
                seconds: ts.seconds,
                nanos: ts.nanos,
            };
        }
        Ok(QueryState { ms, header })
    }

    /// Runs the gRPC query `path` at `height` (0 = latest) and returns the
    /// response bytes and the height used (the gRPC server path, which
    /// never proves).
    pub fn grpc_query(
        &mut self,
        path: &str,
        data: &[u8],
        height: i64,
    ) -> Result<(Vec<u8>, i64), GrpcQueryError> {
        sdk::version::set_app_version(self.app_version());
        if absent_at(self.app_version(), path) {
            let svc = path
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or_default();
            return Err(GrpcQueryError::UnknownService(svc.to_owned()));
        }
        if let Some(handler) = crate::query_services::route(path) {
            let mut state = self
                .create_query_context(height, false)
                .map_err(GrpcQueryError::Context)?;
            let h = state.header.height;
            return handler(self, &mut state, data)
                .map(|v| (v, h))
                .map_err(GrpcQueryError::Handler);
        }
        let Some(handler) = grpc_query::router().route(path) else {
            return Err(GrpcQueryError::Unimplemented(path.to_owned()));
        };
        let mut state = self
            .create_query_context(height, false)
            .map_err(GrpcQueryError::Context)?;
        let h = state.header.height;
        let mut ctx = state.context();
        handler(&mut ctx, data)
            .map(|v| (v, h))
            .map_err(GrpcQueryError::Handler)
    }

    /// `Query`.
    pub fn query(&mut self, req: &RequestQuery) -> ResponseQuery {
        sdk::version::set_app_version(self.app_version());
        let mut req = req.clone();
        if req.height == 0 {
            req.height = self.last_height;
        }
        if req.path == QUERY_PATH_BROADCAST_TX {
            return query_result(&errs::invalid_request("can't route a broadcast tx message"));
        }
        let absent_v9 = absent_at(self.app_version(), &req.path);
        let app_route = crate::query_services::route(&req.path);
        if !absent_v9 && (app_route.is_some() || grpc_query::router().route(&req.path).is_some()) {
            return self.handle_query_grpc(&req);
        }
        let path: Vec<&str> = split_path(&req.path);
        if path.is_empty() {
            return query_result(&errs::unknown_request("no query path provided"));
        }
        match path[0] {
            "app" => self.handle_query_app(&path, &req),
            "store" => self.handle_query_store(&path, &req),
            "p2p" => handle_query_p2p(&path),
            "custom" => self.handle_query_custom(&path, &req),
            _ => query_result(&errs::unknown_request("unknown query path")),
        }
    }

    fn handle_query_grpc(&mut self, req: &RequestQuery) -> ResponseQuery {
        let result = if let Some(handler) = crate::query_services::route(&req.path) {
            match self.create_query_context(req.height, req.prove) {
                Err(e) => return query_result(&e),
                Ok(mut state) => handler(self, &mut state, &req.data),
            }
        } else {
            let Some(handler) = grpc_query::router().route(&req.path) else {
                return query_result(&errs::unknown_request("unknown query path"));
            };
            match self.create_query_context(req.height, req.prove) {
                Err(e) => return query_result(&e),
                Ok(mut state) => {
                    let mut ctx = state.context();
                    handler(&mut ctx, &req.data)
                }
            }
        };
        match result {
            Ok(value) => ResponseQuery {
                height: req.height,
                value,
                ..ResponseQuery::default()
            },
            // A recovered panic is reported by `BaseApp.Query`'s deferred
            // handler, without a height.
            Err(e) if e.is_panic() => query_result(&e.to_sdk()),
            Err(e) => ResponseQuery {
                height: req.height,
                ..query_result(&e.to_sdk())
            },
        }
    }

    fn handle_query_app(&mut self, path: &[&str], req: &RequestQuery) -> ResponseQuery {
        if path.len() >= 2 {
            match path[1] {
                "simulate" => {
                    return match crate::query_services::simulate_json(self, &req.data) {
                        Ok(value) => ResponseQuery {
                            codespace: "sdk".into(),
                            height: req.height,
                            value,
                            ..ResponseQuery::default()
                        },
                        Err(e) => query_result(&e.wrap("failed to simulate tx")),
                    };
                }
                "version" => {
                    return ResponseQuery {
                        codespace: "sdk".into(),
                        height: req.height,
                        value: crate::COMPATIBILITY_VERSION.as_bytes().to_vec(),
                        ..ResponseQuery::default()
                    };
                }
                _ => {
                    return query_result(&errs::unknown_request(format!(
                        "unknown query: [{}]",
                        path.join(" ")
                    )));
                }
            }
        }
        query_result(&errs::unknown_request(
            "expected second parameter to be either 'simulate' or 'version', neither was present",
        ))
    }

    fn handle_query_store(&mut self, path: &[&str], req: &RequestQuery) -> ResponseQuery {
        let sub = format!("/{}", path[1..].join("/"));
        if req.height <= 1 && req.prove {
            return query_result(&errs::invalid_request(
                "cannot query with proof when height <= 1; please provide a valid height",
            ));
        }
        match self.store_query(&sub, req) {
            Ok(mut resp) => {
                resp.height = req.height;
                resp
            }
            Err(e) => query_result(&e),
        }
    }

    /// `rootmulti.Store.Query`.
    fn store_query(&self, path: &str, req: &RequestQuery) -> Result<ResponseQuery, SdkError> {
        let Some(rest) = path.strip_prefix('/') else {
            return Err(store_errs::unknown_request(format!("invalid path: {path}")));
        };
        let (store_name, subpath) = match rest.split_once('/') {
            Some((s, sub)) => (s, format!("/{sub}")),
            None => (rest, String::new()),
        };
        match self.ms.kind(store_name) {
            Err(_) => {
                return Err(store_errs::unknown_request(format!(
                    "no such store: {store_name}"
                )));
            }
            Ok(sdk::Kind::Transient) => {
                return Err(store_errs::unknown_request(format!(
                    "store {store_name} (type *transient.Store) doesn't support queries"
                )));
            }
            Ok(sdk::Kind::Memory) => {
                return Err(store_errs::unknown_request(format!(
                    "store {store_name} (type *mem.Store) doesn't support queries"
                )));
            }
            Ok(sdk::Kind::Iavl) => {}
        }
        let res = self.iavl_query(store_name, &subpath, req);
        if !req.prove || subpath != "/key" {
            return res;
        }
        // With a proof requested Go ignores the store's error and fails on
        // the missing proof instead.
        let mut res = res.unwrap_or_default();
        let Some(ops) = res.proof_ops.as_mut().filter(|o| !o.ops.is_empty()) else {
            return Err(store_errs::invalid_request(
                "proof is unexpectedly empty; ensure height has not been pruned",
            ));
        };
        let info = self
            .ms
            .root()
            .get_commit_info(res.height)
            .map_err(|e| store_errs::logic(e.to_string()))?;
        let op = info
            .proof_op(store_name)
            .map_err(|e| store_errs::logic(e.to_string()))?;
        ops.ops.push(ProofOp {
            r#type: op.field_type,
            key: op.key,
            data: op.data,
        });
        Ok(res)
    }

    /// `iavl.Store.Query`.
    fn iavl_query(
        &self,
        store_name: &str,
        subpath: &str,
        req: &RequestQuery,
    ) -> Result<ResponseQuery, SdkError> {
        if req.data.is_empty() {
            return Err(store_errs::tx_decode("query cannot be zero length"));
        }
        let logic = |e: &dyn std::fmt::Display| store_errs::logic(e.to_string());
        let store = self
            .ms
            .root()
            .get_store(store_name)
            .map_err(|e| logic(&e))?;
        let tree = store.tree();
        let height = req.height;
        let mut res = ResponseQuery {
            height,
            ..ResponseQuery::default()
        };
        match subpath {
            "/key" => {
                res.key.clone_from(&req.data);
                if !tree.version_exists(height) {
                    res.log = "version does not exist".into();
                    return Ok(res);
                }
                let view = tree.get_immutable(height).map_err(|e| logic(&e))?;
                res.value = view
                    .get(&req.data)
                    .map_err(|e| logic(&e))?
                    .unwrap_or_default();
                if req.prove {
                    let proof = view
                        .get_with_proof(&req.data)
                        .map_err(|e| logic(&e))?
                        .ok_or_else(|| store_errs::logic("no proof"))?;
                    let ops = proof
                        .ops()
                        .iter()
                        .map(|op| ProofOp {
                            r#type: ics23_proofs::PROOF_OP_IAVL_COMMITMENT.into(),
                            key: op.key().to_vec(),
                            data: op.data().to_vec(),
                        })
                        .collect();
                    res.proof_ops = Some(ProofOps { ops });
                }
            }
            "/subspace" => {
                res.key.clone_from(&req.data);
                let end = sdk::context::prefix_end(&req.data);
                let committed = tree.get_immutable(tree.version()).map_err(|e| logic(&e))?;
                let mut pairs = Vec::new();
                for e in committed.iter(Some(&req.data), end.as_deref()) {
                    let e = e.map_err(|e| logic(&e))?;
                    let mut pair = Vec::new();
                    encode_bytes_field(&mut pair, 1, &e.key);
                    encode_bytes_field(&mut pair, 2, &e.value);
                    encode_bytes_field(&mut pairs, 1, &pair);
                }
                res.value = pairs;
            }
            other => {
                return Err(store_errs::unknown_request(format!(
                    "unexpected query path: {other}"
                )));
            }
        }
        Ok(res)
    }

    fn handle_query_custom(&mut self, path: &[&str], req: &RequestQuery) -> ResponseQuery {
        if path.len() < 2 || path[1].is_empty() {
            return query_result(&errs::unknown_request(
                "no route for custom query specified",
            ));
        }
        if !crate::query_services::has_custom_route(path[1]) {
            return query_result(&errs::unknown_request(format!(
                "no custom querier found for route {}",
                path[1]
            )));
        }
        let mut state = match self.create_query_context(req.height, req.prove) {
            Ok(s) => s,
            Err(e) => return query_result(&e),
        };
        match crate::query_services::custom(self, &mut state, path[1], &path[2..], req) {
            Ok(value) => ResponseQuery {
                height: req.height,
                value,
                ..ResponseQuery::default()
            },
            Err(e) => ResponseQuery {
                height: req.height,
                ..query_result(&e)
            },
        }
    }
}

fn handle_query_p2p(path: &[&str]) -> ResponseQuery {
    if path.len() < 4 {
        return query_result(&errs::unknown_request(
            "path should be p2p filter <addr|id> <parameter>",
        ));
    }
    if path[1] != "filter" {
        return query_result(&errs::unknown_request(
            "expected second parameter to be 'filter'",
        ));
    }
    // No peer filters are configured: every peer is accepted.
    ResponseQuery::default()
}

/// `cosmossdk.io/store/types` errors (codespace `store`).
mod store_errs {
    use sdk::SdkError;

    fn err(code: u32, desc: &str, msg: impl AsRef<str>) -> SdkError {
        SdkError::new("store", code, format!("{}: {desc}", msg.as_ref()))
    }

    pub fn tx_decode(msg: impl AsRef<str>) -> SdkError {
        err(3, "tx parse error", msg)
    }

    pub fn unknown_request(msg: impl AsRef<str>) -> SdkError {
        err(4, "unknown request", msg)
    }

    pub fn logic(msg: impl AsRef<str>) -> SdkError {
        err(5, "internal logic error", msg)
    }

    pub fn invalid_request(msg: impl AsRef<str>) -> SdkError {
        err(7, "invalid request", msg)
    }
}

/// `SplitABCIQueryPath`.
#[must_use]
pub fn split_path(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    if parts.first() == Some(&"") {
        parts.remove(0);
    }
    parts
}

fn encode_bytes_field(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    prost::encoding::encode_key(field, prost::encoding::WireType::LengthDelimited, out);
    prost::encoding::encode_varint(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

impl AbciApp {
    /// Runs `f` on the app under the ABCI lock (the gRPC and REST servers
    /// share the app with the ABCI connections). `None` if the lock is
    /// poisoned.
    pub fn with_app<R>(&self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        self.inner.lock().ok().map(|mut app| f(&mut app))
    }

    /// A gRPC query (the gRPC server path): celestia-core-backed services
    /// outside the lock, every other service under it.
    pub fn grpc_query(
        &self,
        path: &str,
        data: &[u8],
        height: i64,
    ) -> Result<(Vec<u8>, i64), GrpcQueryError> {
        if path.starts_with(grpc_query::reflection_extra::V2ALPHA1) {
            let chain_id = self
                .with_app(|a| a.chain_id().to_owned())
                .unwrap_or_default();
            if let Some(r) = grpc_query::reflection_extra::v2alpha1(path, data, &chain_id) {
                // No `x-cosmos-block-height`: Go serves it outside the router.
                return r.map(|b| (b, -1)).map_err(GrpcQueryError::Handler);
            }
        }
        if let Some(r) = crate::node_services::grpc_query(self, path, data, height) {
            return r;
        }
        self.with_app(|a| a.grpc_query(path, data, height))
            .unwrap_or_else(|| {
                Err(GrpcQueryError::Handler(grpc_query::QueryError::Status(
                    grpc_query::Code::Internal,
                    "app lock poisoned".into(),
                )))
            })
    }
}
