//! Peers learned from a public RPC node's `/net_info`, and the celestia-app
//! release from a node's `/abci_info`.
#![allow(clippy::unwrap_used)]

use node_census::classify::Implementation;
use std::net::IpAddr;

use node_census::crawl::{Observed, Source};
use node_census::rpc::{parse_abci_info, parse_net_info, probe_app_releases, rpc_url};
use serde_json::json;

const A: &str = "d513f98bb4f8859f94bde2efc6bf18d29f732c48";
const B: &str = "acca7837e4eb5f9dc7f5a94ed1d82edda6931ff8";
const C: &str = "12ad7c73c7e1f2460941326937a039139aa78884";

fn peer(
    id: &str,
    version: &str,
    listen: &str,
    remote_ip: &str,
    network: &str,
) -> serde_json::Value {
    json!({
        "node_info": {
            "protocol_version": {"p2p": "8", "block": "11", "app": "6"},
            "id": id,
            "listen_addr": listen,
            "network": network,
            "version": version,
            "moniker": format!("m-{id}"),
        },
        "remote_ip": remote_ip,
    })
}

#[test]
fn reads_peer_node_info_and_dialable_address() {
    let body = json!({"jsonrpc": "2.0", "result": {"peers": [
        peer(A, "0.38.17-celestia-app.10.6.0", "188.40.102.137:15656", "188.40.102.137", "celestia"),
        // Listening on all interfaces: dial the IP the RPC node saw instead.
        peer(B, "0.38.17-celestia-rs.0.1.0-rc.9", "tcp://0.0.0.0:26656", "65.21.1.2", "celestia"),
        // Another chain's peer is ignored.
        peer(C, "0.38.17", "1.2.3.4:26656", "1.2.3.4", "mocha-5"),
    ]}});
    let peers = parse_net_info(&body, "celestia");
    assert_eq!(peers.len(), 2);

    assert_eq!(peers[0].observed.id, A);
    assert_eq!(peers[0].observed.implementation, Implementation::Go);
    assert_eq!(peers[0].observed.app_version, 6);
    assert_eq!(peers[0].observed.app_release.as_deref(), Some("10.6.0"));
    assert_eq!(peers[0].observed.source, Source::Rpc);
    assert_eq!(
        peers[0].addr.as_ref().unwrap().to_string(),
        format!("{A}@188.40.102.137:15656")
    );

    assert_eq!(peers[1].observed.implementation, Implementation::Rust);
    assert_eq!(
        peers[1].addr.as_ref().unwrap().to_string(),
        format!("{B}@65.21.1.2:26656")
    );
}

#[test]
fn malformed_bodies_yield_nothing() {
    assert!(parse_net_info(&json!({}), "celestia").is_empty());
    assert!(
        parse_net_info(
            &json!({"result": {"peers": [{"node_info": {}}]}}),
            "celestia"
        )
        .is_empty()
    );
}

// Recorded `/abci_info` bodies: a Mainnet node (2026-11-27), and the same
// shape with a `v`-prefixed release.
const ABCI_INFO_MAINNET: &str = r#"{"jsonrpc":"2.0","id":-1,"result":{"response":{"data":"celestia-app","version":"10.6.0","app_version":"10","last_block_height":"16293137","last_block_app_hash":"fMHwOkOtRgP07RH71MiA/6Ar4Vmigo5VIgOq21I5GiY=","timeout_info":{"timeout_propose":"3000000000","timeout_propose_delta":"500000000","timeout_prevote":"2000000000","timeout_prevote_delta":"500000000","timeout_precommit":"3000000000","timeout_precommit_delta":"500000000","timeout_commit":"500000000","delayed_precommit_timeout":"2100000000"}}}}"#;
const ABCI_INFO_V_PREFIX: &str = r#"{"jsonrpc":"2.0","id":-1,"result":{"response":{"data":"celestia-app","version":"v11.0.0","app_version":"10","last_block_height":"1","last_block_app_hash":""}}}"#;

#[test]
fn abci_info_yields_the_celestia_app_release() {
    let v: serde_json::Value = serde_json::from_str(ABCI_INFO_MAINNET).unwrap();
    assert_eq!(parse_abci_info(&v).as_deref(), Some("10.6.0"));
    // A leading `v` is dropped so releases group together.
    let v: serde_json::Value = serde_json::from_str(ABCI_INFO_V_PREFIX).unwrap();
    assert_eq!(parse_abci_info(&v).as_deref(), Some("11.0.0"));
}

#[test]
fn abci_info_without_a_usable_version_yields_nothing() {
    for body in [
        json!({}),
        json!({"result": {"response": {}}}),
        json!({"result": {"response": {"version": ""}}}),
        json!({"result": {"response": {"version": 6}}}),
        json!({"result": {"response": {"version": "<script>alert(1)</script>"}}}),
        json!({"result": {"response": {"version": "1".repeat(65)}}}),
        json!({"error": {"code": -32603, "message": "internal"}}),
    ] {
        assert_eq!(parse_abci_info(&body), None, "{body}");
    }
}

#[test]
fn rpc_url_uses_the_node_ip_for_unroutable_hosts() {
    let ip: IpAddr = "65.21.1.2".parse().unwrap();
    assert_eq!(
        rpc_url("tcp://0.0.0.0:26657", ip).as_deref(),
        Some("http://65.21.1.2:26657")
    );
    assert_eq!(
        rpc_url("tcp://127.0.0.1:36657", ip).as_deref(),
        Some("http://65.21.1.2:36657")
    );
    assert_eq!(
        rpc_url("tcp://10.0.0.5:26657", ip).as_deref(),
        Some("http://65.21.1.2:26657")
    );
    // A routable advertised IP is kept.
    assert_eq!(
        rpc_url("tcp://188.40.102.137:26657", ip).as_deref(),
        Some("http://188.40.102.137:26657")
    );
    // Missing address or port: the default port on the node's IP.
    assert_eq!(rpc_url("", ip).as_deref(), Some("http://65.21.1.2:26657"));
    assert_eq!(
        rpc_url("tcp://0.0.0.0", ip).as_deref(),
        Some("http://65.21.1.2:26657")
    );
    let v6: IpAddr = "2a01:4f9::1".parse().unwrap();
    assert_eq!(
        rpc_url("tcp://0.0.0.0:26657", v6).as_deref(),
        Some("http://[2a01:4f9::1]:26657")
    );
}

#[test]
fn rpc_url_rejects_unix_sockets_and_unroutable_node_ips() {
    let ip: IpAddr = "65.21.1.2".parse().unwrap();
    assert_eq!(rpc_url("unix:///var/run/rpc.sock", ip), None);
    assert_eq!(
        rpc_url("tcp://0.0.0.0:26657", "127.0.0.1".parse().unwrap()),
        None
    );
}

#[test]
fn net_info_peers_carry_an_rpc_url() {
    let mut p = peer(A, "0.38.17", "tcp://0.0.0.0:26656", "65.21.1.2", "celestia");
    p["node_info"]["other"] = json!({"tx_index": "on", "rpc_address": "tcp://0.0.0.0:26657"});
    let peers = parse_net_info(&json!({"result": {"peers": [p]}}), "celestia");
    assert_eq!(
        peers[0].observed.rpc_url.as_deref(),
        Some("http://65.21.1.2:26657")
    );
    assert_eq!(peers[0].observed.app_release, None);
}

fn observed(id: &str, rpc_url: Option<&str>) -> Observed {
    Observed {
        id: id.into(),
        version: "0.38.17".into(),
        implementation: Implementation::Go,
        moniker: id.into(),
        app_version: 6,
        source: Source::P2p,
        app_release: None,
        rpc_url: rpc_url.map(Into::into),
    }
}

#[tokio::test]
async fn probing_records_releases_and_ignores_failures() {
    let mut nodes = vec![
        observed(A, Some("http://1.1.1.1:26657")),
        observed(B, Some("http://2.2.2.2:26657")),
        observed(C, None),
        observed("advertised", Some("http://3.3.3.3:26657")),
    ];
    nodes[3].app_release = Some("10.7.0".into());
    let answered = probe_app_releases(&mut nodes, 2, |url: String| {
        if url == "http://1.1.1.1:26657" {
            Ok(serde_json::from_str(ABCI_INFO_MAINNET)?)
        } else if url == "http://3.3.3.3:26657" {
            Ok(json!({"result": {"response": {"version": "10.5.0"}}}))
        } else {
            anyhow::bail!("connection refused")
        }
    })
    .await;
    assert_eq!(answered, 1);
    assert_eq!(nodes[0].app_release.as_deref(), Some("10.6.0"));
    assert_eq!(nodes[1].app_release, None);
    assert_eq!(nodes[2].app_release, None);
    assert_eq!(nodes[3].app_release.as_deref(), Some("10.7.0"));
}

#[test]
fn rpc_url_is_not_published_and_old_snapshots_still_parse() {
    let mut n = observed(A, Some("http://1.1.1.1:26657"));
    let v = serde_json::to_value(&n).unwrap();
    assert!(v.get("rpc_url").is_none());
    assert!(v.get("app_release").is_none());
    n.app_release = Some("6.4.0".into());
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["app_release"], "6.4.0");
    let old = json!({"id": A, "version": "0.38.17", "implementation": "go",
        "moniker": "m", "app_version": 6, "source": "p2p"});
    let parsed: Observed = serde_json::from_value(old).unwrap();
    assert_eq!(parsed.app_release, None);
}
