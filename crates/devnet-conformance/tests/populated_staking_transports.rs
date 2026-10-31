//! Rust replay of the exact populated-staking inputs admitted by PR #204.
//!
//! The Go capture remains the oracle: this test reconstructs its chain from
//! `genesis.json` and `abci.rec`, binds the Rust state to the recorded chain,
//! height and app hashes, then sends each recorded protobuf request through
//! direct ABCI, Comet JSON-RPC, Cosmos gRPC and REST.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use celestia_proto::celestia_core_proto::abci;
use celestia_proto::celestia_core_proto::tendermint::crypto::ProofOp as AbciProofOp;
use devnet_conformance::querycorpus::canonical_value;
use ics23_proofs::{CommitmentOp, ProofOp};
use kvdb::MemBackend;
use rpc::{AppQuery, Environment, NodeInfo, NodeStatus};
use serde_json::Value;

const SCENARIO: &str = "v105-populated-staking";

fn capture() -> Value {
    let path = devnet_conformance::scenario_dir(SCENARIO).join("capture/transports.json");
    serde_json::from_slice(&std::fs::read(path).expect("read admitted transport capture"))
        .expect("decode admitted transport capture")
}

fn assert_admitted_inputs() {
    let root = devnet_conformance::scenario_dir(SCENARIO);
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(root.join("capture/metadata.json")).expect("read admitted metadata"),
    )
    .expect("decode admitted metadata");
    for name in [
        "genesis.json",
        "abci.rec",
        "capture/transports.json",
        "state/40.json",
    ] {
        let expected = metadata["artifact_sha256"][name]
            .as_str()
            .expect("admitted artifact hash");
        let actual = hash::sha256(&std::fs::read(root.join(name)).expect("read admitted artifact"));
        assert_eq!(hex::encode(actual), expected, "admitted artifact {name}");
    }
}

fn hex_bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex string")).expect("valid hex")
}

fn replayed_app(capture: &Value) -> app::App {
    let mut app = app::App::new();
    app.set_pruning(app::PruningOptions::nothing());
    let report = devnet_conformance::replay::replay_into(SCENARIO, &mut app, 0, None)
        .expect("replay admitted ABCI stream");
    assert!(
        report.errors.is_empty(),
        "replay errors: {:?}",
        report.errors
    );
    assert_eq!(report.first_mismatch, None, "{:#?}", report.store_diffs);
    assert_eq!(report.matched_heights, (1_i64..=40).collect::<Vec<_>>());

    let chain_id = capture["chain_id"].as_str().expect("chain id");
    let query_height = capture["query_height"].as_i64().expect("query height");
    let query_root = hex_bytes(&capture["app_hash"]);
    let committed = app
        .multistore()
        .root()
        .get_commit_info(query_height)
        .expect("query-height commit info")
        .hash()
        .expect("query-height app hash");
    assert_eq!(app.chain_id(), chain_id);
    assert_eq!(committed.as_slice(), query_root.as_slice());

    let final_state: Value = serde_json::from_slice(
        &std::fs::read(devnet_conformance::scenario_dir(SCENARIO).join("state/40.json"))
            .expect("read final Go state dump"),
    )
    .expect("decode final Go state dump");
    let info = app.info();
    assert_eq!(
        info.last_block_height,
        capture["tip_height"].as_i64().unwrap()
    );
    assert_eq!(
        info.last_block_app_hash,
        hex_bytes(&final_state["app_hash"]),
        "Rust final state must equal the admitted Go state"
    );
    app
}

fn canonical(method: &str, bytes: &[u8]) -> String {
    canonical_value(method, &hex::encode(bytes))
}

fn assert_response_value(case: &Value, bytes: &[u8], transport: &str) {
    let method = case["method"].as_str().unwrap();
    let expected = match case["transports"][transport]["response_hex"].as_str() {
        Some(value) => hex::decode(value).expect("valid recorded response hex"),
        None if transport == "comet_rpc" => base64::engine::general_purpose::STANDARD
            .decode(
                case["transports"]["comet_rpc"]["body"]["result"]["response"]["value"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .expect("valid recorded Comet response base64"),
        None => panic!("missing recorded response for {transport}"),
    };
    assert_eq!(
        canonical(method, bytes),
        canonical(method, &expected),
        "{} via {transport}",
        case["name"]
    );
}

fn assert_abci_response(case: &Value, response: &abci::ResponseQuery) {
    let expected = &case["transports"]["abci"]["body"];
    assert_eq!(u64::from(response.code), expected["code"].as_u64().unwrap());
    assert_eq!(response.log, expected["log"].as_str().unwrap());
    assert_eq!(response.info, expected["info"].as_str().unwrap());
    assert_eq!(
        response.index.to_string(),
        expected["index"].as_str().unwrap()
    );
    assert!(response.key.is_empty());
    assert!(expected["key"].is_null());
    assert!(response.proof_ops.is_none());
    assert!(expected["proofOps"].is_null());
    assert_eq!(
        response.height.to_string(),
        expected["height"].as_str().unwrap()
    );
    assert_eq!(response.codespace, expected["codespace"].as_str().unwrap());
    assert_response_value(case, &response.value, "abci");
}

fn canonical_rest(mut value: Value) -> Value {
    if let Some(rewards) = value.get_mut("rewards").and_then(Value::as_array_mut) {
        rewards.sort_by(|a, b| {
            a["validator_address"]
                .as_str()
                .cmp(&b["validator_address"].as_str())
        });
    }
    value
}

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
        panic!("query-only verifier cannot check transactions")
    }
}

impl mempool::AppConnMempool for Query {
    fn check_tx(&self, _: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        panic!("query-only verifier cannot check transactions")
    }

    fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
        panic!("query-only verifier has no sequence query")
    }
}

struct Node {
    chain_id: String,
    app_version: u64,
}

impl NodeStatus for Node {
    fn node_info(&self) -> NodeInfo {
        NodeInfo {
            network: self.chain_id.clone(),
            app_version: self.app_version,
            ..NodeInfo::default()
        }
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
        panic!("query-only verifier cannot add evidence")
    }
}

fn rpc_environment(app: app::App, capture: &Value) -> Arc<Environment> {
    let chain_id = app.chain_id().to_owned();
    let app_version = app.info().app_version;
    let height = capture["tip_height"].as_i64().unwrap();
    let query = Arc::new(Query(Mutex::new(app)));
    let pool = Arc::new(mempool::TxPool::new(
        mempool::MempoolConfig::celestia_app(),
        query.clone(),
        height,
    ));
    Arc::new(Environment::new(
        nodestore::BlockStore::new(Arc::new(MemBackend::new())).unwrap(),
        nodestore::StateStore::new(Arc::new(MemBackend::new())),
        query,
        pool,
        Arc::new(Node {
            chain_id,
            app_version,
        }),
        Arc::new(Evidence),
        ctypes::PubKey::Ed25519([0; 32]),
        rpc::GenesisData::default(),
        None,
        None,
        Arc::new(rpc::EventBus::new()),
        rpc::RpcConfig::default(),
    ))
}

fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    height: Option<i64>,
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let stream = TcpStream::connect(addr).expect("connect HTTP transport");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut stream = BufReader::new(stream);
    write!(
        stream.get_mut(),
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    )
    .unwrap();
    if let Some(height) = height {
        write!(
            stream.get_mut(),
            "{}: {height}\r\n",
            api_server::BLOCK_HEIGHT_HEADER
        )
        .unwrap();
    }
    write!(stream.get_mut(), "\r\n").unwrap();
    stream.get_mut().write_all(body).unwrap();
    let mut status = String::new();
    stream.read_line(&mut status).unwrap();
    let status = status.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim().split_once(':').unwrap();
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    (status, headers, response)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn assert_comet_response(case: &Value, actual: &Value) {
    let expected = &case["transports"]["comet_rpc"]["body"];
    assert_eq!(actual["jsonrpc"], expected["jsonrpc"]);
    assert_eq!(actual["id"], expected["id"]);
    let actual = &actual["result"]["response"];
    let expected = &expected["result"]["response"];
    for field in [
        "code",
        "log",
        "info",
        "index",
        "key",
        "proofOps",
        "height",
        "codespace",
    ] {
        assert_eq!(
            actual[field], expected[field],
            "{} Comet field {field}",
            case["name"]
        );
    }
    let actual_value = base64::engine::general_purpose::STANDARD
        .decode(actual["value"].as_str().unwrap_or_default())
        .unwrap();
    assert_response_value(case, &actual_value, "comet_rpc");
}

fn verify_rust_proof_against_admitted_root(app: &mut app::App, capture: &Value) {
    assert!(
        capture["omissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "proof validity is not asserted by this transport capture")
    );
    let state: Value = serde_json::from_slice(
        &std::fs::read(devnet_conformance::scenario_dir(SCENARIO).join("state/40.json")).unwrap(),
    )
    .unwrap();
    let staking = state["stores"]
        .as_array()
        .unwrap()
        .iter()
        .find(|store| store["name"] == "staking")
        .unwrap();
    let key = hex_bytes(&staking["kv"][0][0]);
    let query_height = capture["query_height"].as_i64().unwrap();
    let response = app.query(&abci::RequestQuery {
        path: "/store/staking/key".into(),
        data: key,
        height: query_height,
        prove: true,
    });
    assert_eq!(response.code, 0, "{}", response.log);
    let ops = response.proof_ops.expect("Rust store proof").ops;
    assert_eq!(ops.len(), 2, "IAVL and multistore proof operations");
    let decode = |op: &AbciProofOp| {
        CommitmentOp::from_proof_op(ProofOp {
            field_type: op.r#type.clone(),
            key: op.key.clone(),
            data: op.data.clone(),
        })
        .expect("decode Rust commitment operation")
    };
    let iavl = decode(&ops[0]);
    let store_roots = if response.value.is_empty() {
        iavl.run(&[])
    } else {
        iavl.run(&[response.value.as_slice()])
    }
    .expect("verify Rust IAVL proof");
    let multistore = decode(&ops[1]);
    let app_roots = multistore
        .run(&[store_roots[0].as_slice()])
        .expect("verify Rust multistore proof");
    let admitted_root = hex_bytes(&capture["app_hash"]);
    assert_eq!(app_roots[0].as_slice(), admitted_root.as_slice());
}

fn assert_capture_policy(capture: &Value) {
    assert_eq!(capture["schema_version"], 1);
    assert_eq!(capture["cases"].as_array().unwrap().len(), 11);
    assert_eq!(
        capture["normalizations"],
        serde_json::json!([{
            "case": "delegation-rewards",
            "policy": "validator-address-order",
            "reason": "Go iterates the validator rewards map independently on each transport"
        }])
    );
    assert_eq!(
        capture["omissions"],
        serde_json::json!([
            "proof validity is not asserted by this transport capture",
            "non-staking module workloads remain separate captures",
            "invalid UTF-8 and Any/JSON edge cases remain separate captures"
        ])
    );
}

fn verify_direct_and_proof(capture: &Value) {
    let mut direct = replayed_app(capture);
    for case in capture["cases"].as_array().unwrap() {
        let response = direct.query(&abci::RequestQuery {
            path: case["method"].as_str().unwrap().into(),
            data: hex_bytes(&case["request_hex"]),
            height: case["height"].as_i64().unwrap(),
            prove: false,
        });
        assert_abci_response(case, &response);
    }
    verify_rust_proof_against_admitted_root(&mut direct, capture);
}

async fn verify_comet(capture: &Value) {
    let rpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_addr = rpc_listener.local_addr().unwrap();
    tokio::spawn(rpc::server::serve(
        rpc_listener,
        rpc_environment(replayed_app(capture), capture),
    ));
    for case in capture["cases"].as_array().unwrap() {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "abci_query",
            "params": {
                "path": case["method"].as_str().unwrap(),
                "data": case["request_hex"].as_str().unwrap().to_uppercase(),
                "height": case["height"].as_i64().unwrap().to_string(),
                "prove": false
            }
        })
        .to_string();
        let (status, _, response) = tokio::task::spawn_blocking(move || {
            http_request(rpc_addr, "POST", "/", None, body.as_bytes())
        })
        .await
        .unwrap();
        assert_eq!(status, 200);
        assert_comet_response(case, &serde_json::from_slice(&response).unwrap());
    }
}

async fn verify_grpc_and_rest(capture: &Value) {
    let api = Arc::new(app::AbciApp::new(replayed_app(capture)));
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    tokio::spawn(api_server::serve_grpc_on(grpc_listener, Arc::clone(&api)));
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_addr = rest_listener.local_addr().unwrap();
    tokio::spawn(api_server::serve_rest_on(rest_listener, api));

    let channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = tonic::client::Grpc::new(channel);
    for case in capture["cases"].as_array().unwrap() {
        client.ready().await.unwrap();
        let mut request = tonic::Request::new(hex_bytes(&case["request_hex"]));
        request.metadata_mut().insert(
            api_server::BLOCK_HEIGHT_HEADER,
            case["height"]
                .as_i64()
                .unwrap()
                .to_string()
                .parse()
                .unwrap(),
        );
        let path: http::uri::PathAndQuery = case["method"].as_str().unwrap().parse().unwrap();
        let response = client
            .unary(request, path, api_server::RawCodec)
            .await
            .unwrap();
        let expected_height = case["height"].as_i64().unwrap().to_string();
        assert_eq!(
            response
                .metadata()
                .get(api_server::BLOCK_HEIGHT_HEADER)
                .unwrap(),
            expected_height.as_str()
        );
        assert_response_value(case, response.get_ref(), "cosmos_grpc");

        let rest_path = case["rest_path"].as_str().unwrap().to_owned();
        let height = case["height"].as_i64().unwrap();
        let (status, headers, body) = tokio::task::spawn_blocking(move || {
            http_request(rest_addr, "GET", &rest_path, Some(height), &[])
        })
        .await
        .unwrap();
        assert_eq!(
            status,
            u16::try_from(case["transports"]["rest"]["status"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(header(&headers, "content-type"), Some("application/json"));
        assert_eq!(
            header(&headers, "grpc-metadata-content-type"),
            Some("application/grpc+cosmos-sdk-grpc-codec")
        );
        assert_eq!(
            header(&headers, "grpc-metadata-x-cosmos-block-height"),
            Some(expected_height.as_str())
        );
        let actual: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            canonical_rest(actual),
            canonical_rest(case["transports"]["rest"]["body"].clone()),
            "{} via REST",
            case["name"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_populated_inputs_match_all_rust_transports() {
    let capture = capture();
    assert_admitted_inputs();
    assert_capture_policy(&capture);
    verify_direct_and_proof(&capture);
    verify_comet(&capture).await;
    verify_grpc_and_rest(&capture).await;
}
