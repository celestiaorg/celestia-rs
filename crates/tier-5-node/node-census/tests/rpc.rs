//! Peers learned from a public RPC node's `/net_info`.

use node_census::classify::Implementation;
use node_census::crawl::Source;
use node_census::rpc::parse_net_info;
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
        peer(A, "0.38.17", "188.40.102.137:15656", "188.40.102.137", "celestia"),
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
