//! Test harness: a fake chain and signer around the Go-generated shard of
//! `conformance/fixtures/fibre-server/shard-verify.json.zst` (validator 0
//! of 100 equal validators whose keys are `seed = [0x70 + i; 32]`), and a
//! tonic client that verifies the server certificate with `tlsid`.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::result_large_err
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use celestia_proto::celestia::fibre::v1::{BlobRow, BlobShard, PaymentPromise, UploadShardRequest};
use fibre_server::state::{
    BoxFuture, ChainState, NodeStatus, ProviderRegistration, Signer, SignerError, StateError,
    Validator, ValidatorSet, VerifiedPromise,
};
use fibre_server::tlsid::{self, Ed25519ConsensusKey, Ed25519RawBytesSigner};
use fibre_server::{Server, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::Deserialize;
use tokio::sync::Semaphore;

pub const CHAIN_ID: &str = "celestia";
pub const HEIGHT: i64 = 100;

#[derive(Deserialize)]
struct File {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct RowJson {
    index: u32,
    data: String,
    #[serde(default)]
    proof: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ShardJson {
    rows: Vec<RowJson>,
    rlcs: String,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    commitment: String,
    upload_size: u32,
    validator_index: i64,
    #[serde(default)]
    voting_powers: Option<Vec<i64>>,
    shard: ShardJson,
}

/// The Go-generated blob shard and its validator set.
#[derive(Clone)]
pub struct Fixture {
    pub commitment: [u8; 32],
    pub upload_size: u32,
    pub shard: BlobShard,
    /// The validator set in CometBFT order.
    pub set: ValidatorSet,
    /// Seed of the validator the shard belongs to (set index 0).
    pub seed: [u8; 32],
}

fn row(r: &RowJson) -> BlobRow {
    BlobRow {
        index: r.index,
        data: hex::decode(&r.data).unwrap(),
        proof: r
            .proof
            .iter()
            .flatten()
            .map(|p| hex::decode(p).unwrap())
            .collect(),
    }
}

pub fn fixture() -> Fixture {
    let raw = std::fs::read(format!(
        "{}/../../../conformance/fixtures/fibre-server/shard-verify.json.zst",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let f: File = serde_json::from_slice(&zstd::decode_all(raw.as_slice()).unwrap()).unwrap();
    let c = f
        .cases
        .iter()
        .find(|c| c.name == "default_shape_validator0")
        .unwrap();
    assert_eq!(c.validator_index, 0);
    let powers = c.voting_powers.clone().unwrap();
    // conformance/fibre-server validatorSet(powers, 0x70): CometBFT order
    // (power descending, address ascending).
    let mut vals: Vec<(Validator, [u8; 32])> = powers
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let seed = [0x70u8 + u8::try_from(i).unwrap(); 32];
            let pk = Ed25519RawBytesSigner::from_seed(&seed).public_key();
            (Validator::new(pk, p), seed)
        })
        .collect();
    vals.sort_by(|a, b| {
        b.0.voting_power
            .cmp(&a.0.voting_power)
            .then(a.0.address.cmp(&b.0.address))
    });
    let seed = vals[0].1;
    Fixture {
        commitment: hex::decode(&c.commitment).unwrap().try_into().unwrap(),
        upload_size: c.upload_size,
        shard: BlobShard {
            rows: c.shard.rows.iter().map(row).collect(),
            rlcs: hex::decode(&c.shard.rlcs).unwrap(),
        },
        set: ValidatorSet {
            validators: vals.into_iter().map(|(v, _)| v).collect(),
            height: u64::try_from(HEIGHT).unwrap(),
        },
        seed,
    }
}

/// The escrow key the promises are signed with.
pub fn escrow_key() -> [u8; 32] {
    crypto::secp256k1::secret_key_from_secret(b"fibre-server e2e escrow")
}

/// A v0 blob namespace.
pub fn namespace() -> Vec<u8> {
    let mut ns = vec![0u8; 19];
    ns.extend_from_slice(b"fibre-test");
    ns
}

pub fn timestamp(t: SystemTime) -> pbjson_types::Timestamp {
    let d = t.duration_since(UNIX_EPOCH).unwrap();
    pbjson_types::Timestamp {
        seconds: i64::try_from(d.as_secs()).unwrap(),
        nanos: i32::try_from(d.subsec_nanos()).unwrap(),
    }
}

/// Signs `p` with the escrow key.
pub fn sign_promise(p: &mut PaymentPromise) {
    let sb = x_fibre::promise::sign_bytes(p).unwrap();
    p.signature = crypto::secp256k1::sign(&escrow_key(), &sb)
        .unwrap()
        .to_vec();
}

impl Fixture {
    /// A valid, signed promise for the fixture blob, created at `created`.
    pub fn promise_at(&self, created: SystemTime) -> PaymentPromise {
        let sk = escrow_key();
        let mut p = PaymentPromise {
            chain_id: CHAIN_ID.into(),
            height: HEIGHT,
            namespace: namespace(),
            blob_size: self.upload_size,
            blob_version: 0,
            commitment: self.commitment.to_vec(),
            creation_timestamp: Some(timestamp(created)),
            signer_public_key: Some(celestia_proto::cosmos::crypto::secp256k1::PubKey {
                key: crypto::secp256k1::public_key_from_secret_key(&sk)
                    .unwrap()
                    .to_vec(),
            }),
            signature: vec![],
        };
        sign_promise(&mut p);
        p
    }

    pub fn request(&self) -> UploadShardRequest {
        self.request_at(SystemTime::now())
    }

    pub fn request_at(&self, created: SystemTime) -> UploadShardRequest {
        UploadShardRequest {
            promise: Some(self.promise_at(created)),
            shard: Some(self.shard.clone()),
        }
    }

    pub fn validator_key(&self) -> [u8; 32] {
        Ed25519RawBytesSigner::from_seed(&self.seed).public_key()
    }
}

/// What the fake chain answers; tests change it under the lock.
pub struct ChainAnswers {
    pub set: Result<ValidatorSet, StateError>,
    pub budget: Result<i64, StateError>,
    pub status: Result<NodeStatus, StateError>,
    pub registration: ProviderRegistration,
    /// Promise lifetime from its creation (Go's mock: 1 h).
    pub promise_timeout: Duration,
    /// Overrides `expires_at` (pruning tests).
    pub expires_at: Option<SystemTime>,
    pub shard_retention: Duration,
}

pub struct FakeChain {
    pub answers: Mutex<ChainAnswers>,
    pub verify_calls: AtomicUsize,
    /// When set, `verify_promise` waits for a permit (stream-cap tests).
    pub verify_gate: Mutex<Option<Arc<Semaphore>>>,
    pub verify_inflight: AtomicUsize,
    pub verify_max_inflight: AtomicUsize,
}

impl FakeChain {
    pub fn new(set: ValidatorSet) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(ChainAnswers {
                set: Ok(set),
                budget: Ok(1 << 40),
                status: Ok(NodeStatus {
                    chain_id: CHAIN_ID.into(),
                    height: 42,
                    block_time: Some(SystemTime::now()),
                    catching_up: false,
                }),
                registration: ProviderRegistration {
                    found: true,
                    host: "127.0.0.1:7980".into(),
                },
                promise_timeout: Duration::from_secs(3600),
                expires_at: None,
                shard_retention: Duration::ZERO,
            }),
            verify_calls: AtomicUsize::new(0),
            verify_gate: Mutex::new(None),
            verify_inflight: AtomicUsize::new(0),
            verify_max_inflight: AtomicUsize::new(0),
        })
    }
}

fn ready<T: Send + 'static>(v: T) -> BoxFuture<'static, T> {
    Box::pin(async move { v })
}

impl ChainState for FakeChain {
    fn chain_id(&self) -> String {
        CHAIN_ID.into()
    }

    fn validator_set(&self, _height: u64) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
        ready(self.answers.lock().unwrap().set.clone())
    }

    fn head_validator_set(&self) -> BoxFuture<'_, Result<ValidatorSet, StateError>> {
        ready(self.answers.lock().unwrap().set.clone())
    }

    fn verify_promise<'a>(
        &'a self,
        promise: &'a PaymentPromise,
    ) -> BoxFuture<'a, Result<VerifiedPromise, StateError>> {
        Box::pin(async move {
            self.verify_calls.fetch_add(1, Ordering::SeqCst);
            let gate = self.verify_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                let n = self.verify_inflight.fetch_add(1, Ordering::SeqCst) + 1;
                self.verify_max_inflight.fetch_max(n, Ordering::SeqCst);
                let _permit = gate.acquire().await.unwrap();
                self.verify_inflight.fetch_sub(1, Ordering::SeqCst);
            }
            let a = self.answers.lock().unwrap();
            let created = fibre_server_timestamp(promise);
            let expiration = created + a.promise_timeout;
            if SystemTime::now() >= expiration {
                return Err(StateError::other(format!(
                    "payment promise expired: creation_timestamp {created:?} + timeout {:?}",
                    a.promise_timeout
                )));
            }
            Ok(VerifiedPromise {
                expires_at: a.expires_at.unwrap_or(expiration),
                shard_retention: a.shard_retention,
            })
        })
    }

    fn full_stake_storage_budget(&self) -> BoxFuture<'_, Result<i64, StateError>> {
        ready(self.answers.lock().unwrap().budget.clone())
    }

    fn node_status(&self) -> BoxFuture<'_, Result<NodeStatus, StateError>> {
        let mut st = self.answers.lock().unwrap().status.clone();
        if let Ok(s) = &mut st {
            s.block_time = Some(SystemTime::now());
        }
        ready(st)
    }

    fn provider_registration<'a>(
        &'a self,
        _consensus_address: &'a [u8; 20],
    ) -> BoxFuture<'a, Result<ProviderRegistration, StateError>> {
        ready(Ok(self.answers.lock().unwrap().registration.clone()))
    }
}

pub fn fibre_server_timestamp(p: &PaymentPromise) -> SystemTime {
    let ts = p.creation_timestamp.unwrap_or_default();
    UNIX_EPOCH
        + Duration::new(
            u64::try_from(ts.seconds).unwrap_or(0),
            u32::try_from(ts.nanos).unwrap_or(0),
        )
}

/// A local Ed25519 signer whose signing can be held at a gate.
pub struct FakeSigner {
    pub key: Ed25519RawBytesSigner,
    pub sign_gate: Mutex<Option<Arc<Semaphore>>>,
    pub signs: AtomicUsize,
}

impl FakeSigner {
    pub fn new(seed: &[u8; 32]) -> Arc<Self> {
        Arc::new(Self {
            key: Ed25519RawBytesSigner::from_seed(seed),
            sign_gate: Mutex::new(None),
            signs: AtomicUsize::new(0),
        })
    }
}

impl Signer for FakeSigner {
    fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>> {
        ready(Ok(self.key.public_key()))
    }

    fn sign_raw_bytes<'a>(
        &'a self,
        chain_id: &'a str,
        unique_id: &'a str,
        raw: &'a [u8],
    ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>> {
        Box::pin(async move {
            let gate = if unique_id == x_fibre::promise::SIGN_BYTES_PREFIX {
                self.sign_gate.lock().unwrap().clone()
            } else {
                None
            };
            if let Some(gate) = gate {
                let _permit = gate.acquire().await.unwrap();
            }
            self.signs.fetch_add(1, Ordering::SeqCst);
            tlsid::RawBytesSigner::sign_raw_bytes(&self.key, chain_id, unique_id, raw)
                .map_err(SignerError)
        })
    }
}

/// A running server over the fixture's chain.
pub struct TestServer {
    pub server: Server,
    pub chain: Arc<FakeChain>,
    pub signer: Arc<FakeSigner>,
    pub fixture: Fixture,
    pub dir: tempfile::TempDir,
}

pub fn test_config(dir: &std::path::Path) -> ServerConfig {
    let mut cfg = ServerConfig {
        server_listen_address: "127.0.0.1:0".into(),
        store: fibre_server::store::StoreConfig::new(dir.join("store")),
        ..ServerConfig::default()
    };
    cfg.health.check_interval = "50ms".into();
    cfg
}

pub async fn start_with(modify: impl FnOnce(&mut ServerConfig, &FakeChain)) -> TestServer {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let chain = FakeChain::new(fixture.set.clone());
    let signer = FakeSigner::new(&fixture.seed);
    let mut cfg = test_config(dir.path());
    modify(&mut cfg, &chain);
    let server = Server::new(cfg, chain.clone(), signer.clone()).unwrap();
    server.start().await.unwrap();
    TestServer {
        server,
        chain,
        signer,
        fixture,
        dir,
    }
}

pub async fn start() -> TestServer {
    start_with(|_, _| {}).await
}

// --- TLS client ---------------------------------------------------------------

/// Accepts exactly the certificate endorsed by `key` for `chain_id`
/// (Go's client-side `tlsid.VerifyPeer`).
#[derive(Debug)]
pub struct TlsIdVerifier {
    pub key: [u8; 32],
    pub chain_id: String,
    pub provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for TlsIdVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        tlsid::verify_cert(
            end_entity.as_ref(),
            &Ed25519ConsensusKey(self.key),
            &self.chain_id,
            tlsid::unix_now(),
        )
        .map_err(|e| rustls::Error::General(e.to_string()))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is not allowed".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn tls_connector(key: [u8; 32], chain_id: &str) -> tokio_rustls::TlsConnector {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TlsIdVerifier {
            key,
            chain_id: chain_id.into(),
            provider,
        }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec()];
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
}

/// A TLS connection to the server (no HTTP/2 yet).
pub async fn tls_connect(
    addr: std::net::SocketAddr,
    key: [u8; 32],
) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    tls_connector(key, CHAIN_ID)
        .connect(ServerName::try_from("celestia-fibre").unwrap(), tcp)
        .await
}

/// A tonic channel over one TLS connection that verifies the server's
/// identity against `key`.
pub async fn channel(
    addr: std::net::SocketAddr,
    key: [u8; 32],
) -> Result<tonic::transport::Channel, tonic::transport::Error> {
    tonic::transport::Endpoint::from_static("http://fibre.invalid")
        .connect_with_connector(tower::service_fn(move |_uri| async move {
            let tls = tls_connect(addr, key).await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(tls))
        }))
        .await
}

pub async fn unary<Req, Resp>(
    channel: &tonic::transport::Channel,
    path: &'static str,
    req: Req,
) -> Result<Resp, tonic::Status>
where
    Req: prost::Message + Send + Sync + 'static,
    Resp: prost::Message + Default + Send + Sync + 'static,
{
    unary_with(channel, path, tonic::Request::new(req)).await
}

pub async fn unary_with<Req, Resp>(
    channel: &tonic::transport::Channel,
    path: &'static str,
    req: tonic::Request<Req>,
) -> Result<Resp, tonic::Status>
where
    Req: prost::Message + Send + Sync + 'static,
    Resp: prost::Message + Default + Send + Sync + 'static,
{
    let mut grpc = tonic::client::Grpc::new(channel.clone())
        .max_decoding_message_size(usize::MAX)
        .max_encoding_message_size(usize::MAX);
    grpc.ready()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
    grpc.unary(
        req,
        http::uri::PathAndQuery::from_static(path),
        tonic::codec::ProstCodec::<Req, Resp>::default(),
    )
    .await
    .map(tonic::Response::into_inner)
}

pub const UPLOAD: &str = "/celestia.fibre.v1.Fibre/UploadShard";
pub const DOWNLOAD: &str = "/celestia.fibre.v1.Fibre/DownloadShard";
pub const HEALTH_CHECK: &str = "/grpc.health.v1.Health/Check";
pub const HEALTH_WATCH: &str = "/grpc.health.v1.Health/Watch";

/// Polls `f` until it holds or `timeout` passes.
pub async fn eventually(timeout: Duration, mut f: impl AsyncFnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if f().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
