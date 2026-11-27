//! The in-flight upload memory budget: uploads are admitted by the length
//! their gRPC frame declares, before tonic buffers the body, so peers that
//! announce large messages cannot make the process hold more than
//! `max_inflight_upload_bytes`. Small requests (health, download) are not
//! affected.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_possible_wrap)]

mod common;

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use common::*;
use fibre_server::metrics::RejectReason;
use fibre_server::proto::{HealthCheckRequest, HealthCheckResponse, ServingStatus};
use http_body::Frame;
use tokio::sync::mpsc;

/// A request body fed by a channel: the test sends the gRPC prefix and
/// then holds the stream open without sending the message.
struct ChanBody(mpsc::Receiver<Bytes>);

impl http_body::Body for ChanBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        self.0.poll_recv(cx).map(|b| b.map(|b| Ok(Frame::data(b))))
    }
}

/// An HTTP/2 connection to the server over its TLS identity.
async fn h2(t: &TestServer) -> hyper::client::conn::http2::SendRequest<ChanBody> {
    let tls = tls_connect(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let (send, conn) = hyper::client::conn::http2::handshake(
        hyper_util::rt::TokioExecutor::new(),
        hyper_util::rt::TokioIo::new(tls),
    )
    .await
    .unwrap();
    tokio::spawn(conn);
    send
}

/// Starts an `UploadShard` call that declares a `len`-byte message and
/// sends none of it. Returns the body sender (drop it to end the call) and
/// the response future.
async fn declare_upload(
    send: &mut hyper::client::conn::http2::SendRequest<ChanBody>,
    len: u32,
) -> (
    mpsc::Sender<Bytes>,
    tokio::task::JoinHandle<http::Response<hyper::body::Incoming>>,
) {
    let (tx, rx) = mpsc::channel(4);
    let mut prefix = vec![0u8];
    prefix.extend_from_slice(&len.to_be_bytes());
    tx.send(Bytes::from(prefix)).await.unwrap();
    let req = http::Request::post("http://fibre.invalid/celestia.fibre.v1.Fibre/UploadShard")
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(ChanBody(rx))
        .unwrap();
    send.ready().await.unwrap();
    let fut = send.send_request(req);
    (tx, tokio::spawn(async move { fut.await.unwrap() }))
}

fn grpc_status(resp: &http::Response<hyper::body::Incoming>) -> Option<(i32, String)> {
    let code = resp
        .headers()
        .get("grpc-status")?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    let msg = resp
        .headers()
        .get("grpc-message")
        .map(|m| m.to_str().unwrap_or_default().to_owned())
        .unwrap_or_default();
    Some((code, msg))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declared_upload_sizes_are_admitted_against_the_memory_budget() {
    let max = fibre_server::params::DEFAULT_PROTOCOL_PARAMS.max_message_size();
    // Room for two maximum uploads (each reserves twice its size: the
    // receive buffer and the decoded copy).
    let t = start_with(|cfg, _| {
        cfg.max_inflight_upload_bytes = i64::try_from(4 * max).unwrap();
    })
    .await;
    let len = u32::try_from(max).unwrap();
    let mut send = h2(&t).await;

    let (held_a, call_a) = declare_upload(&mut send, len).await;
    let (_held_b, _call_b) = declare_upload(&mut send, len).await;
    // Both are admitted and wait for their bodies.
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().upload_memory_reserved() == 4 * max as i64
        })
        .await
    );

    // A third is refused at once, before any body byte is buffered.
    let (_held_c, call_c) = declare_upload(&mut send, len).await;
    let resp = tokio::time::timeout(Duration::from_secs(5), call_c)
        .await
        .expect("refused promptly")
        .unwrap();
    let (code, msg) = grpc_status(&resp).expect("trailers-only status");
    assert_eq!(code, tonic::Code::ResourceExhausted as i32, "{msg}");
    assert!(msg.contains("max_inflight_upload_bytes"), "{msg}");
    assert_eq!(t.server.metrics().rejected(RejectReason::InflightMemory), 1);
    assert_eq!(
        t.server
            .metrics()
            .upload_count(tonic::Code::ResourceExhausted),
        1
    );

    // Small requests are unaffected while the budget is full.
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let h: HealthCheckResponse = unary(
        &ch,
        HEALTH_CHECK,
        HealthCheckRequest {
            service: "fibre-liveness".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(h.status, ServingStatus::Serving as i32);
    let missing = unary::<_, celestia_proto::celestia::fibre::v1::DownloadShardResponse>(
        &ch,
        DOWNLOAD,
        celestia_proto::celestia::fibre::v1::DownloadShardRequest {
            blob_id: vec![0; 33],
        },
    )
    .await
    .unwrap_err();
    assert_ne!(missing.code(), tonic::Code::ResourceExhausted);

    // Ending a held call returns its reservation; a new upload fits again.
    drop(held_a);
    let _ = tokio::time::timeout(Duration::from_secs(5), call_a).await;
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().upload_memory_reserved() == 2 * max as i64
        })
        .await
    );
    let (_held_d, _call_d) = declare_upload(&mut send, len).await;
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().upload_memory_reserved() == 4 * max as i64
        })
        .await
    );
    assert_eq!(t.server.metrics().rejected(RejectReason::InflightMemory), 1);
}

#[test]
fn the_budget_must_fit_one_maximum_upload() {
    let max = fibre_server::params::DEFAULT_PROTOCOL_PARAMS.max_message_size();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = test_config(dir.path());
    assert_eq!(cfg.max_inflight_upload_bytes, 2 << 30);
    cfg.max_inflight_upload_bytes = i64::try_from(2 * max).unwrap() - 1;
    assert_eq!(
        cfg.validate().unwrap_err().0,
        format!(
            "max_inflight_upload_bytes must be at least {} (twice the maximum message size), got {}",
            2 * max,
            2 * max - 1
        )
    );
    cfg.max_inflight_upload_bytes = i64::try_from(2 * max).unwrap();
    cfg.validate().unwrap();
}
