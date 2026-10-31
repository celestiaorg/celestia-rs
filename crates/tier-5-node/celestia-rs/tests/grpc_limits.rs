//! Production listener wiring, using the historical single-validator node-test
//! genesis. This is a runtime regression; fresh current-release Go evidence is
//! recorded separately under conformance/grpc-limits.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::result_large_err,
    clippy::too_many_lines,
    clippy::unwrap_used
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Node(Child);
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn ready(addr: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(100)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    if write!(stream, "GET /status HTTP/1.0\r\nHost: {addr}\r\n\r\n").is_err() {
        return false;
    }
    let mut text = String::new();
    if stream.take(65536).read_to_string(&mut text).is_err() {
        return false;
    }
    let Some((_, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    v["result"]["sync_info"]["latest_block_height"]
        .as_str()
        .is_some_and(|h| h != "0")
}
fn setup(home: &Path, rpc: SocketAddr, app: SocketAddr, core: SocketAddr, p2p: SocketAddr) {
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home/config");
    for file in ["genesis.json", "priv_validator_key.json"] {
        std::fs::copy(fixtures.join(file), cfg.join(file)).unwrap();
    }
    std::fs::write(
        home.join("data/priv_validator_state.json"),
        r#"{"height":"0","round":0,"step":0}"#,
    )
    .unwrap();
    std::fs::write(cfg.join("config.toml"), format!(
        "moniker = \"grpc-limits\"\n[rpc]\nladdr = \"tcp://{rpc}\"\ngrpc_laddr = \"tcp://{core}\"\ngrpc_max_open_connections = 1\n[p2p]\nladdr = \"tcp://{p2p}\"\npex = false\naddr_book_strict = false\n[tx_index]\nindexer = \"kv\"\n"
    )).unwrap();
    std::fs::write(cfg.join("app.toml"), format!(
        "[grpc]\nenable = true\naddress = \"{app}\"\nmax-recv-msg-size = \"64\"\nmax-send-msg-size = \"128\"\n[api]\nenable = false\n"
    )).unwrap();
}
async fn channel(addr: SocketAddr) -> Result<tonic::transport::Channel, tonic::transport::Error> {
    tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_timeout(Duration::from_millis(500))
        .connect()
        .await
}
fn padded(size: usize) -> Vec<u8> {
    for payload in (size.saturating_sub(6)..=size.saturating_sub(3)).rev() {
        let mut bytes = vec![0xa2, 0x06];
        let mut length = payload;
        while length >= 128 {
            bytes.push(u8::try_from(length & 127).unwrap() | 128);
            length >>= 7;
        }
        bytes.push(u8::try_from(length).unwrap());
        if bytes.len() + payload == size {
            bytes.resize(size, b'x');
            return bytes;
        }
    }
    panic!("no padding for request size {size}")
}
async fn unary(
    client: &mut tonic::client::Grpc<tonic::transport::Channel>,
    method: &'static str,
    data: Vec<u8>,
) -> Result<tonic::Response<Vec<u8>>, tonic::Status> {
    client.ready().await.unwrap();
    client
        .unary(
            tonic::Request::new(data),
            method.parse().unwrap(),
            api_server::RawCodec,
        )
        .await
}
async fn reflection_error(
    client: &mut tonic::client::Grpc<tonic::transport::Channel>,
    data: Vec<u8>,
) -> tonic::Status {
    client.ready().await.unwrap();
    let response = client
        .streaming(
            tonic::Request::new(futures_util::stream::iter([data])),
            "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"
                .parse()
                .unwrap(),
            api_server::RawCodec,
        )
        .await;
    match response {
        Err(status) => status,
        Ok(response) => response
            .into_inner()
            .message()
            .await
            .expect_err("reflection request unexpectedly produced a response"),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_app_limits_reach_module_and_block_api_and_core_capacity_recovers()
-> anyhow::Result<()> {
    let sockets: Vec<_> = (0..4)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let addrs: Vec<_> = sockets.iter().map(|s| s.local_addr().unwrap()).collect();
    let (rpc, app, core, p2p) = (addrs[0], addrs[1], addrs[2], addrs[3]);
    let dir = tempfile::tempdir()?;
    setup(dir.path(), rpc, app, core, p2p);
    let log_path = dir.path().join("node.log");
    let log = std::fs::File::create(&log_path)?;
    drop(sockets);
    let mut node = Node(
        Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
            .args(["--home", dir.path().to_str().unwrap(), "start"])
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    while !ready(rpc) {
        anyhow::ensure!(
            node.0.try_wait()?.is_none(),
            "node exited: {}",
            std::fs::read_to_string(&log_path)?
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "node not ready: {}",
            std::fs::read_to_string(&log_path)?
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut sdk = tonic::client::Grpc::new(channel(app).await?);
    for size in [63, 64, 65, 3] {
        let response = unary(
            &mut sdk,
            "/cosmos.base.node.v1beta1.Service/Config",
            padded(size),
        )
        .await;
        if size <= 64 {
            let response = response?;
            assert!(!response.get_ref().is_empty());
            assert!(
                response
                    .metadata()
                    .get(api_server::BLOCK_HEIGHT_HEADER)
                    .is_some()
            );
        } else {
            let status = response.unwrap_err();
            assert_eq!(status.code(), tonic::Code::ResourceExhausted);
            assert_eq!(
                status.message(),
                "grpc: received message larger than max (65 vs. 64)"
            );
        }
    }
    // Reflection is served by a distinct streaming router on the production
    // SDK listener. It must use the same configured receive limit: the
    // 64-byte unknown-only protobuf reaches reflection and is rejected by the
    // handler, while the 65-byte request is rejected at the transport boundary.
    let status = reflection_error(&mut sdk, padded(64)).await;
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert_eq!(status.message(), "invalid MessageRequest: <nil>");
    let status = reflection_error(&mut sdk, padded(65)).await;
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    assert_eq!(
        status.message(),
        "grpc: received message larger than max (65 vs. 64)"
    );
    for (method, expected) in [
        ("/missing.Service/Query", "unknown service missing.Service"),
        (
            "/cosmos.bank.v1beta1.Query/Missing",
            "unknown method Missing for service cosmos.bank.v1beta1.Query",
        ),
        (
            "/tendermint.rpc.grpc.BlockAPI/Missing",
            "unknown method Missing for service tendermint.rpc.grpc.BlockAPI",
        ),
        (
            "/tendermint.rpc.grpc.BroadcastAPI/Ping",
            "unknown service tendermint.rpc.grpc.BroadcastAPI",
        ),
    ] {
        let status = unary(&mut sdk, method, padded(65)).await.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unimplemented);
        assert_eq!(status.message(), expected);
    }
    let mut core_client = tonic::client::Grpc::new(channel(core).await?);
    // The standalone listener retains 4 MiB receive limits, independently of SDK.
    unary(
        &mut core_client,
        "/tendermint.rpc.grpc.BroadcastAPI/Ping",
        padded(65),
    )
    .await?;
    for size in [(4 << 20) - 1, 4 << 20, (4 << 20) + 1] {
        let result = unary(
            &mut core_client,
            "/tendermint.rpc.grpc.BroadcastAPI/Ping",
            padded(size),
        )
        .await;
        if size <= 4 << 20 {
            result?;
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code(), tonic::Code::ResourceExhausted);
            assert_eq!(
                error.message(),
                "grpc: received message larger than max (4194305 vs. 4194304)"
            );
        }
    }

    let core_status = unary(
        &mut core_client,
        "/tendermint.rpc.grpc.BlockAPI/Status",
        Vec::new(),
    )
    .await?;
    let expected_length = core_status.get_ref().len();
    let error = unary(&mut sdk, "/tendermint.rpc.grpc.BlockAPI/Status", Vec::new())
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
    assert_eq!(
        error.message(),
        format!("grpc: trying to send message larger than max ({expected_length} vs. 128)")
    );
    core_client.ready().await?;
    let mut parts = core_client
        .server_streaming(
            tonic::Request::new(vec![8, 1]),
            "/tendermint.rpc.grpc.BlockAPI/BlockByHeight"
                .parse()
                .unwrap(),
            api_server::RawCodec,
        )
        .await?
        .into_inner();
    let part_length = parts.message().await?.unwrap().len();
    drop(parts);
    sdk.ready().await?;
    let error = sdk
        .server_streaming(
            tonic::Request::new(vec![8, 1]),
            "/tendermint.rpc.grpc.BlockAPI/BlockByHeight"
                .parse()
                .unwrap(),
            api_server::RawCodec,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
    assert_eq!(
        error.message(),
        format!("trying to send message larger than max ({part_length} vs. 128)")
    );
    // Saturating core connections must not consume SDK capacity. Closing the
    // first HTTP/2 channel makes the core slot usable again.
    // A TCP connect may complete into the kernel backlog before accept. Only
    // an actual HTTP/2 RPC distinguishes accepted capacity from a queued socket.
    let blocked_call = tokio::time::timeout(Duration::from_millis(300), async {
        let mut blocked = tonic::client::Grpc::new(channel(core).await?);
        unary(
            &mut blocked,
            "/tendermint.rpc.grpc.BroadcastAPI/Ping",
            Vec::new(),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    })
    .await;
    assert!(
        blocked_call.is_err(),
        "a second core RPC unexpectedly acquired capacity"
    );
    unary(
        &mut sdk,
        "/cosmos.base.node.v1beta1.Service/Config",
        Vec::new(),
    )
    .await?;
    drop(core_client);
    let mut recovered = tonic::client::Grpc::new(channel(core).await?);
    tokio::time::timeout(
        Duration::from_secs(5),
        unary(
            &mut recovered,
            "/tendermint.rpc.grpc.BroadcastAPI/Ping",
            Vec::new(),
        ),
    )
    .await??;
    Ok(())
}
