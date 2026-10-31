//! Current-release real-transport regression for Go's missing ABCI Query value.
//! The P2P-filter routes are stateless; a real Rust App handles every request.
#![allow(clippy::unwrap_used, clippy::panic, clippy::too_many_lines)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ctypes::proto::abci;
use futures_util::{SinkExt as _, StreamExt as _};
use kvdb::MemBackend;
use rpc::{AppQuery, Environment, NodeInfo, NodeStatus};
use serde_json::Value;

struct Query(Mutex<app::App>);
impl AppQuery for Query {
    fn info(&self) -> Result<abci::ResponseInfo, String> {
        Ok(self.0.lock().unwrap().info())
    }
    fn query(&self, req: abci::RequestQuery) -> Result<abci::ResponseQuery, String> {
        Ok(self.0.lock().unwrap().query(&req))
    }
    fn query_response(
        &self,
        req: abci::RequestQuery,
    ) -> Result<Option<abci::ResponseQuery>, String> {
        Ok(self.0.lock().unwrap().query_response(&req))
    }
    fn check_tx(&self, _: &[u8]) -> Result<abci::ResponseCheckTx, String> {
        panic!("query-only regression cannot check a transaction")
    }
}
impl mempool::AppConnMempool for Query {
    fn check_tx(&self, _: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        panic!("query-only regression cannot check a transaction")
    }
    fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
        panic!("no sequence query")
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
        panic!("no evidence query")
    }
}
fn environment() -> Arc<Environment> {
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
        rpc::RpcConfig::default(),
    ))
}

struct Http(BufReader<TcpStream>);
impl Http {
    fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        Self(BufReader::new(stream))
    }
    fn request(
        &mut self,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, Vec<(String, String)>, String) {
        write!(self.0.get_mut(), "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut line = String::new();
        self.0.read_line(&mut line).unwrap();
        let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut headers = Vec::new();
        loop {
            line.clear();
            self.0.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (key, value) = line.trim().split_once(':').unwrap();
            headers.push((key.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let length: usize = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .unwrap()
            .1
            .parse()
            .unwrap();
        assert!(length < 1 << 20);
        let mut body = vec![0; length];
        self.0.read_exact(&mut body).unwrap();
        (status, headers, String::from_utf8(body).unwrap())
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../conformance/mainnet-rpc/v10.5.0/empty-query.json"
    ))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::if_not_else)] // Keep the blocking HTTP arm outside the async WS arm.
async fn p2p_empty_query_matches_current_go_over_reusable_http_and_websocket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(rpc::server::serve(listener, environment()));
    let fixture = fixture();
    for case in fixture["cases"].as_array().unwrap() {
        let case = case.clone();
        if case["transport"] != "websocket" {
            tokio::task::spawn_blocking(move || {
                let mut http = Http::connect(addr);
                let request = &case["request"];
                let (status, headers, body) = http.request(
                    request["method"].as_str().unwrap(),
                    request["path"].as_str().unwrap(),
                    request["body"].as_str().unwrap_or_default(),
                );
                assert_eq!(
                    status,
                    u16::try_from(case["response"]["status"].as_u64().unwrap()).unwrap(),
                    "{case}"
                );
                assert_eq!(
                    body,
                    case["response"]["body"]["text"].as_str().unwrap(),
                    "{case}"
                );
                for name in ["content-type", "cache-control"] {
                    let ours = headers
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.as_str());
                    let expected = case["response"]["headers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|h| h[0].as_str().unwrap().eq_ignore_ascii_case(name))
                        .and_then(|h| h[1].as_str());
                    assert_eq!(ours, expected, "header {name}: {case}");
                }
                assert!(headers.iter().any(|(k, _)| k == "x-server-time"));
                let (status, _, body) = http.request("GET", "/health", "");
                assert_eq!(status, 200);
                assert_eq!(body, case["health_after"]["body"]["text"].as_str().unwrap());
                assert_eq!(case["same_connection"], true);
            })
            .await
            .unwrap();
        } else {
            let (mut ws, upgrade) =
                tokio_tungstenite::connect_async(format!("ws://{addr}/websocket"))
                    .await
                    .unwrap();
            assert_eq!(upgrade.status().as_u16(), 101);
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                case["request"].as_str().unwrap().into(),
            ))
            .await
            .unwrap();
            let response = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(response.is_text(), "expected Go's text-frame opcode");
            assert_eq!(
                response.into_text().unwrap().as_str(),
                case["response"]["frames"][0]["payload"]["text"]
                    .as_str()
                    .unwrap(),
                "{case}"
            );
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"jsonrpc":"2.0","id":8,"method":"health","params":{}}"#.into(),
            ))
            .await
            .unwrap();
            let response = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(response.is_text(), "expected Go's text-frame opcode");
            assert_eq!(
                response.into_text().unwrap().as_str(),
                case["health_after"]["frames"][0]["payload"]["text"]
                    .as_str()
                    .unwrap()
            );
            ws.close(None).await.unwrap();
        }
    }
    server.abort();
}
