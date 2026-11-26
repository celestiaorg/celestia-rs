//! TLS and HTTP/2 serving (Go `internal/grpc/server.go` with
//! `credentials.NewTLS`).
//!
//! - TLS 1.3 only, ALPN `h2`, no client authentication, with the `tlsid`
//!   certificate minted at start.
//! - At most `max_connections` connections: the accept loop takes a slot
//!   before it accepts (Go `netutil.LimitListener`), so extra peers wait in
//!   the kernel backlog.
//! - At most `max_concurrent_streams` streams per connection (HTTP/2
//!   `SETTINGS_MAX_CONCURRENT_STREAMS`).
//! - The TLS handshake must finish within the connection timeout (15 s);
//!   keepalive pings every 2 min with a 20 s ack deadline; a connection
//!   without requests for 30 s, or older than 5 min (±10%), is sent a
//!   GOAWAY, and an aged one is closed after a further 2 min grace.
//! - Messages up to the protocol's maximum message size in both directions;
//!   the codec's pre-allocation limits (see [`crate::codec`]).
//! - A panic in a handler becomes `Internal` `internal error`.
//! - Graceful stop: GOAWAY on every connection and wait for in-flight
//!   requests; the caller forces the rest closed after its deadline.

use std::convert::Infallible;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use celestia_proto::celestia::fibre::v1::{
    DownloadShardRequest, DownloadShardResponse, UploadShardRequest, UploadShardResponse,
};
use futures_util::FutureExt as _;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, Request, Response};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tonic::Status;
use tonic::server::{Grpc, UnaryService};

use crate::codec::{FibreCodec, Inbound, Outbound};
use crate::config::{ServerConfig, Tuning};
use crate::proto::{
    HealthCheckRequest, HealthCheckResponse, HealthListRequest, HealthListResponse,
};
use crate::server::Inner;
use crate::service::CallContext;
use crate::tlsid::ServerCert;

/// The Fibre service name.
pub const FIBRE_SERVICE: &str = "celestia.fibre.v1.Fibre";
/// The health service name.
pub const HEALTH_SERVICE: &str = "grpc.health.v1.Health";

/// Transport settings derived from the configuration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransportOptions {
    pub(crate) max_connections: usize,
    pub(crate) max_concurrent_streams: u32,
    pub(crate) max_message_size: usize,
    pub(crate) tuning: Tuning,
}

impl TransportOptions {
    pub(crate) fn from_config(cfg: &ServerConfig) -> Self {
        Self {
            max_connections: usize::try_from(cfg.max_connections).unwrap_or(1).max(1),
            max_concurrent_streams: u32::try_from(cfg.max_concurrent_streams).unwrap_or(u32::MAX),
            max_message_size: cfg.max_message_size,
            tuning: cfg.tuning,
        }
    }
}

/// The server's TLS configuration (Go `credentials.NewTLS(&tls.Config{
/// Certificates: [cert], MinVersion: TLS13})`, which also sets ALPN `h2`).
pub(crate) fn tls_config(cert: &ServerCert) -> Result<Arc<rustls::ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.private_key_pkcs8_der.clone())),
        )
        .map_err(|e| e.to_string())?;
    cfg.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Arc::new(cfg))
}

/// Accept errors grpc-go retries (`Temporary()` network errors).
fn is_temporary(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        e.kind(),
        ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::Interrupted
            | ErrorKind::WouldBlock
            | ErrorKind::OutOfMemory
    ) || matches!(e.raw_os_error(), Some(23 | 24 | 105 | 12))
}

/// Serves until `shutdown` (Ok) or a fatal accept error (Err).
pub(crate) async fn serve(
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    inner: Arc<Inner>,
    opts: TransportOptions,
    shutdown: CancellationToken,
    force: CancellationToken,
    connections: TaskTracker,
) -> std::io::Result<()> {
    let acceptor = TlsAcceptor::from(tls);
    let slots = Arc::new(Semaphore::new(opts.max_connections));
    let mut backoff = Duration::ZERO;
    loop {
        let permit = tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            p = Arc::clone(&slots).acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => return Ok(()),
            },
        };
        let accepted = tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            a = listener.accept() => a,
        };
        let (tcp, peer) = match accepted {
            Ok(a) => {
                backoff = Duration::ZERO;
                a
            }
            Err(e) if is_temporary(&e) => {
                backoff = if backoff.is_zero() {
                    Duration::from_millis(5)
                } else {
                    (backoff * 2).min(Duration::from_secs(1))
                };
                tracing::warn!(error = %e, retry_in = ?backoff, "accept failed; retrying");
                tokio::time::sleep(backoff).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        let _ = tcp.set_nodelay(true);
        let conn = Connection {
            inner: Arc::clone(&inner),
            acceptor: acceptor.clone(),
            opts,
            shutdown: shutdown.clone(),
            force: force.clone(),
        };
        connections.spawn(async move {
            let _permit = permit;
            conn.run(tcp, peer).await;
        });
    }
}

struct Connection {
    inner: Arc<Inner>,
    acceptor: TlsAcceptor,
    opts: TransportOptions,
    shutdown: CancellationToken,
    force: CancellationToken,
}

/// Requests in flight on one connection, for the idle timer.
#[derive(Default)]
struct Activity {
    active: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
}

impl Activity {
    fn begin(self: &Arc<Self>) -> ActivityGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(Arc::clone(self))
    }

    fn idle_for(&self) -> Option<Duration> {
        if self.active.load(Ordering::SeqCst) > 0 {
            return None;
        }
        self.idle_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map(|t| t.elapsed())
    }
}

struct ActivityGuard(Arc<Activity>);

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self
                .0
                .idle_since
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        }
    }
}

/// Go's `MaxConnectionAge` jitter: ±10%.
fn jittered(d: Duration) -> Duration {
    let mut b = [0u8; 8];
    let r = getrandom::fill(&mut b).map_or(0, |()| u64::from_le_bytes(b));
    #[allow(clippy::cast_precision_loss)]
    let f = 0.9 + 0.2 * ((r >> 11) as f64 / (1u64 << 53) as f64);
    d.mul_f64(f)
}

impl Connection {
    async fn run(self, tcp: tokio::net::TcpStream, peer: std::net::SocketAddr) {
        let metrics = Arc::clone(&self.inner.metrics);
        metrics.connection_opened();
        let _closed = scopeguard(move || metrics.connection_closed());

        let tuning = self.opts.tuning;
        let tls = tokio::select! {
            () = self.force.cancelled() => return,
            r = tokio::time::timeout(tuning.connection_timeout, self.acceptor.accept(tcp)) => r,
        };
        let tls = match tls {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                self.inner.metrics.handshake_failed();
                tracing::debug!(%peer, error = %e, "TLS handshake failed");
                return;
            }
            Err(_) => {
                self.inner.metrics.handshake_failed();
                tracing::debug!(%peer, "TLS handshake timed out");
                return;
            }
        };

        let activity = Arc::new(Activity {
            active: AtomicUsize::new(0),
            idle_since: Mutex::new(Some(Instant::now())),
        });
        let router = Router {
            inner: Arc::clone(&self.inner),
            activity: Arc::clone(&activity),
            max_message_size: self.opts.max_message_size,
        };
        let service = hyper::service::service_fn(move |req| {
            let router = router.clone();
            async move { Ok::<_, Infallible>(router.route(req).await) }
        });
        let mut builder = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
        builder
            .timer(TokioTimer::new())
            .max_concurrent_streams(self.opts.max_concurrent_streams)
            .adaptive_window(true)
            .keep_alive_interval(tuning.keepalive_ping_time)
            .keep_alive_timeout(tuning.keepalive_ping_timeout);
        let conn = builder.serve_connection(TokioIo::new(tls), service);
        tokio::pin!(conn);

        let age_deadline = tokio::time::Instant::now() + jittered(tuning.max_connection_age);
        let mut close_at: Option<tokio::time::Instant> = None;
        let mut draining = false;
        let mut idle_tick =
            tokio::time::interval((tuning.max_connection_idle / 4).max(Duration::from_millis(10)));
        loop {
            tokio::select! {
                res = conn.as_mut() => {
                    if let Err(e) = res {
                        tracing::debug!(%peer, error = %e, "connection closed with error");
                    }
                    return;
                }
                () = self.force.cancelled() => return,
                () = self.shutdown.cancelled(), if !draining => {
                    conn.as_mut().graceful_shutdown();
                    draining = true;
                }
                () = tokio::time::sleep_until(age_deadline), if !draining => {
                    conn.as_mut().graceful_shutdown();
                    draining = true;
                    close_at = Some(tokio::time::Instant::now() + tuning.max_connection_age_grace);
                }
                () = sleep_until_opt(close_at) => return,
                _ = idle_tick.tick(), if !draining => {
                    if activity.idle_for().is_some_and(|d| d >= tuning.max_connection_idle) {
                        conn.as_mut().graceful_shutdown();
                        draining = true;
                        // hyper finishes a pending HTTP/2 handshake before it
                        // honours the GOAWAY: a peer that completed TLS but
                        // never sent the preface would keep its slot.
                        close_at = Some(tokio::time::Instant::now() + tuning.connection_timeout);
                    }
                }
            }
        }
    }
}

async fn sleep_until_opt(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

struct ScopeGuard<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

fn scopeguard<F: FnOnce()>(f: F) -> ScopeGuard<F> {
    ScopeGuard(Some(f))
}

/// Parses the `grpc-timeout` header (`<digits><unit>`, unit one of
/// `H M S m u n`).
fn grpc_timeout(headers: &http::HeaderMap) -> Option<Duration> {
    let v = headers.get("grpc-timeout")?.to_str().ok()?;
    if v.len() < 2 || v.len() > 9 {
        return None;
    }
    let (digits, unit) = v.split_at(v.len() - 1);
    let n: u64 = digits.parse().ok()?;
    Some(match unit {
        "H" => Duration::from_secs(n.saturating_mul(3600)),
        "M" => Duration::from_secs(n.saturating_mul(60)),
        "S" => Duration::from_secs(n),
        "m" => Duration::from_millis(n),
        "u" => Duration::from_micros(n),
        "n" => Duration::from_nanos(n),
        _ => return None,
    })
}

#[derive(Clone)]
struct Router {
    inner: Arc<Inner>,
    activity: Arc<Activity>,
    max_message_size: usize,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Go's `recoverUnaryInterceptor`.
async fn recovering<T>(
    method: &'static str,
    fut: impl Future<Output = Result<T, Status>>,
) -> Result<T, Status> {
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(r) => r,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::error!(method, panic = %msg, "recovered from panic in gRPC handler");
            Err(Status::internal("internal error"))
        }
    }
}

type Handler<Req, Resp> =
    Arc<dyn Fn(tonic::Request<Req>) -> BoxFuture<Result<Resp, Status>> + Send + Sync>;

/// One unary method: the request type, the response type and the handler.
#[derive(Clone)]
struct Method<Req, Resp> {
    handler: Handler<Req, Resp>,
}

impl<Req: Send + 'static, Resp: Send + 'static> UnaryService<Req> for Method<Req, Resp> {
    type Response = Resp;
    type Future = BoxFuture<Result<tonic::Response<Resp>, Status>>;

    fn call(&mut self, request: tonic::Request<Req>) -> Self::Future {
        let fut = (self.handler)(request);
        Box::pin(async move { fut.await.map(tonic::Response::new) })
    }
}

impl Router {
    fn grpc<Req: Inbound, Resp: Outbound>(&self, count_codec: bool) -> Grpc<FibreCodec<Resp, Req>> {
        Grpc::new(FibreCodec::new(
            self.inner.codec_limits,
            self.max_message_size,
            count_codec.then(|| Arc::clone(&self.inner.metrics)),
        ))
        .max_decoding_message_size(self.max_message_size)
        .max_encoding_message_size(self.max_message_size)
    }

    async fn route(&self, req: Request<Incoming>) -> Response<axum::body::Body> {
        let _active = self.activity.begin();
        let content_type = req.headers().get(CONTENT_TYPE).cloned();
        let timeout = grpc_timeout(req.headers());
        let path = req.uri().path().to_owned();
        let resp = match path.as_str() {
            "/celestia.fibre.v1.Fibre/UploadShard" => {
                let inner = Arc::clone(&self.inner);
                let method = Method::<UploadShardRequest, UploadShardResponse> {
                    handler: Arc::new(move |r: tonic::Request<UploadShardRequest>| {
                        let inner = Arc::clone(&inner);
                        Box::pin(async move {
                            let ctx =
                                timeout.map_or_else(CallContext::new, CallContext::with_timeout);
                            let _cancel_on_drop = ctx.token().clone().drop_guard();
                            recovering(
                                "/celestia.fibre.v1.Fibre/UploadShard",
                                inner.upload_shard(&ctx, r.into_inner()),
                            )
                            .await
                        })
                    }),
                };
                self.grpc::<UploadShardRequest, UploadShardResponse>(true)
                    .unary(method, req)
                    .await
            }
            "/celestia.fibre.v1.Fibre/DownloadShard" => {
                let inner = Arc::clone(&self.inner);
                let method = Method::<DownloadShardRequest, DownloadShardResponse> {
                    handler: Arc::new(move |r: tonic::Request<DownloadShardRequest>| {
                        let inner = Arc::clone(&inner);
                        Box::pin(async move {
                            recovering(
                                "/celestia.fibre.v1.Fibre/DownloadShard",
                                inner.download_shard(r.into_inner()),
                            )
                            .await
                        })
                    }),
                };
                self.grpc::<DownloadShardRequest, DownloadShardResponse>(false)
                    .unary(method, req)
                    .await
            }
            "/grpc.health.v1.Health/Check" => {
                let inner = Arc::clone(&self.inner);
                let method = Method::<HealthCheckRequest, HealthCheckResponse> {
                    handler: Arc::new(move |r: tonic::Request<HealthCheckRequest>| {
                        let res = inner
                            .health
                            .check(&r.get_ref().service)
                            .map(|s| HealthCheckResponse { status: s as i32 });
                        Box::pin(async move { res })
                    }),
                };
                self.grpc::<HealthCheckRequest, HealthCheckResponse>(false)
                    .unary(method, req)
                    .await
            }
            "/grpc.health.v1.Health/List" => {
                let inner = Arc::clone(&self.inner);
                let method = Method::<HealthListRequest, HealthListResponse> {
                    handler: Arc::new(move |_r: tonic::Request<HealthListRequest>| {
                        let statuses = inner
                            .health
                            .list()
                            .into_iter()
                            .map(|(k, v)| (k, HealthCheckResponse { status: v as i32 }))
                            .collect();
                        Box::pin(async move { Ok(HealthListResponse { statuses }) })
                    }),
                };
                self.grpc::<HealthListRequest, HealthListResponse>(false)
                    .unary(method, req)
                    .await
            }
            // Go rejects Watch: open streams would hold the graceful stop.
            "/grpc.health.v1.Health/Watch" => {
                Status::unimplemented("Watch is not supported, use Check").into_http()
            }
            other => unknown_method(other).into_http(),
        };
        let mut resp = grpc_transport::response(resp);
        // grpc-go answers with the request's content subtype
        // (`application/grpc+fibre-proto` for Go clients).
        if let Some(ct) = content_type.filter(|ct| ct.as_bytes().starts_with(b"application/grpc")) {
            resp.headers_mut().insert(CONTENT_TYPE, ct);
        } else {
            resp.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/grpc"));
        }
        resp
    }
}

/// grpc-go's answer for an unregistered method.
fn unknown_method(path: &str) -> Status {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let Some((service, method)) = trimmed.rsplit_once('/') else {
        return Status::unimplemented(format!("malformed method name: {}", gostr::quote(path)));
    };
    if service == FIBRE_SERVICE || service == HEALTH_SERVICE {
        Status::unimplemented(format!("unknown method {method} for service {service}"))
    } else {
        Status::unimplemented(format!("unknown service {service}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_methods() {
        assert_eq!(
            unknown_method("/celestia.fibre.v1.Fibre/Nope").message(),
            "unknown method Nope for service celestia.fibre.v1.Fibre"
        );
        assert_eq!(unknown_method("/x.Y/Z").message(), "unknown service x.Y");
    }

    // Go TestRecoverUnaryInterceptor(+Passthrough).
    #[tokio::test]
    #[allow(clippy::panic, clippy::manual_assert)]
    async fn panics_become_internal() {
        let err = recovering("/test/Panic", async {
            if true {
                panic!("boom");
            }
            Ok::<(), Status>(())
        })
        .await
        .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert_eq!(err.message(), "internal error");
        // A panic on the blocking pool resumes on the handler's task.
        let err = recovering("/test/BlockingPanic", async {
            crate::service::blocking_for_test(|| panic!("boom")).await
        })
        .await
        .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert_eq!(
            recovering("/test/Ok", async { Ok::<_, Status>("ok") })
                .await
                .ok(),
            Some("ok")
        );
    }

    /// A recovered panic reaches the client as `Internal`; the handler's
    /// RPC timer must count it the same way, not as a client cancel.
    #[tokio::test]
    #[allow(clippy::panic, clippy::manual_assert)]
    async fn recovered_panics_count_as_internal() {
        use crate::metrics::{Rpc, ServerMetrics};
        let metrics = Arc::new(ServerMetrics::new(Arc::new(
            crate::occupancy::Occupancy::new(0),
        )));
        let m = Arc::clone(&metrics);
        let _ = recovering("/test/Panic", async move {
            let _timer = m.start_rpc(Rpc::Upload);
            if true {
                panic!("boom");
            }
            Ok::<(), Status>(())
        })
        .await;
        let m = Arc::clone(&metrics);
        let _ = recovering("/test/BlockingPanic", async move {
            let _timer = m.start_rpc(Rpc::Upload);
            crate::service::blocking_for_test(|| panic!("boom")).await
        })
        .await;
        assert_eq!(metrics.upload_count(tonic::Code::Internal), 2);
        assert_eq!(metrics.upload_count(tonic::Code::Cancelled), 0);
        assert_eq!(metrics.uploads_in_flight(), 0);
        // A handler dropped without a panic is still a cancel.
        drop(metrics.start_rpc(Rpc::Upload));
        assert_eq!(metrics.upload_count(tonic::Code::Cancelled), 1);
    }

    #[test]
    fn grpc_timeout_header() {
        let parse = |v: &str| {
            let mut h = http::HeaderMap::new();
            h.insert(
                "grpc-timeout",
                HeaderValue::from_str(v).unwrap_or(HeaderValue::from_static("")),
            );
            grpc_timeout(&h)
        };
        assert_eq!(parse("100m"), Some(Duration::from_millis(100)));
        assert_eq!(parse("2S"), Some(Duration::from_secs(2)));
        assert_eq!(parse("1H"), Some(Duration::from_secs(3600)));
        assert_eq!(parse("3M"), Some(Duration::from_secs(180)));
        assert_eq!(parse("5u"), Some(Duration::from_micros(5)));
        assert_eq!(parse("7n"), Some(Duration::from_nanos(7)));
        assert_eq!(parse("7x"), None);
        assert_eq!(parse("m"), None);
        assert_eq!(parse("123456789S"), None, "at most 8 digits");
        assert_eq!(grpc_timeout(&http::HeaderMap::new()), None);
    }

    #[test]
    fn jitter_range() {
        for _ in 0..100 {
            let d = jittered(Duration::from_secs(100));
            assert!(d >= Duration::from_secs(90) && d <= Duration::from_secs(110));
        }
    }
}
