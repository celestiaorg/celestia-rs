//! Current-source v10.5.0 Simulate statuses through actual gRPC sockets.
//! Go diagnostic source frames are the one documented message deviation.

#![allow(clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

#[derive(Deserialize)]
struct Metadata {
    source_sha256: BTreeMap<String, String>,
    artifacts_sha256: BTreeMap<String, String>,
}

#[test]
fn current_release_fixture_identity() -> anyhow::Result<()> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repository.join("conformance/mainnet-rpc/simulation");
    let metadata: Metadata = serde_json::from_slice(&std::fs::read(root.join("metadata.json"))?)?;
    for (name, expected) in &metadata.source_sha256 {
        assert_eq!(
            hex::encode(hash::sha256(&std::fs::read(repository.join(name))?)),
            *expected,
            "{name}"
        );
    }
    for (name, expected) in &metadata.artifacts_sha256 {
        assert_eq!(
            hex::encode(hash::sha256(&std::fs::read(root.join(name))?)),
            *expected,
            "{name}"
        );
    }
    let build: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("oracle-build.json"))?)?;
    assert_eq!(
        build["source_commit"],
        "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    );
    assert_eq!(build["build_tags"], serde_json::json!(["ledger"]));
    assert_eq!(build["compiler"], "go version go1.26.9 linux/amd64");
    Ok(())
}

#[derive(Deserialize)]
struct Case {
    name: String,
    phase: String,
    request_hex: String,
    #[serde(default)]
    height: String,
    code: i32,
    message: String,
    details: Vec<serde_json::Value>,
    headers: Option<BTreeMap<String, Vec<String>>>,
    trailers: Option<BTreeMap<String, Vec<String>>>,
    response_hex: String,
}

fn height(values: Option<&BTreeMap<String, Vec<String>>>) -> Option<&str> {
    values?
        .get(api_server::BLOCK_HEIGHT_HEADER)?
        .first()
        .map(String::as_str)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_release_simulation_errors() -> anyhow::Result<()> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/mainnet-rpc/simulation");
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(root.join("go-errors.json"))?)?;
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("go-state.json"))?)?;
    assert_eq!(
        cases.len(),
        14,
        "review the behavior inventory when adding cases"
    );
    let mut app = app::App::new();
    app.set_pruning(app::PruningOptions::nothing());
    // Reuse the public inputs only; the expected errors above were freshly
    // recorded from v10.5.0. Historical answers are not relabelled current.
    let replay = devnet_conformance::replay::replay_into("empty", &mut app, 0, None)?;
    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    let info = app.info();
    assert_eq!(
        info.last_block_height,
        state["height"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("fixture height missing"))?
    );
    assert_eq!(
        hex::encode_upper(&info.last_block_app_hash),
        state["app_hash"]
    );
    assert_eq!(app.chain_id(), state["chain_id"]);
    let app = Arc::new(app::AbciApp::new(app));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(api_server::serve_grpc_on(listener, app));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    for case in &cases {
        assert!(
            case.details.is_empty(),
            "{}: fixture contains status details; extend comparison",
            case.name
        );
        client.ready().await?;
        let mut request = tonic::Request::new(hex::decode(&case.request_hex)?);
        if !case.height.is_empty() {
            request
                .metadata_mut()
                .insert(api_server::BLOCK_HEIGHT_HEADER, case.height.parse()?);
        }
        let path = "/cosmos.tx.v1beta1.Service/Simulate".parse::<http::uri::PathAndQuery>()?;
        let actual = client.unary(request, path, api_server::RawCodec).await;
        let expected_height =
            height(case.headers.as_ref()).or_else(|| height(case.trailers.as_ref()));
        match actual {
            Ok(response) => {
                assert_eq!(case.code, 0, "{} ({})", case.name, case.phase);
                assert_eq!(
                    hex::encode(response.get_ref()),
                    case.response_hex,
                    "{}",
                    case.name
                );
                assert_eq!(
                    response
                        .metadata()
                        .get(api_server::BLOCK_HEIGHT_HEADER)
                        .and_then(|v| v.to_str().ok()),
                    expected_height,
                    "{}",
                    case.name
                );
            }
            Err(status) => {
                assert_eq!(
                    status.code() as i32,
                    case.code,
                    "{} ({})",
                    case.name,
                    case.phase
                );
                assert_eq!(
                    status.message(),
                    devnet_conformance::querycorpus::strip_go_frames(&case.message),
                    "{}",
                    case.name
                );
                assert!(
                    status.details().is_empty(),
                    "{}: unexpected protobuf status details",
                    case.name
                );
                assert_eq!(
                    status
                        .metadata()
                        .get(api_server::BLOCK_HEIGHT_HEADER)
                        .and_then(|v| v.to_str().ok()),
                    expected_height,
                    "{}",
                    case.name
                );
            }
        }
    }
    server.abort();
    Ok(())
}
