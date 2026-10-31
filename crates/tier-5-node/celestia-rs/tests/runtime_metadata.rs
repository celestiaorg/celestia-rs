//! Production node listeners must report this node's own identity consistently.
//! This is a runtime-wiring regression, not current-release Go parity evidence:
//! the single-validator genesis is the existing historical node-test fixture.

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
use std::time::{Duration, Instant};

use celestia_proto::cosmos::base::tendermint::v1beta1::GetNodeInfoResponse;
use prost::Message as _;
use serde_json::{Value, json};

struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(addr: SocketAddr, path: &str) -> anyhow::Result<(String, Value)> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    write!(
        s,
        "GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut bytes = Vec::new();
    s.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
    let text = String::from_utf8(bytes)?;
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("no HTTP headers"))?;
    anyhow::ensure!(
        head.lines().next().unwrap_or_default().contains(" 200 "),
        "{text}"
    );
    Ok((head.to_owned(), serde_json::from_str(body)?))
}

fn fixture(home: &Path, rpc: SocketAddr, grpc: SocketAddr, rest: SocketAddr, p2p: SocketAddr) {
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home/config");
    for name in ["genesis.json", "priv_validator_key.json"] {
        std::fs::copy(source.join(name), cfg.join(name)).unwrap();
    }
    std::fs::write(
        home.join("data/priv_validator_state.json"),
        r#"{"height":"0","round":0,"step":0}"#,
    )
    .unwrap();
    std::fs::write(cfg.join("config.toml"), format!(
        "moniker = \"runtime-metadata\"\n[rpc]\nladdr = \"tcp://{rpc}\"\ngrpc_laddr = \"\"\n[p2p]\nladdr = \"tcp://{p2p}\"\npex = false\naddr_book_strict = false\n[tx_index]\nindexer = \"kv\"\n"
    )).unwrap();
    std::fs::write(cfg.join("app.toml"), format!(
        "[grpc]\nenable = true\naddress = \"{grpc}\"\n[api]\nenable = true\naddress = \"tcp://{rest}\"\n"
    )).unwrap();
}

fn assert_node_info(v: &Value) {
    assert_eq!(v["network"], "rs-node-test");
    assert_eq!(v["version"], celestia_rs::version::node_version());
    let version = v["version"].as_str().unwrap();
    assert!(version.starts_with("0.38."));
    assert!(version.contains(&format!(
        "celestia-rs.{}",
        celestia_rs::version::SOFTWARE_VERSION
    )));
    assert_eq!(
        v["protocol_version"],
        json!({"p2p":"8", "block":"11", "app":"10"})
    );
    assert_eq!(v["moniker"], "runtime-metadata");
    assert_eq!(v["other"]["tx_index"], "on");
}

fn assert_fields(actual: &Value, recorded: &Value, label: &str) {
    let keys = |v: &Value| {
        let mut fields = v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        fields.sort();
        fields
    };
    assert_eq!(keys(actual), keys(recorded), "{label}");
}

fn assert_scalar_type(actual: &Value, recorded: &Value, label: &str) {
    let kind = |v: &Value| match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    assert_eq!(kind(actual), kind(recorded), "{label}");
}

fn assert_consensus_schema(rpc: SocketAddr) -> anyhow::Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/node-metadata/v10.5.0/consensus.json"
    )))?;
    let info: Value =
        serde_json::from_str(fixture["responses"]["abci_info"]["body"].as_str().unwrap())?;
    assert_eq!(
        info["result"]["response"]["version"],
        app::COMPATIBILITY_VERSION
    );
    assert_eq!(
        fixture["provenance"]["application_commit"],
        "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    );
    for method in ["consensus_state", "dump_consensus_state"] {
        let recorded: Value =
            serde_json::from_str(fixture["responses"][method]["body"].as_str().unwrap())?;
        let (_, actual) = get(rpc, &format!("/{method}"))?;
        assert_fields(&actual["result"], &recorded["result"], method);
        let actual = &actual["result"]["round_state"];
        let recorded = &recorded["result"]["round_state"];
        assert_fields(actual, recorded, method);
        // Consensus advances independently between requests. Keep exact field
        // names and string-vs-number encodings while leaving only the values
        // (height, round, step, timestamps, hashes, votes) unconstrained here.
        let fields: &[&str] = if method == "consensus_state" {
            &[
                "height/round/step",
                "start_time",
                "proposal_block_hash",
                "locked_block_hash",
                "valid_block_hash",
            ]
        } else {
            &[
                "height",
                "round",
                "step",
                "start_time",
                "commit_time",
                "locked_round",
                "valid_round",
                "commit_round",
                "triggered_timeout_precommit",
            ]
        };
        for &field in fields {
            assert_scalar_type(&actual[field], &recorded[field], field);
        }
        let vote_field = if method == "consensus_state" {
            "height_vote_set"
        } else {
            "votes"
        };
        let exemplar = &recorded[vote_field].as_array().unwrap()[0];
        for votes in actual[vote_field].as_array().unwrap() {
            assert_fields(votes, exemplar, vote_field);
            for field in [
                "round",
                "prevotes",
                "prevotes_bit_array",
                "precommits",
                "precommits_bit_array",
            ] {
                assert_scalar_type(&votes[field], &exemplar[field], field);
            }
        }
        if method == "consensus_state" {
            assert_fields(&actual["proposer"], &recorded["proposer"], "proposer");
            assert_scalar_type(
                &actual["proposer"]["address"],
                &recorded["proposer"]["address"],
                "proposer address",
            );
            assert_scalar_type(
                &actual["proposer"]["index"],
                &recorded["proposer"]["index"],
                "proposer index",
            );
        } else {
            for field in ["validators", "last_validators"] {
                assert_fields(&actual[field], &recorded[field], field);
                let exemplar = &recorded[field]["validators"].as_array().unwrap()[0];
                for validator in actual[field]["validators"].as_array().unwrap() {
                    assert_fields(validator, exemplar, "validator");
                    for key in ["address", "voting_power", "proposer_priority"] {
                        assert_scalar_type(&validator[key], &exemplar[key], key);
                    }
                    assert_fields(
                        &validator["pub_key"],
                        &exemplar["pub_key"],
                        "validator pubkey",
                    );
                    assert_eq!(validator["pub_key"]["type"], exemplar["pub_key"]["type"]);
                }
            }
        }
    }
    Ok(())
}

fn assert_node_schema(actual: &Value) -> anyhow::Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/node-metadata/v10.5.0/consensus.json"
    )))?;
    let status: Value =
        serde_json::from_str(fixture["responses"]["status"]["body"].as_str().unwrap())?;
    let recorded = &status["result"]["node_info"];
    assert_fields(actual, recorded, "status node_info");
    assert_fields(
        &actual["protocol_version"],
        &recorded["protocol_version"],
        "status protocol_version",
    );
    assert_fields(&actual["other"], &recorded["other"], "status other");
    for field in [
        "id",
        "listen_addr",
        "network",
        "version",
        "channels",
        "moniker",
    ] {
        assert_scalar_type(&actual[field], &recorded[field], field);
    }
    for field in ["p2p", "block", "app"] {
        assert_scalar_type(
            &actual["protocol_version"][field],
            &recorded["protocol_version"][field],
            field,
        );
    }
    Ok(())
}

fn assert_net_info_schema(actual: &Value) -> anyhow::Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/node-metadata/v10.5.0/consensus.json"
    )))?;
    let net: Value =
        serde_json::from_str(fixture["responses"]["net_info"]["body"].as_str().unwrap())?;
    let recorded = &net["result"];
    assert_fields(actual, recorded, "net_info");
    for field in ["listening", "listeners", "n_peers", "peers"] {
        assert_scalar_type(&actual[field], &recorded[field], field);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_listeners_report_build_and_compatibility_separately() -> anyhow::Result<()> {
    // Hold all chosen ports until startup; the bind-close-start window is small
    // and any collision fails the readiness checks instead of using a default.
    let sockets: Vec<_> = (0..4)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let addrs: Vec<_> = sockets.iter().map(|s| s.local_addr().unwrap()).collect();
    let (rpc, grpc, rest, p2p) = (addrs[0], addrs[1], addrs[2], addrs[3]);
    let dir = tempfile::tempdir()?;
    fixture(dir.path(), rpc, grpc, rest, p2p);
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
    let status = loop {
        anyhow::ensure!(
            node.0.try_wait()?.is_none(),
            "node exited: {}",
            std::fs::read_to_string(&log_path)?
        );
        if let Ok((_, status)) = get(rpc, "/status")
            && status["result"]["sync_info"]["latest_block_height"]
                .as_str()
                .unwrap_or("0")
                != "0"
        {
            break status;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "node not ready: {}",
            std::fs::read_to_string(&log_path)?
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_node_info(&status["result"]["node_info"]);
    assert_node_schema(&status["result"]["node_info"])?;
    assert_consensus_schema(rpc)?;
    let (_, info) = get(rpc, "/abci_info")?;
    assert_eq!(info["result"]["response"]["data"], app::APPLICATION_NAME);
    assert_eq!(
        info["result"]["response"]["version"],
        app::COMPATIBILITY_VERSION
    );
    assert_eq!(info["result"]["response"]["app_version"], "10");
    let (_, query) = get(rpc, "/abci_query?path=%22/app/version%22")?;
    assert_eq!(query["result"]["response"]["code"], 0);
    assert_eq!(
        query["result"]["response"]["value"],
        app::comet::to_b64(app::COMPATIBILITY_VERSION.as_bytes())
    );
    let (_, net) = get(rpc, "/net_info")?;
    assert_net_info_schema(&net["result"])?;
    assert_eq!(net["result"]["n_peers"], "0");
    assert_eq!(net["result"]["peers"], json!([]));
    assert_eq!(net["result"]["listening"], true);

    // Read the production P2P listener's authenticated NodeInfo, independently
    // of the RPC NodeInfo construction. The two used to disagree on app version
    // and operator metadata (hard-coded app=0/index=off/RPC address empty).
    let probe_key = p2p::NodeKey::generate();
    let probe_info = p2p::NodeInfo {
        id: probe_key.id().to_string(),
        protocol_version: p2p::ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 10,
        },
        listen_addr: "tcp://127.0.0.1:1".into(),
        network: "rs-node-test".into(),
        version: "metadata-probe/1".into(),
        moniker: "metadata-probe".into(),
        channels: vec![0x31, 0x32, 0x50],
        ..p2p::NodeInfo::default()
    };
    let probe = p2p::transport::Transport::new(
        probe_key,
        probe_info,
        p2p::transport::TransportConfig::default(),
    );
    let address = p2p::netaddress::NetAddress::parse(&format!(
        "{}@{p2p}",
        status["result"]["node_info"]["id"].as_str().unwrap()
    ))?;
    let connection = probe.dial(&address).await?;
    assert_eq!(
        connection.node_info.version,
        celestia_rs::version::node_version()
    );
    assert_eq!(
        connection.node_info.protocol_version,
        p2p::ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 10
        }
    );
    assert_eq!(connection.node_info.other.tx_index, "on");
    assert_eq!(
        connection.node_info.other.rpc_address,
        format!("tcp://{rpc}")
    );

    let channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await?;
    let response = client
        .unary(
            tonic::Request::new(Vec::new()),
            "/cosmos.base.tendermint.v1beta1.Service/GetNodeInfo"
                .parse::<axum::http::uri::PathAndQuery>()?,
            api_server::RawCodec,
        )
        .await?;
    let height = response
        .metadata()
        .get(api_server::BLOCK_HEIGHT_HEADER)
        .unwrap()
        .to_str()?
        .parse::<u64>()?;
    assert!(height > 0);
    let grpc_info = GetNodeInfoResponse::decode(response.get_ref().as_slice())?;
    let node_info = grpc_info.default_node_info.unwrap();
    assert_eq!(node_info.version, celestia_rs::version::node_version());
    assert_eq!(node_info.network, "rs-node-test");
    assert_eq!(node_info.moniker, "runtime-metadata");
    assert_eq!(node_info.protocol_version.unwrap().app, 10);
    let build = grpc_info.application_version.unwrap();
    assert_eq!(build.name, app::APPLICATION_NAME);
    assert_eq!(build.app_name, "celestia-rs");
    assert_eq!(build.version, celestia_rs::version::SOFTWARE_VERSION);
    assert!(!build.git_commit.is_empty());
    assert!(build.go_version.starts_with("rustc "));
    assert!(!build.build_tags.is_empty());

    let (_, rest_info) = get(rest, "/cosmos/base/tendermint/v1beta1/node_info")?;
    assert_node_info(&rest_info["default_node_info"]);
    assert_eq!(rest_info["application_version"]["name"], build.name);
    assert_eq!(rest_info["application_version"]["version"], build.version);
    assert_eq!(
        rest_info["application_version"]["git_commit"],
        build.git_commit
    );
    assert_eq!(
        rest_info["application_version"]["go_version"],
        build.go_version
    );
    assert_eq!(
        rest_info["application_version"]["build_tags"],
        build.build_tags
    );

    let cli = Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
        .args(["version", "--long"])
        .output()?;
    anyhow::ensure!(cli.status.success());
    let cli = String::from_utf8(cli.stdout)?;
    assert!(cli.contains(&format!("version: {}\n", build.version)));
    assert!(cli.contains(&format!("commit: {}\n", build.git_commit)));
    assert!(cli.contains(&format!("build_profile: {}\n", build.build_tags)));
    assert!(cli.contains(&format!("toolchain: {}\n", build.go_version)));
    assert!(cli.contains(&format!(
        "compatibility_target: celestia-app v{}, celestia-core v{}",
        app::COMPATIBILITY_VERSION,
        celestia_rs::version::CORE_COMPATIBILITY_VERSION
    )));
    Ok(())
}
