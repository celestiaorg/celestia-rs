//! Conformance against Go celestia-core v0.42.3 vectors
//! (`conformance/p2pvectors`, fixture `conformance/fixtures/p2p/go-vectors.json`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use std::time::Duration;

use celestia_core_proto::tendermint::p2p as pb;
use p2p::mconnection::{self, ChannelDescriptor, MConnConfig, MConnection};
use p2p::netaddress::NetAddress;
use p2p::node_info::NodeInfo;
use p2p::pex::{AddrBook, reactor::cap_addresses};
use p2p::secret_connection::{ByteRead, ByteWrite, Plain, SecretConnection, encode_delimited};
use p2p::{NodeId, NodeKey};
use prost::Message as _;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/p2p/go-vectors.json"
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn hx(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn arr32(v: &Value) -> [u8; 32] {
    hx(v).try_into().unwrap()
}

fn pattern(n: usize, salt: u8) -> Vec<u8> {
    (0..n).map(|i| ((i * 7) as u8) ^ salt).collect()
}

/// Run our side of a recorded handshake: we are the peer with `seed` /
/// `eph`, the remote's bytes are replayed, and our output must equal Go's.
async fn replay_side(
    seed: [u8; 32],
    eph: [u8; 32],
    expect_remote_id: &str,
    remote_wire: Vec<u8>,
    our_wire: Vec<u8>,
    our_msgs: Vec<Vec<u8>>,
    their_msgs: Vec<Vec<u8>>,
) {
    let (ours, mut theirs) = tokio::io::duplex(1 << 20);
    theirs.write_all(&remote_wire).await.unwrap();
    let key = NodeKey::from_seed(&seed);
    let expected_len = our_wire.len();
    let collector = tokio::spawn(async move {
        let mut got = vec![0u8; expected_len];
        theirs.read_exact(&mut got).await.unwrap();
        got
    });
    let sc = SecretConnection::handshake_with_ephemeral(ours, &key, eph)
        .await
        .unwrap();
    assert_eq!(sc.remote_id().to_string(), expect_remote_id);
    let (mut r, mut w) = sc.into_split();
    for m in &our_msgs {
        w.write_all_bytes(m).await.unwrap();
    }
    let want: Vec<u8> = their_msgs.concat();
    let mut got = vec![0u8; want.len()];
    r.read_exact_bytes(&mut got).await.unwrap();
    assert_eq!(got, want);
    let wire = tokio::time::timeout(Duration::from_secs(5), collector)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hex::encode(&wire),
        hex::encode(&our_wire),
        "wire bytes differ"
    );
}

#[tokio::test]
async fn secret_connection_matches_go() {
    let v = vectors();
    let cases = v["secret_connection"].as_array().unwrap();
    assert!(cases.len() >= 4);
    for c in cases {
        let msgs = |k: &str| c[k].as_array().unwrap().iter().map(hx).collect::<Vec<_>>();
        // Rust plays A against Go's B, then B against Go's A.
        replay_side(
            arr32(&c["seed_a"]),
            arr32(&c["eph_a"]),
            c["id_b"].as_str().unwrap(),
            hx(&c["wire_b"]),
            hx(&c["wire_a"]),
            msgs("msgs_a"),
            msgs("msgs_b"),
        )
        .await;
        replay_side(
            arr32(&c["seed_b"]),
            arr32(&c["eph_b"]),
            c["id_a"].as_str().unwrap(),
            hx(&c["wire_a"]),
            hx(&c["wire_b"]),
            msgs("msgs_b"),
            msgs("msgs_a"),
        )
        .await;
        assert_eq!(
            NodeKey::from_seed(&arr32(&c["seed_a"])).id().to_string(),
            c["id_a"].as_str().unwrap()
        );
    }
}

#[test]
fn packets_match_go() {
    let v = vectors();
    for p in v["packets"].as_array().unwrap() {
        let ours = match p["name"].as_str().unwrap() {
            "ping" => mconnection::ping_packet(),
            "pong" => mconnection::pong_packet(),
            "msg_empty_eof" => mconnection::msg_packet(0x20, true, vec![]),
            "msg_ch0_small" => mconnection::msg_packet(0, true, vec![1, 2, 3]),
            "msg_not_eof" => mconnection::msg_packet(0x30, false, pattern(100, 1)),
            "msg_max_payload" => mconnection::msg_packet(1, true, pattern(1024, 2)),
            "msg_ch_ff" => mconnection::msg_packet(0xff, true, pattern(300, 3)),
            other => panic!("unknown packet vector {other}"),
        };
        assert_eq!(
            hex::encode(ours),
            p["delimited"].as_str().unwrap(),
            "{}",
            p["name"]
        );
    }
}

fn mconn_cfg(max_payload: usize) -> MConnConfig {
    MConnConfig {
        max_packet_msg_payload_size: max_payload,
        flush_throttle: Duration::from_millis(5),
        ..MConnConfig::default()
    }
}

#[tokio::test]
async fn mconnection_wire_matches_go() {
    let v = vectors();
    for c in v["mconnection"].as_array().unwrap() {
        let max_payload = usize::try_from(c["max_payload"].as_u64().unwrap()).unwrap();
        assert_eq!(
            mconnection::max_packet_msg_size(max_payload),
            usize::try_from(c["max_packet_msg_size"].as_u64().unwrap()).unwrap()
        );
        let ch = u8::try_from(c["channel_id"].as_u64().unwrap()).unwrap();
        let msgs: Vec<Vec<u8>> = c["msgs"].as_array().unwrap().iter().map(hx).collect();
        let wire = hx(&c["wire"]);
        let descs = vec![ChannelDescriptor {
            send_queue_capacity: 100,
            ..ChannelDescriptor::new(ch, 1)
        }];

        // Receive: Go's bytes reassemble into the same messages.
        let (a, mut b) = tokio::io::duplex(1 << 20);
        let (ar, aw) = tokio::io::split(a);
        let (conn, mut rx) =
            MConnection::start(Plain(ar), Plain(aw), &descs, mconn_cfg(max_payload)).unwrap();
        b.write_all(&wire).await.unwrap();
        for m in &msgs {
            let (got_ch, got) = rx.recv().await.unwrap();
            assert_eq!(got_ch, ch);
            assert_eq!(&got, m);
        }
        conn.stop();

        // Send: we produce Go's bytes.
        let (a, mut b) = tokio::io::duplex(1 << 20);
        let (ar, aw) = tokio::io::split(a);
        let (conn, _rx) =
            MConnection::start(Plain(ar), Plain(aw), &descs, mconn_cfg(max_payload)).unwrap();
        for m in &msgs {
            assert!(conn.send(ch, m.clone()).await);
        }
        let mut got = vec![0u8; wire.len()];
        tokio::time::timeout(Duration::from_secs(5), b.read_exact(&mut got))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hex::encode(&got), hex::encode(&wire), "{}", c["name"]);
        conn.stop();
    }
}

#[test]
fn node_info_matches_go() {
    let v = vectors();
    let cases = v["node_info"].as_array().unwrap();
    let infos: Vec<(String, NodeInfo)> = cases
        .iter()
        .map(|c| {
            let proto = hx(&c["proto"]);
            let ni = NodeInfo::from_proto(&pb::DefaultNodeInfo::decode(proto.as_slice()).unwrap());
            assert_eq!(
                hex::encode(ni.to_proto().encode_to_vec()),
                c["proto"].as_str().unwrap()
            );
            assert_eq!(
                hex::encode(encode_delimited(&ni.to_proto())),
                c["delimited"].as_str().unwrap()
            );
            (c["name"].as_str().unwrap().to_string(), ni)
        })
        .collect();
    for (c, (name, ni)) in cases.iter().zip(&infos) {
        let go_err = c["validate_err"].as_str().unwrap();
        let ours = ni.validate();
        assert_eq!(
            ours.is_err(),
            !go_err.is_empty(),
            "validate {name}: ours={ours:?} go={go_err}"
        );
        for r in c["compat"].as_array().unwrap() {
            let other = &infos
                .iter()
                .find(|(n, _)| n == r["other"].as_str().unwrap())
                .unwrap()
                .1;
            let go_err = r["err"].as_str().unwrap();
            let ours = ni.compatible_with(other);
            assert_eq!(
                ours.is_err(),
                !go_err.is_empty(),
                "compat {name} vs {}: {ours:?} / {go_err}",
                r["other"]
            );
        }
    }
}

#[test]
fn net_address_matches_go() {
    let v = vectors();
    for c in v["net_address"].as_array().unwrap() {
        let input = c["input"].as_str().unwrap();
        let res = NetAddress::parse(input);
        if c["err"].as_bool().unwrap() {
            assert!(res.is_err(), "{input} should fail: {res:?}");
            continue;
        }
        let na = res.unwrap_or_else(|e| panic!("{input}: {e}"));
        assert_eq!(na.to_string(), c["string"].as_str().unwrap(), "{input}");
        assert_eq!(
            na.dial_string(),
            c["dial_string"].as_str().unwrap(),
            "{input}"
        );
        assert_eq!(na.valid().is_ok(), c["valid"].as_bool().unwrap(), "{input}");
        assert_eq!(na.routable(), c["routable"].as_bool().unwrap(), "{input}");
        assert_eq!(na.local(), c["local"].as_bool().unwrap(), "{input}");
        assert_eq!(
            hex::encode(na.to_proto().encode_to_vec()),
            c["proto"].as_str().unwrap(),
            "{input}"
        );
    }
}

#[test]
fn pex_messages_match_go() {
    let v = vectors();
    let id1: NodeId = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".parse().unwrap();
    let id2: NodeId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
    for c in v["pex"].as_array().unwrap() {
        let ours = match c["name"].as_str().unwrap() {
            "request" => pb::Message {
                sum: Some(pb::message::Sum::PexRequest(pb::PexRequest {})),
            },
            "addrs_empty" => cap_addresses(&[]),
            "addrs_two" => cap_addresses(&[
                NetAddress::with_id(id1, "1.2.3.4".parse().unwrap(), 26656),
                NetAddress::with_id(id2, "2607:f8b0::1".parse().unwrap(), 1),
            ]),
            other => panic!("unknown {other}"),
        };
        assert_eq!(
            hex::encode(ours.encode_to_vec()),
            c["bytes"].as_str().unwrap(),
            "{}",
            c["name"]
        );
    }
}

#[test]
fn files_match_go() {
    let v = vectors();
    let f = &v["files"];
    let json = f["node_key_json"].as_str().unwrap();
    let key = NodeKey::from_json(json.as_bytes()).unwrap();
    assert_eq!(key.id().to_string(), f["node_key_id"].as_str().unwrap());
    assert_eq!(
        hex::encode(key.public_key()),
        f["node_key_pub"].as_str().unwrap()
    );
    assert_eq!(key.to_json(), json);

    let dir = std::env::temp_dir().join(format!("p2p-govec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("addrbook.json");
    std::fs::write(&path, f["addrbook_json"].as_str().unwrap()).unwrap();
    let book = AddrBook::new(Some(path.clone()), false);
    book.load_from_file(&path).unwrap();
    assert_eq!(book.size(), 5);
    let known = book.known_addresses();
    assert_eq!(known.iter().filter(|k| k.is_old()).count(), 1);
    assert_eq!(known.iter().map(|k| k.attempts).sum::<i32>(), 1);
    // Our save is readable back and keeps Go's field set.
    book.save();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let go: Value = serde_json::from_str(f["addrbook_json"].as_str().unwrap()).unwrap();
    assert_eq!(saved["key"], go["key"]);
    let keys = |v: &Value| {
        let mut k: Vec<String> = v["addrs"][0].as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&saved), keys(&go));
    let mut ours: Vec<Value> = saved["addrs"].as_array().unwrap().clone();
    let mut theirs: Vec<Value> = go["addrs"].as_array().unwrap().clone();
    let by_id = |a: &Value, b: &Value| a["addr"]["id"].as_str().cmp(&b["addr"]["id"].as_str());
    ours.sort_by(by_id);
    theirs.sort_by(by_id);
    assert_eq!(ours, theirs, "addrbook entries round-trip exactly");
    let _ = std::fs::remove_dir_all(&dir);
}
