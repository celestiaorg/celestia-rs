//! Fresh populated module comparisons over real ABCI, Comet, gRPC and REST listeners.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
use prost::Message as _;
use serde_json::json;
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

use devnet_conformance::querycorpus::strip_go_frames;

fn hex_bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap_or_default()).unwrap()
}
fn load_capture(scenario: &str) -> Value {
    let bytes = std::fs::read(
        devnet_conformance::scenario_dir(scenario).join("capture/transports.json.gz"),
    )
    .unwrap();
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .read_to_end(&mut plain)
        .unwrap();
    serde_json::from_slice(&plain).unwrap()
}
fn replayed_app(scenario: &str, capture: &Value) -> app::App {
    let mut app = app::App::new();
    app.set_pruning(app::PruningOptions::nothing());
    let report = devnet_conformance::replay::replay_into(scenario, &mut app, 0, None).unwrap();
    assert!(report.errors.is_empty(), "{scenario}: {:?}", report.errors);
    assert_eq!(
        report.first_mismatch, None,
        "{scenario}: {:?}",
        report.store_diffs
    );
    let tip = capture["tip_height"].as_i64().unwrap();
    assert_eq!(report.matched_heights, (1..=tip).collect::<Vec<_>>());
    assert_eq!(app.chain_id(), capture["chain_id"].as_str().unwrap());
    let primary = capture["query_height"].as_i64().unwrap();
    let root = app
        .multistore()
        .root()
        .get_commit_info(primary)
        .unwrap()
        .hash()
        .unwrap();
    assert_eq!(root.as_slice(), hex_bytes(&capture["app_hash"]));
    app
}
fn expected_bytes(case: &Value, transport: &str) -> Vec<u8> {
    if transport == "comet_rpc" {
        base64::engine::general_purpose::STANDARD
            .decode(
                case["transports"][transport]["body"]["result"]["response"]["value"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .unwrap()
    } else {
        hex_bytes(&case["transports"][transport]["response_hex"])
    }
}
fn assert_value(case: &Value, bytes: &[u8], transport: &str) {
    let path = case["method"].as_str().unwrap();
    let canonical = |bytes: &[u8]| {
        if let Some(method) = path.strip_prefix(grpc_query::reflection_extra::V2ALPHA1) {
            if let Some(value) = grpc_query::reflection_extra::canonical_v2alpha1(method, bytes) {
                return hex::encode(value);
            }
        }
        canonical_value(path, &hex::encode(bytes))
    };
    assert!(
        canonical(bytes) == canonical(&expected_bytes(case, transport)),
        "{} via {transport}: response differs",
        case["name"]
    );
}

fn integer(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap())
}
fn assert_abci(case: &Value, response: &abci::ResponseQuery) {
    let expected = &case["transports"]["abci"]["body"];
    assert_eq!(
        i64::from(response.code),
        integer(&expected["code"]),
        "{} code",
        case["name"]
    );
    assert_eq!(
        strip_go_frames(&response.log),
        strip_go_frames(expected["log"].as_str().unwrap()),
        "{} log",
        case["name"]
    );
    assert_eq!(response.info, expected["info"].as_str().unwrap());
    assert_eq!(response.index, integer(&expected["index"]));
    assert_eq!(response.height, integer(&expected["height"]));
    assert_eq!(response.codespace, expected["codespace"].as_str().unwrap());
    let key = base64::engine::general_purpose::STANDARD
        .decode(expected["key"].as_str().unwrap_or_default())
        .unwrap();
    assert_eq!(response.key, key, "{} key", case["name"]);
    let ops = &expected["proofOps"]["ops"];
    if let Some(actual) = &response.proof_ops {
        let expected = ops.as_array().expect("Go proof operations");
        assert_eq!(actual.ops.len(), expected.len());
        for (a, e) in actual.ops.iter().zip(expected) {
            assert_eq!(a.r#type, e["type"].as_str().unwrap());
            assert_eq!(
                a.key,
                base64::engine::general_purpose::STANDARD
                    .decode(e["key"].as_str().unwrap_or_default())
                    .unwrap()
            );
            assert_eq!(
                a.data,
                base64::engine::general_purpose::STANDARD
                    .decode(e["data"].as_str().unwrap())
                    .unwrap()
            );
        }
    } else {
        assert!(ops.is_null(), "{} missing proof", case["name"]);
    }
    assert_value(case, &response.value, "abci");
}
fn canonical_rest(mut value: Value) -> Value {
    if let Some(Value::String(message)) = value.get_mut("message") {
        *message = strip_go_frames(message);
    }
    if let Some(object) = value.as_object_mut() {
        object.remove("application_version");
    }
    for (list, key) in [("subspaces", "subspace"), ("rewards", "validator_address")] {
        if let Some(Value::Array(values)) = value.get_mut(list) {
            values.sort_by(|a, b| a[key].as_str().cmp(&b[key].as_str()));
        }
    }
    value
}
fn header_json(headers: &http::HeaderMap) -> Value {
    let mut out = serde_json::Map::new();
    for key in headers.keys() {
        out.insert(
            key.to_string(),
            json!(
                headers
                    .get_all(key)
                    .iter()
                    .map(|value| hex::encode(value.as_bytes()))
                    .collect::<Vec<_>>()
            ),
        );
    }
    Value::Object(out)
}
fn assert_grpc_headers(case: &Value, headers: &http::HeaderMap) {
    let expected = &case["transports"]["cosmos_grpc"];
    // grpc-go puts a trailers-only response's content type in Trailer metadata.
    let expected = if expected["headers"].is_object() {
        &expected["headers"]
    } else {
        &expected["trailers"]
    };
    for key in ["content-type", "x-cosmos-block-height"] {
        let actual: Vec<_> = headers
            .get_all(key)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        let expected: Vec<_> = expected[key]
            .as_array()
            .map(|values| values.iter().map(|value| value.as_str().unwrap()).collect())
            .unwrap_or_default();
        assert_eq!(actual, expected, "{} gRPC {key}", case["name"]);
    }
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

fn verify_proofs(app: &mut app::App, capture: &Value, scenario: &str) {
    let official: Vec<Value> = serde_json::from_slice(
        &std::fs::read(devnet_conformance::scenario_dir(scenario).join("capture/proofs.json"))
            .unwrap(),
    )
    .unwrap();
    let mut checked = 0;
    let mut verified = 0;
    for case in capture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["prove"] == true)
    {
        let response = app.query(&abci::RequestQuery {
            path: case["method"].as_str().unwrap().into(),
            data: hex_bytes(&case["request_hex"]),
            height: case["height"].as_i64().unwrap(),
            prove: true,
        });
        if response.code != 0 {
            continue;
        }
        assert_abci(case, &response);
        let Some(ops) = response.proof_ops else {
            continue;
        };
        assert_eq!(ops.ops.len(), 2);
        let decode = |op: &AbciProofOp| {
            CommitmentOp::from_proof_op(ProofOp {
                field_type: op.r#type.clone(),
                key: op.key.clone(),
                data: op.data.clone(),
            })
            .unwrap()
        };
        assert_eq!(ops.ops[0].key, response.key);
        let iavl = decode(&ops.ops[0]);
        let official = official
            .iter()
            .find(|entry| entry["name"] == case["name"])
            .expect("official proof verdict");
        checked += 1;
        let result = if response.value.is_empty() {
            iavl.run(&[])
        } else {
            iavl.run(&[response.value.as_slice()])
        };
        if official["valid"] == false {
            assert!(
                result.is_err(),
                "{} must preserve official verifier rejection",
                case["name"]
            );
            continue;
        }
        let roots = result.unwrap_or_else(|error| panic!("{} proof failed: {error}", case["name"]));
        let app_roots = decode(&ops.ops[1]).run(&[roots[0].as_slice()]).unwrap();
        let root = app
            .multistore()
            .root()
            .get_commit_info(response.height)
            .unwrap()
            .hash()
            .unwrap();
        assert_eq!(
            app_roots[0].as_slice(),
            root.as_slice(),
            "{} proof root",
            case["name"]
        );
        assert_eq!(
            hex::encode(app_roots[0]),
            official["root"].as_str().unwrap()
        );
        verified += 1;
    }
    assert_eq!(checked, official.len());
    assert!(verified > 0, "no independently verified proofs");
}
async fn verify_scenario(scenario: &str) {
    let capture = load_capture(scenario);
    let mut direct = replayed_app(scenario, &capture);
    verify_proofs(&mut direct, &capture, scenario);
    let abci_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let abci_addr = abci_listener.local_addr().unwrap();
    let abci_task = tokio::spawn(abci_server::serve(
        abci_listener,
        Arc::new(app::AbciApp::new(direct)),
    ));
    let cases = capture["cases"].clone();
    let abci_record = tokio::task::spawn_blocking(move || {
        let mut records = Vec::new();
        let mut socket = TcpStream::connect(abci_addr).unwrap();
        socket.set_nodelay(true).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        for case in cases.as_array().unwrap() {
            let response = devnet_conformance::querycorpus::net::query_response(
                &mut socket,
                &abci::RequestQuery {
                    path: case["method"].as_str().unwrap().into(),
                    data: hex_bytes(&case["request_hex"]),
                    height: case["height"].as_i64().unwrap(),
                    prove: case["prove"].as_bool().unwrap(),
                },
            )
            .unwrap()
            .expect("ABCI Query response");
            assert_abci(case, &response);
            records.push(
                json!({"name":case["name"],"response_hex":hex::encode(response.encode_to_vec())}),
            );
        }
        records
    })
    .await
    .unwrap();
    abci_task.abort();
    let rpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_addr = rpc_listener.local_addr().unwrap();
    let rpc_task = tokio::spawn(rpc::server::serve(
        rpc_listener,
        rpc_environment(replayed_app(scenario, &capture), &capture),
    ));
    let mut comet_record = Vec::new();
    for case in capture["cases"].as_array().unwrap() {
        let request=serde_json::json!({"jsonrpc":"2.0","id":1,"method":"abci_query","params":{"path":case["method"],"data":case["request_hex"].as_str().unwrap().to_uppercase(),"height":case["height"].as_i64().unwrap().to_string(),"prove":case["prove"]}}).to_string();
        let (status, _, body) = tokio::task::spawn_blocking(move || {
            http_request(rpc_addr, "POST", "/", None, request.as_bytes())
        })
        .await
        .unwrap();
        assert_eq!(status, 200);
        let actual: Value = serde_json::from_slice(&body).unwrap();
        let expected = &case["transports"]["comet_rpc"]["body"];
        for field in [
            "code",
            "info",
            "index",
            "key",
            "proofOps",
            "height",
            "codespace",
        ] {
            assert_eq!(
                actual["result"]["response"][field], expected["result"]["response"][field],
                "{} Comet {field}",
                case["name"]
            );
        }
        assert_eq!(
            strip_go_frames(actual["result"]["response"]["log"].as_str().unwrap()),
            strip_go_frames(expected["result"]["response"]["log"].as_str().unwrap())
        );
        let value = base64::engine::general_purpose::STANDARD
            .decode(
                actual["result"]["response"]["value"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .unwrap();
        assert_value(case, &value, "comet_rpc");
        comet_record.push(json!({"name":case["name"],"status":status,"body":actual}));
    }
    rpc_task.abort();
    let api = Arc::new(app::AbciApp::new(replayed_app(scenario, &capture)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = listener.local_addr().unwrap();
    let grpc_task = tokio::spawn(api_server::serve_grpc_on(listener, Arc::clone(&api)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_addr = listener.local_addr().unwrap();
    let rest_task = tokio::spawn(api_server::serve_rest_on(listener, api));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let channel = tonic::service::interceptor::InterceptedService::new(
        channel,
        |mut request: tonic::Request<()>| {
            request
                .metadata_mut()
                .insert("content-type", "application/grpc+proto".parse().unwrap());
            Ok(request)
        },
    );
    let mut client = tonic::client::Grpc::new(channel);
    let mut grpc_record = Vec::new();
    let mut rest_record = Vec::new();
    for case in capture["cases"].as_array().unwrap() {
        let height = case["height"].as_i64().unwrap();
        let grpc = &case["transports"]["cosmos_grpc"];
        if !grpc.is_null() {
            client.ready().await.unwrap();
            let mut request = tonic::Request::new(hex_bytes(&case["request_hex"]));
            request.metadata_mut().insert(
                api_server::BLOCK_HEIGHT_HEADER,
                height.to_string().parse().unwrap(),
            );
            let path: http::uri::PathAndQuery = case["method"].as_str().unwrap().parse().unwrap();
            match client.unary(request, path, api_server::RawCodec).await {
                Ok(response) => {
                    let headers = response.metadata().clone().into_headers();
                    assert_grpc_headers(case, &headers);
                    assert_eq!(
                        grpc["code"].as_i64().unwrap_or(0),
                        0,
                        "{} gRPC expected error",
                        case["name"]
                    );
                    assert_value(case, response.get_ref(), "cosmos_grpc");
                    grpc_record.push(json!({"name":case["name"],"code":0,"response_hex":hex::encode(response.get_ref()),"headers_hex":header_json(&headers)}));
                    let expected = grpc["headers"]["x-cosmos-block-height"][0].as_str();
                    assert_eq!(
                        response
                            .metadata()
                            .get(api_server::BLOCK_HEIGHT_HEADER)
                            .map(|v| v.to_str().unwrap()),
                        expected,
                        "{} gRPC height",
                        case["name"]
                    );
                }
                Err(status) => {
                    let headers = status.metadata().clone().into_headers();
                    assert_grpc_headers(case, &headers);
                    grpc_record.push(json!({"name":case["name"],"code":status.code() as i32,"message":status.message(),"headers_hex":header_json(&headers),"status_metadata_hex":header_json(&status.metadata().clone().into_headers())}));
                    assert_eq!(
                        i64::from(status.code() as i32),
                        grpc["code"].as_i64().unwrap(),
                        "{} gRPC code: {status}",
                        case["name"]
                    );
                    assert_eq!(
                        strip_go_frames(status.message()),
                        strip_go_frames(grpc["message"].as_str().unwrap_or_default()),
                        "{} gRPC message",
                        case["name"]
                    );
                }
            }
        }
        if !case["transports"]["rest"].is_null() {
            let path = case["rest_path"].as_str().unwrap().to_owned();
            let (status, headers, body) = tokio::task::spawn_blocking(move || {
                http_request(rest_addr, "GET", &path, Some(height), &[])
            })
            .await
            .unwrap();
            let expected = &case["transports"]["rest"];
            assert_eq!(
                i64::from(status),
                expected["status"].as_i64().unwrap(),
                "{} REST status",
                case["name"]
            );
            let actual: Value = serde_json::from_slice(&body)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
            assert_eq!(
                canonical_rest(actual.clone()),
                canonical_rest(expected["body"].clone()),
                "{} REST body",
                case["name"]
            );
            rest_record.push(json!({"name":case["name"],"status":status,"headers":headers,"body_hex":hex::encode(&body),"body":actual}));
            if status == 200 {
                assert_eq!(
                    header(&headers, "grpc-metadata-x-cosmos-block-height"),
                    expected["headers"]["Grpc-Metadata-X-Cosmos-Block-Height"][0].as_str(),
                    "{} REST height",
                    case["name"]
                );
            }
        }
    }
    grpc_task.abort();
    rest_task.abort();
    if let Some(directory) = std::env::var_os("MODULE_CLOSURE_CAPTURE") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = serde_json::to_vec(&json!({"scenario":scenario,"chain_id":capture["chain_id"],"tip_height":capture["tip_height"],"query_height":capture["query_height"],"app_hash":capture["app_hash"],"matched_heights":(1..=capture["tip_height"].as_i64().unwrap()).collect::<Vec<_>>(),"abci":abci_record,"comet_rpc":comet_record,"cosmos_grpc":grpc_record,"rest":rest_record})).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&bytes).unwrap();
        std::fs::write(
            directory.join(format!("{scenario}-rust.json.gz")),
            encoder.finish().unwrap(),
        )
        .unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn populated_module_matrix_matches_real_transports_and_proofs() {
    let path = devnet_conformance::scenario_dir("")
        .join("../../mainnet-rpc/module-closure/campaigns.json");
    let scenarios: Vec<String> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!scenarios.is_empty());
    for scenario in scenarios {
        verify_scenario(&scenario).await;
    }
}
