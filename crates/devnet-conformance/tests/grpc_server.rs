//! End-to-end transport test: the `empty` scenario is replayed into an app
//! served by the gRPC query server and the REST API over real sockets; a
//! sample of the Go-recorded corpus is answered through them (tonic client,
//! `x-cosmos-block-height` metadata; HTTP/1.1 GET) and must match Go.

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
        [
            "/cosmos.bank.",
            "/cosmos.auth.",
            "/cosmos.staking.",
            "/cosmos.gov.",
            "/cosmos.distribution.",
        ]
        .iter()
        .any(|p| c.path.starts_with(p))
            && !c.prove
            && !needs_consensus(&c.path)
            && c.height >= 0
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
