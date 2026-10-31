//! Tagged-Go HTTP/1 boundary replay for Comet RPC prechecks and CORS order.
#![allow(clippy::panic, clippy::too_many_lines, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use ctypes::proto::abci;
use kvdb::MemBackend;
use rpc::{AppQuery, Environment, NodeInfo, NodeStatus, RpcConfig};
use serde_json::Value;

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
    let query = Arc::new(Query(Mutex::new(app::App::new())));
    let pool = Arc::new(mempool::TxPool::new(
        mempool::MempoolConfig::celestia_app(),
        query.clone(),
        0,
    ));
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
