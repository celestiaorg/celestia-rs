//! Ports of Go's in-process server tests (`server_test.go`,
//! `server_upload_test.go`, `server_upload_chain_parity_test.go`,
//! `server_upload_metrics_test.go`, `server_download_test.go`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::result_large_err)]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use celestia_proto::celestia::fibre::v1::{DownloadShardRequest, UploadShardRequest};
use common::*;
use fibre_server::params::{BlobId, Commitment};
use fibre_server::state::StateError;
use fibre_server::{Server, StartError};
use tonic::Code;

type Modify = Box<dyn Fn(&mut UploadShardRequest)>;

fn resigned(f: &Fixture, modify: impl FnOnce(&mut UploadShardRequest)) -> UploadShardRequest {
    let mut req = f.request();
    modify(&mut req);
    if !req.promise.as_ref().unwrap().signature.is_empty() {
        sign_promise(req.promise.as_mut().unwrap());
    }
    req
}

/// Go TestServerRejectsPromisesChainWouldReject: never sign a promise that
/// on-chain ValidateBasic rejects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_promises_the_chain_would_reject() {
    let t = start().await;
    let f = &t.fixture;
    let tx_namespace = {
        let mut ns = vec![0u8; 29];
        ns[28] = 0x01;
        ns
    };
    let parity = vec![0xff; 29];
    let mut tail = vec![0u8; 29];
    tail[0] = 0;
    tail[28] = 0xfe;
    let cases: Vec<(&str, Modify, &str)> = vec![
        (
            "blob version 256",
            Box::new(|r| r.promise.as_mut().unwrap().blob_version = 256),
            "payment promise verification failed: invalid payment promise: invalid blob version: unsupported blob version: 256: invalid request: invalid request",
        ),
        (
            "negative height",
            Box::new(|r| r.promise.as_mut().unwrap().height = -1),
            "payment promise verification failed: invalid payment promise: height must be positive: invalid request",
        ),
        (
            "min int64 height",
            Box::new(|r| r.promise.as_mut().unwrap().height = i64::MIN),
            "payment promise verification failed: invalid payment promise: height must be positive: invalid request",
        ),
        (
            "tx namespace",
            Box::new(move |r| r.promise.as_mut().unwrap().namespace = tx_namespace.clone()),
            "payment promise verification failed: invalid payment promise: invalid blob namespace:",
        ),
        (
            "parity namespace",
            Box::new(move |r| r.promise.as_mut().unwrap().namespace = parity.clone()),
            "payment promise verification failed: invalid payment promise:",
        ),
        (
            "tail pad namespace",
            Box::new(move |r| r.promise.as_mut().unwrap().namespace = tail.clone()),
            "payment promise verification failed: invalid payment promise: invalid blob namespace:",
        ),
    ];
    for (name, modify, msg) in cases {
        let req = resigned(f, |r| modify(r));
        assert!(
            x_fibre::msgs::validate_basic_promise(req.promise.as_ref().unwrap()).is_err(),
            "precondition: chain rejects {name}"
        );
        let err = t.server.upload_shard(req).await.unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument, "{name}");
        assert!(err.message().starts_with(msg), "{name}: {}", err.message());
    }
    assert_eq!(t.chain.verify_calls.load(Ordering::SeqCst), 0);
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Go TestServerLocalMinUploadSize.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_min_upload_size() {
    let t = start_with(|cfg, _| cfg.min_upload_size = 32 << 20).await;
    let req = t.fixture.request();
    let err = t.server.upload_shard(req.clone()).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(
        err.message(),
        format!(
            "payment promise verification failed: upload size {} is below local minimum 33554432 bytes",
            t.fixture.upload_size
        )
    );
    assert_eq!(
        t.chain.verify_calls.load(Ordering::SeqCst),
        0,
        "rejected uploads must not reserve escrow"
    );

    // The inclusive boundary.
    t.server
        .set_min_upload_size(i64::from(req.promise.as_ref().unwrap().blob_size));
    let resp = t.server.upload_shard(req).await.unwrap();
    assert_eq!(resp.validator_signature.len(), 64);
    assert_eq!(t.chain.verify_calls.load(Ordering::SeqCst), 1);

    // Raising the minimum does not block downloads.
    t.server.set_min_upload_size(32 << 20);
    let id = BlobId::new(0, &Commitment(t.fixture.commitment));
    t.server
        .download_shard(DownloadShardRequest {
            blob_id: id.as_bytes().to_vec(),
        })
        .await
        .unwrap();
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Go TestServerUploadShardLastSuccess.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_success_timestamp() {
    let t = start().await;
    let m = t.server.metrics();
    assert_eq!(m.last_upload_success(), None);
    let mut bad = t.fixture.request();
    bad.promise = None;
    t.server.upload_shard(bad.clone()).await.unwrap_err();
    assert_eq!(m.last_upload_success(), None, "not set by a failed upload");
    t.server.upload_shard(t.fixture.request()).await.unwrap();
    let first = m.last_upload_success().unwrap();
    t.server.upload_shard(bad).await.unwrap_err();
    assert_eq!(m.last_upload_success(), Some(first));
    assert!(
        m.render("")
            .contains("fibre_server_upload_shard_last_success_timestamp_seconds")
    );
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assignment_and_state_failures() {
    let t = start().await;
    let f = &t.fixture;

    // The validator set query fails.
    t.chain.answers.lock().unwrap().set = Err(StateError::other("app down"));
    let err = t.server.upload_shard(f.request()).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(
        err.message(),
        "shard assignment verification failed: getting validator set at height 100: app down"
    );

    // This validator is not in the set.
    let mut set = f.set.clone();
    let ours = set.validators.remove(0);
    t.chain.answers.lock().unwrap().set = Ok(set);
    let err = t.server.upload_shard(f.request()).await.unwrap_err();
    assert_eq!(
        err.message(),
        format!(
            "shard assignment verification failed: validator {} not in set at height 100",
            hex::encode_upper(ours.address)
        )
    );
    t.chain.answers.lock().unwrap().set = Ok(f.set.clone());

    // Another validator's row.
    let mut req = f.request();
    let assigned: Vec<u32> = f.shard.rows.iter().map(|r| r.index).collect();
    let other = (0..16384u32).find(|i| !assigned.contains(i)).unwrap();
    req.shard.as_mut().unwrap().rows[0].index = other;
    let err = t.server.upload_shard(req).await.unwrap_err();
    assert_eq!(
        err.message(),
        format!("shard assignment verification failed: row {other} not assigned to validator")
    );

    // The stateful query fails: Go wraps the text.
    let err = t
        .server
        .upload_shard(f.request_at(std::time::SystemTime::now() - Duration::from_secs(4000)))
        .await
        .unwrap_err();
    assert!(
        err.message().starts_with(
            "payment promise verification failed: stateful validation: payment promise expired"
        ),
        "{}",
        err.message()
    );
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Go TestServerDownloadShard: returns rows and RLCs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_returns_rows_and_rlcs() {
    let t = start().await;
    t.server.upload_shard(t.fixture.request()).await.unwrap();
    let id = BlobId::new(0, &Commitment(t.fixture.commitment));
    let shard = t
        .server
        .download_shard(DownloadShardRequest {
            blob_id: id.as_bytes().to_vec(),
        })
        .await
        .unwrap()
        .shard
        .unwrap();
    assert_eq!(shard.rows.len(), t.fixture.shard.rows.len());
    assert!(!shard.rlcs.is_empty());
    t.server.stop(Duration::from_secs(1)).await.unwrap();
}

/// Go TestServerStartFailsWhenStoreCannotOpen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_fails_when_store_cannot_open() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let mut cfg = test_config(dir.path());
    cfg.store = fibre_server::store::StoreConfig::new(&file);
    let server = Server::new(cfg, FakeChain::new(f.set.clone()), FakeSigner::new(&f.seed)).unwrap();
    let err = server.start().await.unwrap_err();
    assert!(err.to_string().starts_with("opening store: "), "{err}");
    assert!(server.store().is_none());
    server.stop(Duration::from_secs(1)).await.unwrap();
    assert!(server.done().is_cancelled());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_errors() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();

    // Invalid config.
    let mut cfg = test_config(dir.path());
    cfg.max_connections = 0;
    let err =
        Server::new(cfg, FakeChain::new(f.set.clone()), FakeSigner::new(&f.seed)).unwrap_err();
    assert_eq!(err.to_string(), "max_connections must be at least 1, got 0");

    // The address is taken.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut cfg = test_config(dir.path());
    cfg.server_listen_address = taken.local_addr().unwrap().to_string();
    let err =
        Server::new(cfg, FakeChain::new(f.set.clone()), FakeSigner::new(&f.seed)).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("opening gRPC listener: listen on 127.0.0.1:"),
        "{err}"
    );

    // The budget cannot be derived: start fails (Go keeps the node down).
    let chain = FakeChain::new(f.set.clone());
    chain.answers.lock().unwrap().budget = Ok(0);
    let server = Server::new(test_config(dir.path()), chain, FakeSigner::new(&f.seed)).unwrap();
    let err = server.start().await.unwrap_err();
    assert_eq!(
        err,
        StartError::Other(
            "deriving initial storage budget: full-stake storage budget must be positive (got 0); pass --unlimited-budget to disable the limiter".into()
        )
    );
    server.stop(Duration::from_secs(1)).await.unwrap();

    // Unlimited budget skips the query.
    let chain = FakeChain::new(f.set.clone());
    chain.answers.lock().unwrap().budget = Err(StateError::other("no module"));
    let mut cfg = test_config(dir.path());
    cfg.unlimited_budget = true;
    let server = Server::new(cfg, chain, FakeSigner::new(&f.seed)).unwrap();
    server.start().await.unwrap();
    assert_eq!(server.occupancy().budget_bytes(), 0);
    assert!(server.start().await.is_err(), "start twice");
    server.stop(Duration::from_secs(1)).await.unwrap();
    server.stop(Duration::from_secs(1)).await.unwrap();

    // Not in the active set: no budget yet (warned), uploads unlimited.
    let mut set = f.set.clone();
    set.validators.remove(0);
    let chain = FakeChain::new(set);
    let server = Server::new(test_config(dir.path()), chain, FakeSigner::new(&f.seed)).unwrap();
    server.start().await.unwrap();
    assert_eq!(server.occupancy().budget_bytes(), 0);
    server.stop(Duration::from_secs(1)).await.unwrap();
}

/// The signer fails: Internal, and the store keeps the shard (store
/// before sign).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_failure_is_internal_after_store() {
    struct Broken(Arc<FakeSigner>);
    impl fibre_server::Signer for Broken {
        fn public_key(
            &self,
        ) -> fibre_server::BoxFuture<'_, Result<[u8; 32], fibre_server::SignerError>> {
            self.0.public_key()
        }
        fn sign_raw_bytes<'a>(
            &'a self,
            chain_id: &'a str,
            unique_id: &'a str,
            raw: &'a [u8],
        ) -> fibre_server::BoxFuture<'a, Result<Vec<u8>, fibre_server::SignerError>> {
            if unique_id == "fibre/pp:v0" {
                Box::pin(async { Ok(vec![1, 2, 3]) })
            } else {
                self.0.sign_raw_bytes(chain_id, unique_id, raw)
            }
        }
    }
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let server = Server::new(
        test_config(dir.path()),
        FakeChain::new(f.set.clone()),
        Arc::new(Broken(FakeSigner::new(&f.seed))),
    )
    .unwrap();
    server.start().await.unwrap();
    let err = server.upload_shard(f.request()).await.unwrap_err();
    assert_eq!(err.code(), Code::Internal);
    assert_eq!(
        err.message(),
        "failed to sign payment promise: invalid signature length: expected 64, got 3"
    );
    assert_eq!(server.metrics().store_puts(), 1);
    server.stop(Duration::from_secs(1)).await.unwrap();
}
