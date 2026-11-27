//! The embedded Fibre server inside `celestia-rs start`.
//!
//! - `local_devnet_serves_fibre_end_to_end`: a `--network local` style
//!   single-validator devnet (the binary) with `--fibre-listen-address`:
//!   the TLS certificate verifies against the consensus key, liveness is
//!   SERVING, readiness turns SERVING after `tx valaddr set-host`, lumina's
//!   Fibre client uploads (escrow funded with `tx fibre deposit-to-escrow`)
//!   and downloads a blob, the Prometheus endpoint shows the upload, and
//!   SIGTERM stops the server gracefully.
//! - `busy_fibre_port_does_not_stop_consensus`: the Fibre port is taken;
//!   the node still commits blocks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use celestia_rs::config::{File, set};
use celestia_rs::node::{self, StartOptions};
use fibre_server::tlsid::{self, Ed25519ConsensusKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::Value;

const CHAIN: &str = "celestia-rs-local";
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/home");
/// The escrow account's secp256k1 key (test only).
const ESCROW_SECRET: [u8; 32] = [0x11; 32];

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// --- TLS identity client --------------------------------------------------

/// Accepts exactly the certificate endorsed by `key` for `chain_id`.
#[derive(Debug)]
struct TlsIdVerifier {
    key: [u8; 32],
    chain_id: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
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

async fn tls_connect(
    addr: SocketAddr,
    key: [u8; 32],
) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TlsIdVerifier {
            key,
            chain_id: CHAIN.into(),
            provider,
        }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec()];
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
        .connect(ServerName::try_from("celestia-fibre").unwrap(), tcp)
        .await
}

async fn health(addr: SocketAddr, key: [u8; 32], service: &str) -> Result<i32, String> {
    let channel = tonic::transport::Endpoint::from_static("http://fibre.invalid")
        .connect_with_connector(tower::service_fn(move |_uri| async move {
            let tls = tls_connect(addr, key).await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(tls))
        }))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await.map_err(|e| e.to_string())?;
    let resp: tonic::Response<celestia_proto_health::HealthCheckResponse> = grpc
        .unary(
            tonic::Request::new(celestia_proto_health::HealthCheckRequest {
                service: service.into(),
            }),
            http::uri::PathAndQuery::from_static("/grpc.health.v1.Health/Check"),
            tonic::codec::ProstCodec::default(),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.into_inner().status)
}

/// `grpc.health.v1` messages.
mod celestia_proto_health {
    pub use fibre_server::proto::{HealthCheckRequest, HealthCheckResponse};
}

const SERVING: i32 = 1;
const NOT_SERVING: i32 = 2;

async fn wait_health(addr: SocketAddr, key: [u8; 32], service: &str, want: i32, node: &mut Node) {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut last = Err(String::new());
    while Instant::now() < deadline {
        node.check_running();
        last = health(addr, key, service).await;
        if last == Ok(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "health {service:?} is {last:?}, want {want}\n{}",
        node.log()
    );
}

// --- the node binary ------------------------------------------------------

struct Node {
    child: Child,
    log: std::path::PathBuf,
}

impl Node {
    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn check_running(&mut self) {
        if let Some(status) = self.child.try_wait().unwrap() {
            panic!("node exited with {status}\n{}", self.log());
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_celestia-rs"));
    c.env_remove("CELESTIA_RS_FIBRE")
        .env_remove("CELESTIA_RS_FIBRE_LISTEN_ADDRESS")
        .env_remove("CELESTIA_RS_NETWORK");
    c
}

fn run_json(home: &Path, args: &[&str]) -> Value {
    let out = bin()
        .args(["--home", home.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .unwrap_or("{}");
    serde_json::from_str(line).unwrap()
}

fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: x\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn validator_key(home: &Path) -> [u8; 32] {
    let key = privval::FilePvKey::from_json(
        &std::fs::read(home.join("config/priv_validator_key.json")).unwrap(),
    )
    .unwrap();
    key.pub_key
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_devnet_serves_fibre_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    celestia_rs::local::setup(home).unwrap();
    let (rpc, p2p, core, app_grpc, prom, fibre) = (
        free_port(),
        free_port(),
        free_port(),
        free_port(),
        free_port(),
        free_port(),
    );
    set(
        home,
        File::Config,
        "rpc.laddr",
        &format!("tcp://127.0.0.1:{rpc}"),
    )
    .unwrap();
    set(
        home,
        File::Config,
        "rpc.grpc_laddr",
        &format!("tcp://127.0.0.1:{core}"),
    )
    .unwrap();
    set(
        home,
        File::Config,
        "p2p.laddr",
        &format!("tcp://127.0.0.1:{p2p}"),
    )
    .unwrap();
    set(home, File::Config, "instrumentation.prometheus", "true").unwrap();
    set(
        home,
        File::Config,
        "instrumentation.prometheus_listen_addr",
        &format!("127.0.0.1:{prom}"),
    )
    .unwrap();
    set(
        home,
        File::App,
        "grpc.address",
        &format!("127.0.0.1:{app_grpc}"),
    )
    .unwrap();
    // `[fibre]` is not in Go's template: `config set` creates it.
    set(home, File::Config, "fibre.health.check_interval", "1s").unwrap();
    set(home, File::Config, "fibre.max_concurrent_streams", "4").unwrap();

    let mut kr = keyring::Keyring::new(home, keyring::Backend::Test);
    let escrow = kr
        .add_secret("escrow", ESCROW_SECRET)
        .unwrap()
        .bech32("celestia");

    let log = home.join("node.log");
    let log_file = std::fs::File::create(&log).unwrap();
    let fibre_addr: SocketAddr = format!("127.0.0.1:{fibre}").parse().unwrap();
    let child = bin()
        .args([
            "--home",
            home.to_str().unwrap(),
            "start",
            "--fibre-listen-address",
            &fibre_addr.to_string(),
        ])
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
        .spawn()
        .unwrap();
    let mut node = Node { child, log };
    let key = validator_key(home);

    // The certificate verifies against the consensus key; liveness serves.
    wait_health(fibre_addr, key, "fibre-liveness", SERVING, &mut node).await;
    // A certificate endorsed by another key is refused.
    assert!(tls_connect(fibre_addr, [7; 32]).await.is_err());
    // Not registered yet: not ready.
    assert_eq!(health(fibre_addr, key, "").await, Ok(NOT_SERVING));
    assert!(
        node.log().contains("fibre server started"),
        "{}",
        node.log()
    );

    let node_flag = format!("tcp://127.0.0.1:{rpc}");
    let tx = |args: &[&str]| {
        let mut all = vec!["tx"];
        all.extend_from_slice(args);
        all.extend_from_slice(&[
            "--keyring-backend",
            "test",
            "--chain-id",
            CHAIN,
            "--node",
            &node_flag,
            "--fees",
            "40000utia",
            "--gas",
            "400000",
            "--wait",
        ]);
        let v = run_json(home, &all);
        assert_eq!(v["code"].as_u64(), Some(0), "{args:?}: {v}");
    };
    tx(&["bank", "send", "validator", &escrow, "2000000000utia"]);
    tx(&[
        "valaddr",
        "set-host",
        &fibre_addr.to_string(),
        "--from",
        "validator",
    ]);
    let deposit =
        format!(r#"{{"signer":"{escrow}","amount":{{"denom":"utia","amount":"1000000000"}}}}"#);
    tx(&[
        "fibre",
        "deposit-to-escrow",
        "--json",
        &deposit,
        "--from",
        "escrow",
    ]);

    // Registered and bonded: ready.
    wait_health(fibre_addr, key, "", SERVING, &mut node).await;
    let valcons = sdk::address::to_bech32(
        sdk::address::VALCONS_PREFIX,
        &crypto::ed25519::address(&key),
    );
    let provider = run_json(
        home,
        &[
            "query", "valaddr", "provider", &valcons, "--node", &node_flag,
        ],
    );
    assert_eq!(provider["found"], true, "{provider}");
    assert_eq!(provider["info"]["host"], fibre_addr.to_string());

    // lumina's Fibre client finds the server through the chain and
    // uploads, then downloads.
    let client = celestia_fibre::FibreClient::from_endpoint(
        format!("http://127.0.0.1:{app_grpc}").as_str(),
        celestia_fibre::FibreClientConfig::new(CHAIN).unwrap(),
    )
    .unwrap();
    let data: Vec<u8> = (0..300_000u32)
        .map(|i| (i * 7 + 3).to_le_bytes()[0])
        .collect();
    let blob = celestia_fibre::EncodedBlob::new(&data, celestia_fibre::BlobConfig::v0()).unwrap();
    let id = blob.id().clone();
    let signing_key = k256::ecdsa::SigningKey::from_slice(&ESCROW_SECRET).unwrap();
    let namespace = fibre_types::nmt::Namespace::new_v0(b"fibre-e2e").unwrap();
    let promise = tokio::time::timeout(
        Duration::from_secs(120),
        client.upload(&signing_key, namespace, blob),
    )
    .await
    .expect("upload finished")
    .unwrap_or_else(|e| panic!("upload: {e}\n{}", node.log()));
    assert_eq!(promise.validator_signatures.len(), 1);
    let got = tokio::time::timeout(
        Duration::from_secs(120),
        client.download(&id, celestia_fibre::DownloadOptions::default()),
    )
    .await
    .expect("download finished")
    .unwrap();
    assert_eq!(got.data(), data.as_slice());

    // The node's Prometheus endpoint carries the server's series.
    let metrics = http_get(format!("127.0.0.1:{prom}").parse().unwrap(), "/metrics");
    assert!(
        metrics.contains(&format!(
            "fibre_server_upload_shard_total{{chain_id=\"{CHAIN}\",grpc_code=\"OK\"}} 1"
        )),
        "{metrics}"
    );
    assert!(metrics.contains(&format!("fibre_server_ready{{chain_id=\"{CHAIN}\"}} 1")));
    assert!(metrics.contains("cometbft_consensus_height"));

    // SIGTERM: readiness off, drain, store closed, clean exit.
    let pid = node.child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = node.child.try_wait().unwrap() {
            break s;
        }
        assert!(
            Instant::now() < deadline,
            "node did not stop\n{}",
            node.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = node.log();
    assert!(status.success(), "{status}\n{log}");
    let stopping = log.find("stopping fibre server").expect("stop logged");
    let stopped = log.find("fibre server stopped").expect("stopped logged");
    assert!(stopping < stopped);
    assert!(log[stopped..].contains("node stopped"), "{log}");
    assert!(TcpStream::connect(fibre_addr).is_err());
    assert!(home.join("data/fibre").is_dir());
}

// --- in process -----------------------------------------------------------

fn copy_fixture(home: &Path) {
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    for f in ["genesis.json", "priv_validator_key.json"] {
        std::fs::copy(Path::new(FIXTURE).join("config").join(f), cfg.join(f)).unwrap();
    }
    std::fs::write(
        home.join("data/priv_validator_state.json"),
        r#"{"height":"0","round":0,"step":0}"#,
    )
    .unwrap();
}

#[test]
fn busy_fibre_port_does_not_stop_consensus() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    let busy = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = busy.local_addr().unwrap();
    let mut opts = StartOptions::new(home.to_path_buf(), 3, consensus::ConsensusConfig::default());
    opts.fibre = Some(fibre_server::ServerConfig {
        server_listen_address: addr.to_string(),
        store: fibre_server::store::StoreConfig::new(home.join("data/fibre")),
        ..fibre_server::ServerConfig::default()
    });
    let started = Instant::now();
    node::start(&opts).unwrap();
    assert_eq!(node::open_stores(home).unwrap().0.height(), 3);
    // The node did not wait for Fibre (its retries back off up to 60 s).
    assert!(started.elapsed() < Duration::from_secs(60));
    // The failed start never opened the store.
    assert!(!home.join("data/fibre").exists());
    drop(busy);
}
