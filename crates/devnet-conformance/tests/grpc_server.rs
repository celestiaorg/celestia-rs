//! End-to-end transport test: the `empty` scenario is replayed into an app
//! served by the gRPC query server and the REST API over real sockets; the
//! Go-recorded gRPC-method corpus cases (tonic client,
//! `x-cosmos-block-height` metadata) and a sample of the REST cases
//! (HTTP/1.1 GET) are answered through them and must match Go.

use std::io::{Read, Write};
use std::sync::Arc;

use devnet_conformance::querycorpus::{load, needs_consensus};

fn app_for(scenario: &str) -> anyhow::Result<Arc<app::AbciApp>> {
    let mut app = app::App::new();
    app.set_pruning(app::PruningOptions::nothing());
    let r = devnet_conformance::replay::replay_into(scenario, &mut app, 0, None)?;
    anyhow::ensure!(r.errors.is_empty() && r.first_mismatch.is_none());
    Ok(Arc::new(app::AbciApp::new(app)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grpc_server_answers_like_go() -> anyhow::Result<()> {
    let app = app_for("empty")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(api_server::serve_grpc_on(listener, app));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    let corpus = load("empty")?;
    let mut checked = 0;
    for case in corpus.abci.iter().filter(|c| {
        celestia_proto::services::method(&c.path).is_some()
            && !c.prove
            && !needs_consensus(&c.path)
            && c.height >= 0
            // Go answers it on the gRPC server only (checked below).
            && !c.path.starts_with(grpc_query::reflection_extra::V2ALPHA1)
    }) {
        client.ready().await?;
        let mut req = tonic::Request::new(hex::decode(&case.data)?);
        if case.height != 0 {
            req.metadata_mut().insert(
                api_server::BLOCK_HEIGHT_HEADER,
                case.height.to_string().parse()?,
            );
        }
        let path: http::uri::PathAndQuery = case.path.parse()?;
        let res = client.unary(req, path, api_server::RawCodec).await;
        match res {
            Ok(r) => {
                assert_eq!(case.response.code, 0, "{} {}", case.path, case.data);
                let canon = devnet_conformance::querycorpus::canonical_value;
                assert_eq!(
                    canon(&case.path, &hex::encode(r.get_ref())),
                    canon(&case.path, &case.response.value),
                    "{} {}",
                    case.path,
                    case.data
                );
                let h = r
                    .metadata()
                    .get(api_server::BLOCK_HEIGHT_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                assert_eq!(h, Some(case.response.height.to_string()), "{}", case.path);
            }
            Err(status) => {
                assert_ne!(
                    case.response.code, 0,
                    "{} {}: {status}",
                    case.path, case.data
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 500, "only {checked} cases");
    Ok(())
}

/// `cosmos.base.reflection.v2alpha1` (Go registers it on the gRPC server
/// only): every method's answer must equal Go's
/// (`conformance/querycorpus/reflection-v2alpha1.json`, recorded from a Go
/// node by `record.sh`), lists Go fills from map iteration compared in a
/// canonical order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reflection_v2alpha1_answers_like_go() -> anyhow::Result<()> {
    let app = app_for("empty")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(api_server::serve_grpc_on(listener, app));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/querycorpus/reflection-v2alpha1.json");
    let go: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(path)?)?;
    anyhow::ensure!(go.len() == 6, "{} methods", go.len());
    for (method, want) in &go {
        client.ready().await?;
        let p: http::uri::PathAndQuery =
            format!("{}{method}", grpc_query::reflection_extra::V2ALPHA1).parse()?;
        let r = client
            .unary(tonic::Request::new(Vec::new()), p, api_server::RawCodec)
            .await?;
        let canon = |b: &[u8]| grpc_query::reflection_extra::canonical_v2alpha1(method, b);
        assert!(
            r.metadata().get(api_server::BLOCK_HEIGHT_HEADER).is_none(),
            "{method}: no height header outside the query router"
        );
        let got = canon(r.get_ref()).ok_or_else(|| anyhow::anyhow!("{method}: decode"))?;
        assert_eq!(Some(got), canon(&hex::decode(want)?), "{method}");
    }
    // A malformed request fails in grpc-go's codec.
    client.ready().await?;
    let p: http::uri::PathAndQuery = format!(
        "{}GetChainDescriptor",
        grpc_query::reflection_extra::V2ALPHA1
    )
    .parse()?;
    let err = client
        .unary(tonic::Request::new(vec![0xff]), p, api_server::RawCodec)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("malformed request accepted"))?;
    assert_eq!(err.code(), tonic::Code::Internal);
    assert_eq!(
        err.message(),
        "grpc: error unmarshalling request: unexpected EOF"
    );
    Ok(())
}

fn http_get(addr: std::net::SocketAddr, url: &str) -> anyhow::Result<(u16, serde_json::Value)> {
    let mut s = std::net::TcpStream::connect(addr)?;
    write!(
        s,
        "GET {url} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("no header end"))?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("no status"))?
        .parse()?;
    Ok((status, serde_json::from_str(body)?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rest_server_answers_like_go() -> anyhow::Result<()> {
    let app = app_for("empty")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(api_server::serve_rest_on(listener, app));
    let corpus = load("empty")?;
    let cases: Vec<_> = corpus
        .rest
        .iter()
        .filter(|c| c.method.starts_with("/cosmos.bank.") && c.height.is_none())
        .cloned()
        .collect();
    assert!(!cases.is_empty());
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        for case in &cases {
            let (status, body) = http_get(addr, &case.url)?;
            assert_eq!(status, case.status, "{}", case.url);
            assert_eq!(body, case.body, "{}", case.url);
        }
        Ok(())
    })
    .await??;
    Ok(())
}

/// `BroadcastTx` over gRPC: celestia-core answers `broadcast_tx_sync` only
/// after running `CheckTx` on the app, so the query must not hold the app
/// lock while it waits (Go runs them concurrently).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broadcast_tx_reaches_check_tx() -> anyhow::Result<()> {
    use abci_server::Application as _;
    use celestia_proto::celestia_core_proto::abci::{RequestCheckTx, request, response};
    use prost::Message as _;
    use std::io::{BufRead as _, BufReader};

    let app = app_for("empty")?;
    // A celestia-core stand-in: CheckTx on the app, then the RPC result.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let rpc_addr = listener.local_addr()?;
    let core = Arc::clone(&app);
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let Ok(clone) = s.try_clone() else { continue };
            let mut r = BufReader::new(clone);
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            if std::io::Read::read_exact(&mut r, &mut body).is_err() {
                continue;
            }
            let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let tx = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                req["params"]["tx"].as_str().unwrap_or_default(),
            )
            .unwrap_or_default();
            let out = core.handle(request::Value::CheckTx(RequestCheckTx {
                tx: tx.clone(),
                ..Default::default()
            }));
            let response::Value::CheckTx(res) = out else {
                // celestia-core reports an ABCI error as an RPC error.
                let out = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "error": {
                    "code": -32603, "message": "Internal error", "data": format!("{out:?}")}})
                .to_string();
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
                continue;
            };
            let out = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": {
                "code": res.code, "data": "", "log": res.log, "codespace": res.codespace,
                "hash": hex::encode_upper(hash::sha256(&tx)),
            }})
            .to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
        }
    });
    app::comet::set_rpc_address(Some(format!("tcp://{rpc_addr}")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(api_server::serve_grpc_on(listener, Arc::clone(&app)));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await?;
    // A tx that decodes but carries no message: CheckTx rejects it with
    // `ErrInvalidRequest`.
    let raw = celestia_proto::cosmos::tx::v1beta1::TxRaw {
        body_bytes: celestia_proto::cosmos::tx::v1beta1::TxBody {
            memo: "no messages".into(),
            ..Default::default()
        }
        .encode_to_vec(),
        auth_info_bytes: celestia_proto::cosmos::tx::v1beta1::AuthInfo::default().encode_to_vec(),
        signatures: Vec::new(),
    }
    .encode_to_vec();
    let req = celestia_proto::cosmos::tx::v1beta1::BroadcastTxRequest {
        tx_bytes: raw.clone(),
        mode: 2,
    };
    let path: http::uri::PathAndQuery = "/cosmos.tx.v1beta1.Service/BroadcastTx".parse()?;
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client.unary(
            tonic::Request::new(req.encode_to_vec()),
            path,
            api_server::RawCodec,
        ),
    )
    .await??;
    let resp =
        celestia_proto::cosmos::tx::v1beta1::BroadcastTxResponse::decode(res.get_ref().as_slice())?;
    let tx = resp.tx_response.unwrap_or_default();
    assert_eq!((tx.codespace.as_str(), tx.code), ("sdk", 18), "{tx:?}");
    assert_eq!(tx.txhash, hex::encode_upper(hash::sha256(&raw)));
    Ok(())
}
