//! End to end over TLS: a tonic client that verifies the server's tlsid
//! certificate talks to a server on 127.0.0.1:0 with a fake chain and the
//! Go-generated shard of validator 0.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::result_large_err)]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use celestia_proto::celestia::fibre::v1::{
    DownloadShardRequest, DownloadShardResponse, UploadShardRequest, UploadShardResponse,
};
use common::*;
use fibre_server::metrics::RejectReason;
use fibre_server::params::{BlobId, Commitment};
use fibre_server::proto::{
    HealthCheckRequest, HealthCheckResponse, RETRY_INFO_TYPE_URL, RetryInfo, RpcStatus,
    ServingStatus,
};
use fibre_server::store::shard_binary_size;
use prost::Message;
use tokio::sync::Semaphore;
use tonic::Code;

fn blob_id(f: &Fixture, version: u8) -> Vec<u8> {
    BlobId::new(version, &Commitment(f.commitment))
        .as_bytes()
        .to_vec()
}

async fn upload(
    ch: &tonic::transport::Channel,
    req: UploadShardRequest,
) -> Result<UploadShardResponse, tonic::Status> {
    unary(ch, UPLOAD, req).await
}

async fn download(
    ch: &tonic::transport::Channel,
    blob_id: Vec<u8>,
) -> Result<DownloadShardResponse, tonic::Status> {
    unary(ch, DOWNLOAD, DownloadShardRequest { blob_id }).await
}

async fn health(
    ch: &tonic::transport::Channel,
    service: &str,
) -> Result<ServingStatus, tonic::Status> {
    let r: HealthCheckResponse = unary(
        ch,
        HEALTH_CHECK,
        HealthCheckRequest {
            service: service.into(),
        },
    )
    .await?;
    Ok(ServingStatus::try_from(r.status).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_sign_download_roundtrip() {
    let t = start().await;
    let addr = t.server.listen_address();
    let ch = channel(addr, t.fixture.validator_key()).await.unwrap();

    let req = t.fixture.request();
    let promise = req.promise.clone().unwrap();
    let resp = upload(&ch, req.clone()).await.unwrap();

    // The validator signature verifies over the promise's sign bytes.
    assert_eq!(resp.validator_signature.len(), 64);
    let sign_bytes = x_fibre::promise::sign_bytes(&promise).unwrap();
    assert!(crypto::ed25519::verify(
        &t.fixture.validator_key(),
        &sign_bytes,
        &resp.validator_signature.clone().try_into().unwrap()
    ));

    let size = shard_binary_size(&t.fixture.shard);
    assert_eq!(t.server.occupancy().usage(), size);

    // Download it back.
    let got = download(&ch, blob_id(&t.fixture, 0)).await.unwrap();
    assert_eq!(got.shard.unwrap(), t.fixture.shard);

    // A duplicate upload is signed again without a second reservation.
    let again = upload(&ch, req).await.unwrap();
    assert_eq!(again.validator_signature, resp.validator_signature);
    assert_eq!(t.server.occupancy().usage(), size);

    let m = t.server.metrics();
    assert_eq!(m.upload_count(Code::Ok), 2);
    assert_eq!(m.download_count(Code::Ok), 1);
    assert_eq!(m.store_puts(), 1);
    assert!(m.last_upload_success().is_some());
    let text = m.render("");
    assert!(
        text.contains("fibre_server_download_shard_bytes_total{} 18944"),
        "{text}"
    );

    t.server.stop(Duration::from_secs(5)).await.unwrap();
    assert!(t.server.done().is_cancelled());
    assert_eq!(t.server.serve_error(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_identity_is_checked_by_the_client() {
    let t = start().await;
    let addr = t.server.listen_address();
    // Another validator's key: the client must refuse the certificate.
    let other = t.fixture.set.validators[1].pub_key;
    let err = tls_connect(addr, other).await.unwrap_err();
    assert!(err.to_string().contains("signature"), "{err}");
    // TLS 1.2 only clients are refused by the server.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS12]);
    assert!(
        cfg.is_err() || {
            let cfg = cfg
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(TlsIdVerifier {
                    key: t.fixture.validator_key(),
                    chain_id: CHAIN_ID.into(),
                    provider,
                }))
                .with_no_client_auth();
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            tokio_rustls::TlsConnector::from(Arc::new(cfg))
                .connect(rustls::pki_types::ServerName::try_from("x").unwrap(), tcp)
                .await
                .is_err()
        }
    );
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Every rejection class of UploadShard with Go's code and message.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn upload_rejections() {
    let t = start().await;
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let f = &t.fixture;

    let expect = async |req: UploadShardRequest, code: Code, msg: &str| {
        let err = upload(&ch, req).await.unwrap_err();
        assert_eq!(err.code(), code, "{}", err.message());
        assert_eq!(err.message(), msg);
    };
    let expect_prefix = async |req: UploadShardRequest, code: Code, prefix: &str| {
        let err = upload(&ch, req).await.unwrap_err();
        assert_eq!(err.code(), code, "{}", err.message());
        assert!(err.message().starts_with(prefix), "{}", err.message());
    };
    let resigned = |modify: &dyn Fn(&mut UploadShardRequest)| {
        let mut req = f.request();
        modify(&mut req);
        sign_promise(req.promise.as_mut().unwrap());
        req
    };

    // Nil promise.
    expect(
        UploadShardRequest {
            promise: None,
            shard: Some(f.shard.clone()),
        },
        Code::InvalidArgument,
        "payment promise verification failed: nil payment promise",
    )
    .await;
    // ValidateBasic.
    let mut req = f.request();
    req.promise.as_mut().unwrap().signature.clear();
    expect(
        req,
        Code::InvalidArgument,
        "payment promise verification failed: invalid payment promise: signature cannot be empty: invalid request",
    )
    .await;
    // Chain ID.
    expect(
        resigned(&|r| r.promise.as_mut().unwrap().chain_id = "wrong-chain".into()),
        Code::InvalidArgument,
        "payment promise verification failed: payment promise chain ID mismatch: expected celestia, got wrong-chain",
    )
    .await;
    // Bad escrow signature.
    let mut req = f.request();
    req.promise.as_mut().unwrap().signature[5] ^= 1;
    expect(
        req,
        Code::InvalidArgument,
        "payment promise verification failed: payment promise validation failed: signature verification failed",
    )
    .await;
    // Expired (stateful).
    expect_prefix(
        f.request_at(SystemTime::now() - Duration::from_secs(7200)),
        Code::InvalidArgument,
        "payment promise verification failed: stateful validation: payment promise expired",
    )
    .await;
    // Missing shard.
    let mut req = f.request();
    req.shard = None;
    expect(req, Code::InvalidArgument, "shard is required").await;
    // Assignment: a duplicated row.
    let mut req = f.request();
    let first = req.shard.as_ref().unwrap().rows[0].clone();
    req.shard.as_mut().unwrap().rows[1] = first.clone();
    expect(
        req,
        Code::InvalidArgument,
        &format!(
            "shard assignment verification failed: duplicate row {}",
            first.index
        ),
    )
    .await;
    // Assignment: a missing row.
    let mut req = f.request();
    req.shard.as_mut().unwrap().rows.pop();
    expect(
        req,
        Code::InvalidArgument,
        &format!(
            "shard assignment verification failed: expected {} rows, got {}",
            f.shard.rows.len(),
            f.shard.rows.len() - 1
        ),
    )
    .await;
    // Shard verification: tampered row data.
    let mut req = f.request();
    req.shard.as_mut().unwrap().rows[9].data[0] ^= 0xff;
    expect_prefix(
        req,
        Code::InvalidArgument,
        "shard verification failed: shard row verification failed: verifying row proofs:",
    )
    .await;
    // Upload size inconsistent with rows.
    expect(
        resigned(&|r| r.promise.as_mut().unwrap().blob_size += 1),
        Code::InvalidArgument,
        &format!(
            "shard verification failed: upload size mismatch: promise has {}, but row size 128 * 4096 original rows = {}",
            f.upload_size + 1,
            f.upload_size
        ),
    )
    .await;
    // Oversized RLCs.
    let mut req = f.request();
    let n = req.shard.as_ref().unwrap().rlcs.len();
    req.shard.as_mut().unwrap().rlcs = vec![0; 2 * n];
    expect(
        req,
        Code::InvalidArgument,
        &format!(
            "shard verification failed: expected {n} RLC bytes, got {}",
            2 * n
        ),
    )
    .await;
    // Codec: too many rows (rejected before decoding allocates).
    let mut req = f.request();
    let row = req.shard.as_ref().unwrap().rows[0].clone();
    req.shard.as_mut().unwrap().rows = vec![
        celestia_proto::celestia::fibre::v1::BlobRow {
            index: 0,
            data: vec![],
            proof: vec![],
        };
        4097
    ];
    expect(
        req,
        Code::Internal,
        "grpc: error unmarshalling request: fibre-proto codec: shard exceeds 4096 rows",
    )
    .await;
    // Codec: too many proof segments.
    let mut req = f.request();
    req.shard.as_mut().unwrap().rows[3].proof = vec![row.proof[0].clone(); 15];
    expect(
        req,
        Code::Internal,
        "grpc: error unmarshalling request: fibre-proto codec: row exceeds 14 proof segments",
    )
    .await;

    let m = t.server.metrics();
    assert_eq!(m.rejected(RejectReason::CodecLimit), 2);
    assert_eq!(m.rejected(RejectReason::PromiseInvalid), 5);
    assert_eq!(m.rejected(RejectReason::ShardMissing), 1);
    assert_eq!(m.rejected(RejectReason::AssignmentInvalid), 2);
    assert_eq!(m.rejected(RejectReason::ShardInvalid), 3);
    assert_eq!(m.upload_count(Code::InvalidArgument), 11);
    assert_eq!(t.server.occupancy().usage(), 0);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_rejections_and_transport_errors() {
    let t = start().await;
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let f = &t.fixture;

    let err = download(&ch, vec![1, 2, 3]).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(
        err.message(),
        "invalid blob ID: blob ID must be 33 bytes, got 3"
    );

    let err = download(&ch, blob_id(f, 99)).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(
        err.message(),
        "unsupported blob version: unsupported blob version: 99"
    );

    let err = download(&ch, blob_id(f, 0)).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_eq!(
        err.message(),
        format!(
            "no blob shard found for commitment {}",
            hex::encode(f.commitment)
        )
    );

    // Go TestDownloadShardOversizedRequestRejected.
    let err = download(&ch, vec![0; 1 << 20]).await.unwrap_err();
    assert_eq!(err.code(), Code::Internal);
    assert_eq!(
        err.message(),
        "grpc: error unmarshalling request: fibre-proto codec: download request exceeds 1024 bytes"
    );

    // Unknown method and service.
    let err = unary::<_, DownloadShardResponse>(
        &ch,
        "/celestia.fibre.v1.Fibre/Nope",
        DownloadShardRequest::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
    assert_eq!(
        err.message(),
        "unknown method Nope for service celestia.fibre.v1.Fibre"
    );

    // Message over the maximum size: grpc-go's ResourceExhausted text.
    let max = t.server.config().max_message_size;
    let mut req = f.request();
    req.shard.as_mut().unwrap().rlcs = vec![0; max];
    let len = req.encoded_len();
    let err = upload(&ch, req).await.unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    assert_eq!(
        err.message(),
        format!("grpc: received message larger than max ({len} vs. {max})")
    );

    // The connection still serves.
    let err = download(&ch, blob_id(f, 0)).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Go TestServerUploadShardBudgetExceeded / TestClientServerStorageBudget.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_limit_rejects_with_retry_info() {
    // Full-stake budget = OriginalRows bytes: the node's budget is its
    // assigned row count in bytes, far below one shard.
    let t = start_with(|_, chain| chain.answers.lock().unwrap().budget = Ok(4096)).await;
    let rows = i64::try_from(t.fixture.shard.rows.len()).unwrap();
    assert_eq!(t.server.occupancy().budget_bytes(), rows);
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let err = upload(&ch, t.fixture.request()).await.unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    assert_eq!(err.message(), "fibre storage budget exceeded");
    let details = RpcStatus::decode(err.details()).unwrap();
    assert_eq!(details.code, Code::ResourceExhausted as i32);
    let any = &details.details[0];
    assert_eq!(any.type_url, RETRY_INFO_TYPE_URL);
    let delay = RetryInfo::decode(any.value.as_ref())
        .unwrap()
        .retry_delay
        .unwrap();
    assert!((60..90).contains(&delay.seconds), "{delay:?}");
    assert_eq!(t.server.occupancy().usage(), 0, "nothing reserved");
    assert_eq!(t.server.metrics().rejected(RejectReason::BudgetExceeded), 1);
    // Nothing stored.
    let err = download(&ch, blob_id(&t.fixture, 0)).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    t.server.stop(Duration::from_secs(1)).await.unwrap();

    // A budget that fits exactly one shard accepts it.
    let size = shard_binary_size(&t.fixture.shard);
    let t = start_with(|_, chain| {
        // budget = full * rows / 4096 = size  =>  full = size * 4096 / rows
        chain.answers.lock().unwrap().budget = Ok(size * 4096 / rows + 4096);
    })
    .await;
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    assert!(t.server.occupancy().budget_bytes() >= size);
    upload(&ch, t.fixture.request()).await.unwrap();
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_service_transitions() {
    let t = start().await;
    let addr = t.server.listen_address();
    let ch = channel(addr, t.fixture.validator_key()).await.unwrap();

    assert_eq!(
        health(&ch, "fibre-liveness").await.unwrap(),
        ServingStatus::Serving
    );
    assert!(
        eventually(Duration::from_secs(10), async || {
            health(&ch, "").await.ok() == Some(ServingStatus::Serving)
        })
        .await
    );
    assert_eq!(
        health(&ch, "celestia.fibre.v1.Fibre").await.unwrap(),
        ServingStatus::Serving
    );
    let err = health(&ch, "other").await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_eq!(err.message(), "unknown service");

    // Watch is rejected so it cannot hold the drain.
    let err = unary::<_, HealthCheckResponse>(&ch, HEALTH_WATCH, HealthCheckRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
    assert_eq!(err.message(), "Watch is not supported, use Check");

    // A failing check flips readiness; recovery flips it back.
    t.chain.answers.lock().unwrap().registration.found = false;
    assert!(
        eventually(Duration::from_secs(10), async || {
            health(&ch, "").await.ok() == Some(ServingStatus::NotServing)
        })
        .await
    );
    assert_eq!(t.server.readiness().1, vec!["registration"]);
    assert_eq!(
        t.server.health_result("registration").unwrap().reason,
        "provider_not_registered"
    );
    assert_eq!(
        health(&ch, "fibre-liveness").await.unwrap(),
        ServingStatus::Serving,
        "liveness is independent of the checks"
    );
    t.chain.answers.lock().unwrap().registration.found = true;
    assert!(
        eventually(Duration::from_secs(10), async || {
            health(&ch, "").await.ok() == Some(ServingStatus::Serving)
        })
        .await
    );
    assert!(t.server.metrics().ready());

    t.server.stop(Duration::from_secs(5)).await.unwrap();
    assert_eq!(
        t.server.health_check("").unwrap(),
        ServingStatus::NotServing
    );
    // The listener is closed.
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

/// Go TestListenCapsConnectionsAtConfiguredLimit, over TLS.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_cap() {
    let t = start_with(|cfg, _| cfg.max_connections = 2).await;
    let addr = t.server.listen_address();
    let key = t.fixture.validator_key();
    let a = tls_connect(addr, key).await.unwrap();
    let _b = tls_connect(addr, key).await.unwrap();
    // The third connects at TCP level but is not accepted.
    let third = tokio::spawn(tls_connect(addr, key));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!third.is_finished(), "accepted beyond max_connections");
    assert_eq!(t.server.metrics().connections_active(), 2);
    drop(a);
    let c = tokio::time::timeout(Duration::from_secs(5), third)
        .await
        .expect("accepted once a slot freed")
        .unwrap();
    c.unwrap();
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// max_concurrent_streams bounds the requests one connection runs at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_cap() {
    let t = start_with(|cfg, _| cfg.max_concurrent_streams = 2).await;
    let gate = Arc::new(Semaphore::new(0));
    *t.chain.verify_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let calls: Vec<_> = (0..3u64)
        .map(|i| {
            let ch = ch.clone();
            let req = t
                .fixture
                .request_at(SystemTime::now() - Duration::from_millis(i));
            tokio::spawn(async move { upload(&ch, req).await })
        })
        .collect();
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.chain.verify_inflight.load(Ordering::SeqCst) == 2
        })
        .await
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        t.chain.verify_inflight.load(Ordering::SeqCst),
        2,
        "the third stream waits for a slot"
    );
    gate.add_permits(3);
    for c in calls {
        c.await.unwrap().unwrap();
    }
    assert_eq!(t.chain.verify_max_inflight.load(Ordering::SeqCst), 2);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_shards_are_pruned() {
    let t = start_with(|cfg, chain| {
        cfg.tuning.prune_interval = Duration::from_millis(200);
        // Expired an hour ago, no retention: due at the first prune pass.
        chain.answers.lock().unwrap().expires_at =
            Some(SystemTime::now() - Duration::from_secs(3600));
    })
    .await;
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    // Created 2 h ago so the prune time (max(expiry, creation)) is past.
    t.chain.answers.lock().unwrap().promise_timeout = Duration::from_secs(4 * 3600);
    upload(
        &ch,
        t.fixture
            .request_at(SystemTime::now() - Duration::from_secs(7200)),
    )
    .await
    .unwrap();
    assert!(t.server.occupancy().usage() > 0);
    assert!(
        eventually(Duration::from_secs(10), async || {
            t.server.metrics().pruned_entries() == 1
        })
        .await
    );
    assert_eq!(t.server.occupancy().usage(), 0);
    let err = download(&ch, blob_id(&t.fixture, 0)).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Stop drains an in-flight upload before closing the store.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_stop_drains_in_flight_requests() {
    let t = start().await;
    let gate = Arc::new(Semaphore::new(0));
    *t.chain.verify_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let req = t.fixture.request();
    let call = tokio::spawn(async move { upload(&ch, req).await });
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.chain.verify_inflight.load(Ordering::SeqCst) == 1
        })
        .await
    );
    let server = Arc::new(t.server);
    let stopping = {
        let server = Arc::clone(&server);
        tokio::spawn(async move { server.stop(Duration::from_secs(10)).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        server.health_check("").unwrap(),
        ServingStatus::NotServing,
        "NOT_SERVING is published before draining"
    );
    assert!(!stopping.is_finished(), "stop waits for the upload");
    gate.add_permits(1);
    call.await.unwrap().unwrap();
    stopping.await.unwrap().unwrap();
}

/// A forced stop drops requests that outlive the drain timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_forces_after_drain_timeout() {
    let t = start().await;
    *t.chain.verify_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let req = t.fixture.request();
    let call = tokio::spawn(async move { upload(&ch, req).await });
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.chain.verify_inflight.load(Ordering::SeqCst) == 1
        })
        .await
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        t.server.stop(Duration::from_millis(100)),
    )
    .await
    .expect("stop returns after the drain timeout")
    .unwrap();
    assert!(call.await.unwrap().is_err());
}

/// Connections idle for max_connection_idle receive a GOAWAY; a new
/// channel request reconnects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_and_aged_connections_are_closed() {
    let t = start_with(|cfg, _| {
        cfg.tuning.max_connection_idle = Duration::from_millis(200);
        cfg.tuning.max_connection_age = Duration::from_millis(600);
        cfg.tuning.max_connection_age_grace = Duration::from_millis(200);
    })
    .await;
    let addr = t.server.listen_address();
    let ch = channel(addr, t.fixture.validator_key()).await.unwrap();
    health(&ch, "").await.unwrap();
    assert_eq!(t.server.metrics().connections_active(), 1);
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().connections_active() == 0
        })
        .await,
        "idle connection closed"
    );

    // A connection that stays busy is still closed once aged plus grace.
    let gate = Arc::new(Semaphore::new(0));
    *t.chain.verify_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let ch = channel(addr, t.fixture.validator_key()).await.unwrap();
    let req = t.fixture.request();
    let call = tokio::spawn(async move { upload(&ch, req).await });
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().connections_active() == 1
        })
        .await
    );
    assert!(
        eventually(Duration::from_secs(5), async || {
            t.server.metrics().connections_active() == 0
        })
        .await,
        "aged connection closed after the grace period"
    );
    assert!(call.await.unwrap().is_err());
    gate.add_permits(10);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// A client deadline (grpc-timeout) while waiting for a duplicate upload.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_uploads_store_once_and_deadlines_apply() {
    let t = start().await;
    let gate = Arc::new(Semaphore::new(0));
    *t.signer.sign_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let ch = channel(t.server.listen_address(), t.fixture.validator_key())
        .await
        .unwrap();
    let req = t.fixture.request();
    let first = {
        let (ch, req) = (ch.clone(), req.clone());
        tokio::spawn(async move { upload(&ch, req).await })
    };
    // The first owner holds the promise while it signs.
    assert!(
        eventually(Duration::from_secs(5), async || t
            .server
            .metrics()
            .store_puts()
            == 1)
        .await
    );
    // A duplicate with a short deadline gives up waiting.
    let ctx = fibre_server::CallContext::with_timeout(Duration::from_millis(100));
    let err = t
        .server
        .upload_shard_with(&ctx, req.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);
    assert_eq!(
        err.message(),
        "waiting for existing upload: context deadline exceeded"
    );
    // Over gRPC: the server honours the client's grpc-timeout header; the
    // tonic client enforces the same deadline itself, so either side may
    // report it first.
    let mut timed = tonic::Request::new(req.clone());
    timed.set_timeout(Duration::from_millis(100));
    let err = unary_with::<_, UploadShardResponse>(&ch, UPLOAD, timed)
        .await
        .unwrap_err();
    assert!(
        err.code() == Code::DeadlineExceeded || err.message() == "Timeout expired",
        "{err:?}"
    );

    // A cancelled duplicate.
    let ctx = fibre_server::CallContext::new();
    ctx.cancel();
    let err = t
        .server
        .upload_shard_with(&ctx, req.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Cancelled);
    assert_eq!(
        err.message(),
        "waiting for existing upload: context canceled"
    );

    let second = {
        let (ch, req) = (ch.clone(), req.clone());
        tokio::spawn(async move { upload(&ch, req).await })
    };
    gate.add_permits(10);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(t.server.metrics().store_puts(), 1, "stored once");
    assert_eq!(
        t.server.occupancy().usage(),
        shard_binary_size(&t.fixture.shard)
    );
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}
