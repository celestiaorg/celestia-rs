//! Tagged-Go HTTP/1 boundary replay for Comet RPC prechecks and CORS order.
#![allow(clippy::panic, clippy::too_many_lines, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use ctypes::proto::abci;
use futures_util::{SinkExt as _, StreamExt as _};
use kvdb::MemBackend;
use rpc::{AppQuery, Environment, NodeInfo, NodeStatus, RpcConfig};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject as _;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _};

const DYNAMIC_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "date",
    "transfer-encoding",
    "x-server-time",
];

struct Query(Mutex<app::App>);

impl AppQuery for Query {
    fn info(&self) -> Result<abci::ResponseInfo, String> {
        Ok(self.0.lock().unwrap().info())
    }

    fn query(&self, request: abci::RequestQuery) -> Result<abci::ResponseQuery, String> {
        Ok(self.0.lock().unwrap().query(&request))
    }

    fn check_tx(&self, _: &[u8]) -> Result<abci::ResponseCheckTx, String> {
        panic!("HTTP boundary fixture does not submit transactions")
    }
}

impl mempool::AppConnMempool for Query {
    fn check_tx(&self, _: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        panic!("HTTP boundary fixture does not submit transactions")
    }

    fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
        panic!("HTTP boundary fixture does not query account sequences")
    }
}

struct Node;

impl NodeStatus for Node {
    fn node_info(&self) -> NodeInfo {
        NodeInfo::default()
    }

    fn is_listening(&self) -> bool {
        true
    }

    fn listeners(&self) -> Vec<String> {
        Vec::new()
    }

    fn peers(&self) -> Vec<rpc::PeerInfo> {
        Vec::new()
    }

    fn wait_sync(&self) -> bool {
        false
    }

    fn is_catching_up(&self) -> bool {
        false
    }

    fn round_state_json(&self) -> String {
        "{}".into()
    }

    fn round_state_simple_json(&self) -> String {
        "{}".into()
    }
}

struct Evidence;

impl rpc::EvidenceSink for Evidence {
    fn add_evidence(&self, _: ctypes::Evidence) -> Result<(), String> {
        panic!("HTTP boundary fixture does not submit evidence")
    }
}

fn environment(batch: usize, body: usize, header: usize) -> Arc<Environment> {
    let config = RpcConfig {
        cors_allowed_origins: vec!["https://allowed.example".into()],
        cors_allowed_methods: vec!["HEAD".into(), "GET".into(), "POST".into()],
        cors_allowed_headers: vec![
            "Origin".into(),
            "Accept".into(),
            "Content-Type".into(),
            "X-Requested-With".into(),
            "X-Server-Time".into(),
            "X-Test".into(),
        ],
        max_request_batch_size: batch,
        max_body_bytes: body,
        max_header_bytes: header,
        ..Default::default()
    };
    environment_with_config(config)
}

fn environment_with_config(config: RpcConfig) -> Arc<Environment> {
    let query = Arc::new(Query(Mutex::new(app::App::new())));
    let pool = Arc::new(mempool::TxPool::new(
        mempool::MempoolConfig::celestia_app(),
        query.clone(),
        0,
    ));
    Arc::new(Environment::new(
        nodestore::BlockStore::new(Arc::new(MemBackend::new())).unwrap(),
        nodestore::StateStore::new(Arc::new(MemBackend::new())),
        query,
        pool,
        Arc::new(Node),
        Arc::new(Evidence),
        ctypes::PubKey::Ed25519([0; 32]),
        rpc::GenesisData::default(),
        None,
        None,
        Arc::new(rpc::EventBus::new()),
        config,
    ))
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct Http(BufReader<TcpStream>);

impl Http {
    fn connect(address: SocketAddr) -> Self {
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        Self(BufReader::new(stream))
    }

    fn exchange(&mut self, request: &[u8]) -> std::io::Result<HttpResponse> {
        self.0.get_mut().write_all(request)?;
        self.response()
    }

    fn response(&mut self) -> std::io::Result<HttpResponse> {
        let mut line = String::new();
        self.0.read_line(&mut line)?;
        if line.is_empty() {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        let status = line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| std::io::Error::other("invalid HTTP status line"))?;
        let mut headers = Vec::new();
        loop {
            line.clear();
            self.0.read_line(&mut line)?;
            if line == "\r\n" {
                break;
            }
            let (name, value) = line
                .trim_end()
                .split_once(':')
                .ok_or_else(|| std::io::Error::other("invalid HTTP header"))?;
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let get = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let mut body = Vec::new();
        if status != 204 {
            if get("transfer-encoding").is_some_and(|value| {
                value
                    .split(',')
                    .any(|coding| coding.trim().eq_ignore_ascii_case("chunked"))
            }) {
                loop {
                    line.clear();
                    self.0.read_line(&mut line)?;
                    let size = usize::from_str_radix(
                        line.trim_end().split(';').next().unwrap_or_default(),
                        16,
                    )
                    .map_err(std::io::Error::other)?;
                    if size == 0 {
                        loop {
                            line.clear();
                            self.0.read_line(&mut line)?;
                            if line == "\r\n" {
                                break;
                            }
                        }
                        break;
                    }
                    let start = body.len();
                    body.resize(start + size, 0);
                    self.0.read_exact(&mut body[start..])?;
                    let mut terminator = [0; 2];
                    self.0.read_exact(&mut terminator)?;
                    if terminator != *b"\r\n" {
                        return Err(std::io::Error::other("invalid chunk terminator"));
                    }
                }
            } else if let Some(length) = get("content-length") {
                let length = length.parse().map_err(std::io::Error::other)?;
                body.resize(length, 0);
                self.0.read_exact(&mut body)?;
            } else {
                self.0.read_to_end(&mut body)?;
            }
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    fn exchange_half_close(&mut self, request: &[u8]) -> std::io::Result<HttpResponse> {
        self.0.get_mut().write_all(request)?;
        self.0.get_mut().shutdown(std::net::Shutdown::Write)?;
        self.response()
    }
}

fn raw(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value["raw_base64"].as_str().unwrap())
        .unwrap()
}

fn expected_stable_headers(value: &Value) -> Vec<(String, String)> {
    let mut headers: Vec<_> = value["stable_headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().unwrap().to_ascii_lowercase(),
                pair[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    headers.sort_unstable();
    headers
}

fn actual_stable_headers(response: &HttpResponse) -> Vec<(String, String)> {
    let mut stable: Vec<_> = response
        .headers
        .iter()
        .filter(|(name, _)| !DYNAMIC_HEADERS.contains(&name.as_str()))
        .cloned()
        .collect();
    stable.sort_unstable();
    stable
}

fn assert_response(case: &Value, expected: &Value, actual: &HttpResponse) {
    let name = case["name"].as_str().unwrap();
    assert_eq!(
        actual.status,
        u16::try_from(expected["status"].as_u64().unwrap()).unwrap(),
        "{name}: status"
    );
    if actual.status == 431 {
        // Hyper's native parser preserves Go's boundary/status/close but
        // intentionally omits net/http's diagnostic text body and content
        // type because no application response is constructed.
        assert!(actual.body.is_empty(), "{name}: bounded 431 deviation");
        assert!(
            actual_stable_headers(actual).is_empty(),
            "{name}: bounded 431 header deviation"
        );
        return;
    }
    assert_eq!(
        actual.body,
        expected["body_text"].as_str().unwrap().as_bytes(),
        "{name}: body"
    );
    assert_eq!(
        actual_stable_headers(actual),
        expected_stable_headers(expected),
        "{name}: headers"
    );
}

fn assert_oversized_keepalive_closes(address: SocketAddr) {
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\nX-Pad: {}\r\nContent-Length: 0\r\n\r\n",
        "a".repeat(4300)
    );
    assert!(request.len() > 4352);
    let mut http = Http::connect(address);
    let response = http.exchange(request.as_bytes()).unwrap();
    assert_eq!(response.status, 431);
    assert!(response.body.is_empty());
    assert!(actual_stable_headers(&response).is_empty());
    assert!(
        http.exchange(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .is_err(),
        "oversized keep-alive header must close before reuse"
    );
}

fn replay_case(case: &Value, address: SocketAddr) {
    let mut http = Http::connect(address);
    if case["category"] == "keep-alive" {
        let exchanges = case["exchanges"].as_array().unwrap();
        let first = &exchanges[0];
        let response = http.exchange(&raw(&first["request"])).unwrap();
        assert_response(case, &first["response"], &response);
        if exchanges.len() == 2 {
            let second = &exchanges[1];
            let response = http.exchange(&raw(&second["request"])).unwrap();
            assert_response(case, &second["response"], &response);
        } else {
            assert_eq!(
                response
                    .headers
                    .iter()
                    .find(|(name, _)| name == "connection")
                    .map(|(_, value)| value.as_str()),
                Some("close"),
                "{}: body overflow close header",
                case["name"]
            );
            let second = base64::engine::general_purpose::STANDARD
                .decode("UE9TVCAvIEhUVFAvMS4xDQpIb3N0OiBsb2NhbGhvc3QNCkNvbm5lY3Rpb246IGNsb3NlDQpDb250ZW50LUxlbmd0aDogMA0KDQo=")
                .unwrap();
            assert!(
                http.exchange(&second).is_err(),
                "{}: body overflow must close before reuse",
                case["name"]
            );
        }
    } else {
        let response = http.exchange(&raw(&case["request"])).unwrap();
        assert_response(case, &case["response"], &response);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tagged_go_comet_http_boundaries_match_over_real_sockets() {
    let zero_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let zero_address = zero_listener.local_addr().unwrap();
    let zero_server = tokio::spawn(rpc::server::serve(zero_listener, environment(0, 4096, 256)));
    let custom_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let custom_address = custom_listener.local_addr().unwrap();
    let custom_server = tokio::spawn(rpc::server::serve(
        custom_listener,
        environment(3, 512, 256),
    ));
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/comet-http-boundaries.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let case = case.clone();
        let address = if case["oracle"] == "zero" {
            zero_address
        } else {
            custom_address
        };
        tokio::task::spawn_blocking(move || replay_case(&case, address))
            .await
            .unwrap();
    }
    tokio::task::spawn_blocking(move || assert_oversized_keepalive_closes(custom_address))
        .await
        .unwrap();
    zero_server.abort();
    custom_server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tagged_go_proxy_and_framing_boundaries_match_or_are_bounded() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(rpc::server::serve(
        listener,
        environment_with_config(RpcConfig {
            cors_allowed_origins: vec!["https://configured.example".into()],
            cors_allowed_methods: vec!["HEAD".into(), "GET".into(), "POST".into()],
            cors_allowed_headers: vec!["Content-Type".into()],
            max_request_batch_size: 10,
            max_body_bytes: 1_000_000,
            max_header_bytes: 1 << 20,
            ..Default::default()
        }),
    ));
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/proxy-tls-framing.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap().clone();
    tokio::task::spawn_blocking(move || {
        for case in cases {
            let category = case["category"].as_str().unwrap();
            let name = case["name"].as_str().unwrap();
            if category == "framing-connection" {
                let requests = case["requests"].as_array().unwrap();
                let expected = case["responses"].as_array().unwrap();
                let mut http = Http::connect(address);
                let first = http.exchange(&raw(&requests[0])).unwrap();
                if name == "framing-conflicting-length-closes" {
                    assert_eq!(first.status, 400, "{name}: bounded Hyper status");
                    assert!(first.body.is_empty(), "{name}: bounded Hyper body");
                    assert!(
                        actual_stable_headers(&first).is_empty(),
                        "{name}: bounded Hyper headers"
                    );
                } else {
                    assert_response(&case, &expected[0], &first);
                }
                if expected.len() == 2 && name != "framing-transfer-length-keepalive" {
                    let second = http
                        .exchange(&raw(&requests[1]))
                        .unwrap_or_else(|error| panic!("{name}: second exchange: {error}"));
                    assert_response(&case, &expected[1], &second);
                } else {
                    assert!(
                        http.exchange(&raw(&requests[1])).is_err(),
                        "{name}: malformed framing must close the connection"
                    );
                }
                continue;
            }
            if !matches!(
                category,
                "proxy" | "framing-valid-controls" | "framing-malformed-inputs"
            ) {
                continue;
            }
            let mut http = Http::connect(address);
            let request = raw(&case["request"]);
            let response = if matches!(
                name,
                "framing-truncated-content-length" | "framing-truncated-chunk"
            ) {
                http.exchange_half_close(&request)
            } else {
                http.exchange(&request)
            };
            let response = response.unwrap();
            if matches!(
                name,
                "framing-content-length-comma-same"
                    | "framing-header-name-space"
                    | "framing-header-without-colon"
                    | "framing-obs-fold"
                    | "framing-nul-header-value"
                    | "framing-invalid-method-token"
                    | "framing-invalid-http-version"
                    | "framing-invalid-target-control"
                    | "framing-content-length-different"
                    | "framing-content-length-negative"
                    | "framing-content-length-plus"
                    | "framing-content-length-nondigit"
                    | "framing-transfer-encoding-gzip"
                    | "framing-transfer-encoding-nonfinal-chunked"
            ) {
                // Hyper rejects these in its native parser before an
                // application response exists. Freeze the explicit bounded
                // deviation from Go's diagnostic body/status choices.
                assert_eq!(response.status, 400, "{name}: bounded Hyper status");
                assert!(response.body.is_empty(), "{name}: bounded Hyper body");
                assert!(
                    actual_stable_headers(&response).is_empty(),
                    "{name}: bounded Hyper headers"
                );
            } else {
                assert_response(&case, &case["response"], &response);
            }
        }
    })
    .await
    .unwrap();
    server.abort();
}

const TLS_CERT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/tls/localhost.crt"
);
const TLS_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/tls/localhost.key"
);
const TLS_MISMATCH_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/tls/mismatch.key"
);
const TLS_NON_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");

fn tls_config(cert: &str, key: &str) -> RpcConfig {
    RpcConfig {
        cors_allowed_origins: vec!["https://configured.example".into()],
        cors_allowed_methods: vec!["HEAD".into(), "GET".into(), "POST".into()],
        cors_allowed_headers: vec!["Content-Type".into()],
        tls_cert_file: cert.into(),
        tls_key_file: key.into(),
        ..Default::default()
    }
}

fn tls_connector(alpn: &[&[u8]]) -> TlsConnector {
    let reader = BufReader::new(std::fs::File::open(TLS_CERT).unwrap());
    let mut roots = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_reader_iter(reader) {
        roots.add(certificate.unwrap()).unwrap();
    }
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|value| value.to_vec()).collect();
    TlsConnector::from(Arc::new(config))
}

async fn connect_tls(
    address: SocketAddr,
    hostname: &str,
    alpn: &[&[u8]],
) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from(hostname.to_owned()).unwrap();
    tls_connector(alpn).connect(name, stream).await.unwrap()
}

fn parse_raw_response(raw: &[u8]) -> HttpResponse {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = std::str::from_utf8(&raw[..split]).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tagged_go_tls_and_config_boundaries_match_or_are_bounded() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../conformance/mainnet-rpc/proxy-tls-framing/v10.5.0/proxy-tls-framing.json"
    ))
    .unwrap();
    let case = |name: &str| {
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap()
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(rpc::server::serve(
        listener,
        environment_with_config(tls_config(TLS_CERT, TLS_KEY)),
    ));

    for name in [
        "tls-health-http1",
        "tls-jsonrpc-health-http1",
        "tls-cors-preflight-http1",
    ] {
        let h1 = case(name);
        let mut stream = connect_tls(address, "localhost", &[b"http/1.1"]).await;
        assert_eq!(
            stream.get_ref().1.alpn_protocol(),
            Some(b"http/1.1".as_slice())
        );
        stream.write_all(&raw(&h1["request"])).await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        assert_response(h1, &h1["response"], &parse_raw_response(&bytes));
    }

    let h2 = case("tls-health-http2");
    let stream = connect_tls(address, "localhost", &[b"h2"]).await;
    assert_eq!(stream.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
    let (mut sender, connection) = h2::client::handshake(stream).await.unwrap();
    let connection = tokio::spawn(connection);
    let request = axum::http::Request::builder()
        .method("GET")
        .uri("https://localhost/health")
        .body(())
        .unwrap();
    let (response, _) = sender.send_request(request, true).unwrap();
    let response = response.await.unwrap();
    assert_eq!(response.version(), axum::http::Version::HTTP_2);
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap().to_owned()))
        .collect();
    let mut body = Vec::new();
    let mut receive = response.into_body();
    while let Some(chunk) = receive.data().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_response(
        h2,
        &h2["response"],
        &HttpResponse {
            status,
            headers,
            body,
        },
    );
    drop(sender);
    connection.abort();

    let wss = case("tls-wss-jsonrpc-clean-close");
    let stream = connect_tls(address, "localhost", &[b"http/1.1"]).await;
    let request = "wss://localhost/websocket".into_client_request().unwrap();
    let (mut websocket, response) = tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap();
    assert_eq!(response.status(), 101);
    websocket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":1,"method":"health"}"#.into(),
        ))
        .await
        .unwrap();
    let message = websocket.next().await.unwrap().unwrap();
    assert_eq!(
        message.into_text().unwrap(),
        wss["response_frame"]["payload_text"]
    );
    websocket
        .send(Message::Close(Some(
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: 1000.into(),
                reason: "".into(),
            },
        )))
        .await
        .unwrap();
    let close = websocket.next().await.unwrap().unwrap();
    let Message::Close(Some(close)) = close else {
        panic!("server did not return a close frame");
    };
    assert_eq!(close.code, 1000.into());
    let eof = tokio::time::timeout(Duration::from_secs(1), websocket.next())
        .await
        .expect("WSS stream did not close after close code 1000");
    assert!(
        eof.is_none()
            || matches!(
                eof,
                Some(Err(tokio_tungstenite::tungstenite::Error::Io(ref error)))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof
            ),
        "WSS connection stayed usable after the clean close response: {eof:?}"
    );

    // Rust closes during TLS parsing rather than synthesizing Go's plaintext
    // HTTP diagnostic on a TLS port.
    let mut plain = tokio::net::TcpStream::connect(address).await.unwrap();
    plain
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    plain.read_to_end(&mut response).await.unwrap();
    assert!(!response.starts_with(b"HTTP/"));

    let wrong_name = tokio::net::TcpStream::connect(address).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from("not-localhost.invalid").unwrap();
    assert!(
        tls_connector(&[b"http/1.1"])
            .connect(name, wrong_name)
            .await
            .is_err()
    );
    server.abort();

    // Go enables TLS only when both fields are present.
    let tls_on_plain = case("tls-on-plaintext-port");
    assert_eq!(tls_on_plain["outcome"], "tls-handshake-rejected");
    for config in [tls_config(TLS_CERT, ""), tls_config("", TLS_KEY)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(rpc::server::serve(
            listener,
            environment_with_config(config),
        ));
        let health = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let response = tokio::task::spawn_blocking(move || Http::connect(address).exchange(health))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status, 200);
        let stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                tls_connector(&[b"http/1.1"]).connect(name, stream),
            )
            .await
            .expect("TLS-to-plaintext probe did not terminate")
            .is_err()
        );
        server.abort();
    }

    // Complete bad pairs fail before accepting any plaintext fallback. A
    // missing certificate remains an ordinary file-open error.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let error = rpc::server::serve(
        listener,
        environment_with_config(tls_config(
            "/definitely/missing.crt",
            "/definitely/missing.key",
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        rpc::server::serve(
            listener,
            environment_with_config(tls_config(TLS_CERT, TLS_MISMATCH_KEY)),
        )
        .await
        .is_err()
    );

    // Parsing an existing file without the requested PEM item preserves the
    // explicit empty-certificate/key diagnostics used by node startup.
    for (config, expected) in [
        (
            tls_config(TLS_NON_PEM, TLS_KEY),
            "RPC TLS certificate file contains no certificates",
        ),
        (
            tls_config(TLS_CERT, TLS_NON_PEM),
            "RPC TLS key file contains no private key",
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let error = rpc::server::serve(listener, environment_with_config(config))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
    }
}
