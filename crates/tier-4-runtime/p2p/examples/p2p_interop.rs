//! Live interop with a Go celestia-core node (used by
//! `conformance/devnet/p2p-interop.sh`).
//!
//! Runs a Rust switch with the PEX reactor and a "sink" reactor on the
//! consensus, block sync and propagation channels that decodes every
//! message with celestia-core's protobuf types. Either dials the Go node
//! (`--dial ID@host:port`, outbound) or waits for it to dial us (inbound,
//! when the Go node has us as a persistent peer). After `--duration`
//! seconds it checks that the Go peer stayed connected the whole time in
//! the expected direction, that consensus messages arrived and decoded, and
//! that PEX addresses were exchanged; prints a JSON summary and exits 0 on
//! success.
//!
//! ```text
//! cargo run -p p2p --example p2p_interop -- --home DIR --print-id
//! cargo run -p p2p --example p2p_interop -- --home DIR --chain-id C \
//!     --listen 127.0.0.1:P --expect-peer GOID [--dial GOID@127.0.0.1:Q] [--duration 40]
//! ```
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use celestia_core_proto::tendermint::{blocksync, consensus, mempool, propagation};
use p2p::pex::{AddrBook, PexConfig, PexReactor};
use p2p::{
    ChannelDescriptor, Envelope, MConnConfig, NodeId, NodeInfo, NodeInfoOther, NodeKey, Peer,
    ProtocolVersion, Reactor, Switch, SwitchConfig,
};
use prost::Message as _;

const CONSENSUS: [u8; 4] = [0x20, 0x21, 0x22, 0x23];
const BLOCKSYNC: u8 = 0x40;
const PROPAGATION: [u8; 2] = [0x50, 0x51];
const MEMPOOL_CAT: [u8; 2] = [0x31, 0x32];

#[derive(Default)]
struct SinkState {
    per_channel: BTreeMap<u8, u64>,
    decode_errors: Vec<String>,
    adds: Vec<(String, bool)>,
    removes: Vec<(String, String)>,
}

/// Accepts and decodes everything on the consensus / blocksync /
/// propagation channels; tracks peer lifecycle.
#[derive(Default)]
struct Sink {
    st: Mutex<SinkState>,
}

impl Reactor for Sink {
    fn name(&self) -> String {
        "SINK".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        let mut v: Vec<ChannelDescriptor> = CONSENSUS
            .iter()
            .chain(PROPAGATION.iter())
            .chain(MEMPOOL_CAT.iter())
            .chain(std::iter::once(&BLOCKSYNC))
            .map(|&id| ChannelDescriptor {
                send_queue_capacity: 100,
                recv_message_capacity: 64 << 20,
                ..ChannelDescriptor::new(id, 5)
            })
            .collect();
        v.sort_by_key(|d| d.id);
        v
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        self.st
            .lock()
            .unwrap()
            .adds
            .push((peer.id().to_string(), peer.is_outbound()));
    }

    fn remove_peer(&self, peer: &Arc<Peer>, reason: &str) {
        self.st
            .lock()
            .unwrap()
            .removes
            .push((peer.id().to_string(), reason.to_string()));
    }

    fn receive(&self, env: Envelope) -> Result<(), String> {
        let ch = env.channel_id;
        let res = if CONSENSUS.contains(&ch) {
            consensus::Message::decode(env.message.as_slice()).map(|m| m.sum.is_some())
        } else if MEMPOOL_CAT.contains(&ch) {
            mempool::Message::decode(env.message.as_slice()).map(|m| m.sum.is_some())
        } else if ch == BLOCKSYNC {
            blocksync::Message::decode(env.message.as_slice()).map(|m| m.sum.is_some())
        } else {
            propagation::Message::decode(env.message.as_slice()).map(|m| m.sum.is_some())
        };
        let mut st = self.st.lock().unwrap();
        *st.per_channel.entry(ch).or_default() += 1;
        match res {
            Ok(true) => {}
            Ok(false) => st.decode_errors.push(format!("ch {ch:#x}: empty oneof")),
            Err(e) => st.decode_errors.push(format!("ch {ch:#x}: {e}")),
        }
        Ok(())
    }

    fn queue_size(&self) -> usize {
        1000
    }
}

struct Args {
    home: PathBuf,
    print_id: bool,
    chain_id: String,
    listen: String,
    dial: Option<String>,
    expect_peer: Option<NodeId>,
    duration: u64,
}

fn parse_args() -> Args {
    let mut a = Args {
        home: PathBuf::from("."),
        print_id: false,
        chain_id: String::new(),
        listen: "127.0.0.1:0".into(),
        dial: None,
        expect_peer: None,
        duration: 40,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("missing value for {k}"));
        match k.as_str() {
            "--home" => a.home = PathBuf::from(v()),
            "--print-id" => a.print_id = true,
            "--chain-id" => a.chain_id = v(),
            "--listen" => a.listen = v(),
            "--dial" => a.dial = Some(v()),
            "--expect-peer" => a.expect_peer = Some(v().parse().expect("peer id")),
            "--duration" => a.duration = v().parse().expect("duration"),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = parse_args();
    std::fs::create_dir_all(&args.home).unwrap();
    let key = NodeKey::load_or_generate(&args.home.join("node_key.json")).unwrap();
    if args.print_id {
        println!("{}", key.id());
        return;
    }
    let my_id = key.id();
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        network: args.chain_id.clone(),
        version: p2p::node_info::TM_CORE_SEMVER.into(),
        moniker: "rust-p2p".into(),
        listen_addr: format!("tcp://{}", args.listen),
        other: NodeInfoOther {
            tx_index: "off".into(),
            rpc_address: String::new(),
        },
        ..NodeInfo::default()
    };
    let cfg = SwitchConfig {
        listen_address: Some(format!("tcp://{}", args.listen)),
        allow_duplicate_ip: true,
        persistent_peers: args.dial.iter().cloned().collect(),
        mconn: MConnConfig {
            send_rate: 5_120_000,
            recv_rate: 5_120_000,
            flush_throttle: Duration::from_millis(10),
            // Ping Go often to exercise its pong path.
            ping_interval: Duration::from_secs(5),
            pong_timeout: Duration::from_secs(3),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).unwrap();
    sw.add_peer_filter(Arc::new(p2p::switch::celestia_peer_filter));
    let sink = Arc::new(Sink::default());
    sw.add_reactor(sink.clone()).unwrap();
    let book = Arc::new(AddrBook::new(Some(args.home.join("addrbook.json")), false));
    let pex = PexReactor::new(book.clone(), PexConfig::default());
    sw.set_addr_book(book.clone());
    sw.add_reactor(pex.clone()).unwrap();
    sw.start().await.unwrap();
    eprintln!("rust node {my_id} listening on {:?}", sw.net_address());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.duration);
    let mut status_samples = Vec::new();
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(id) = args.expect_peer
            && let Some(p) = sw.get_peer(&id)
        {
            status_samples.push(p.status());
        }
    }

    let go_peer = args.expect_peer.and_then(|id| sw.get_peer(&id));
    let st = sink.st.lock().unwrap();
    let expect = args.expect_peer.map(|i| i.to_string()).unwrap_or_default();
    let go_adds: Vec<&(String, bool)> = st.adds.iter().filter(|(id, _)| *id == expect).collect();
    let go_removes: Vec<&(String, String)> =
        st.removes.iter().filter(|(id, _)| *id == expect).collect();
    let consensus_msgs: u64 = CONSENSUS
        .iter()
        .map(|c| st.per_channel.get(c).copied().unwrap_or(0))
        .sum();
    let pex_stats = pex.stats();
    book.save();
    let book_size = book.size();
    let want_outbound = args.dial.is_some();
    let last = status_samples.last();
    let summary = serde_json::json!({
        "node_id": my_id.to_string(),
        "mode": if want_outbound { "outbound" } else { "inbound" },
        "go_peer_connected": go_peer.is_some(),
        "go_peer_outbound": go_peer.as_ref().map(|p| p.is_outbound()),
        "go_peer_moniker": go_peer.as_ref().map(|p| p.node_info().moniker.clone()),
        "go_peer_version": go_peer.as_ref().map(|p| p.node_info().version.clone()),
        "go_peer_channels": go_peer.as_ref().map(|p| hex::encode(&p.node_info().channels)),
        "go_adds": go_adds.len(),
        "go_removes": go_removes.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        "messages_per_channel": st.per_channel.iter().map(|(k, v)| (format!("{k:#04x}"), *v)).collect::<BTreeMap<_, _>>(),
        "consensus_messages": consensus_msgs,
        "decode_errors": st.decode_errors,
        "pex": {
            "requests_sent": pex_stats.requests_sent,
            "requests_received": pex_stats.requests_received,
            "addrs_received": pex_stats.addrs_received,
            "addresses_received": pex_stats.addresses_received,
            "book_size": book_size,
        },
        "bytes_sent": last.map(|s| s.send_monitor.bytes),
        "bytes_received": last.map(|s| s.recv_monitor.bytes),
        "connected_seconds": last.map(|s| s.duration.as_secs()),
        "pongs_from_go": last.map(|s| s.pongs_received),
        "pings_from_go": last.map(|s| s.pings_received),
    });
    println!("{summary}");

    let mut failures = Vec::new();
    if go_peer.is_none() {
        failures.push("Go peer not connected at the end".to_string());
    }
    if go_adds.len() != 1 || !go_removes.is_empty() {
        failures.push(format!(
            "Go peer connected {} times, disconnected {} times",
            go_adds.len(),
            go_removes.len()
        ));
    }
    if go_adds
        .first()
        .is_some_and(|(_, out)| *out != want_outbound)
    {
        failures.push("Go peer connected in the wrong direction".into());
    }
    if consensus_msgs == 0 {
        failures.push("no consensus messages received".into());
    }
    if !st.decode_errors.is_empty() {
        failures.push("undecodable messages".into());
    }
    if pex_stats.addrs_received == 0 {
        failures.push("no PexAddrs received".into());
    }
    if last.is_none_or(|s| s.pongs_received == 0) {
        failures.push("Go never answered our pings".into());
    }
    // Go pings every 60s.
    if last.is_some_and(|s| s.duration > Duration::from_secs(65) && s.pings_received == 0) {
        failures.push("no ping received from Go".into());
    }
    if last.is_none_or(|s| s.duration < Duration::from_secs(args.duration.saturating_sub(10))) {
        failures.push("connection did not last".into());
    }
    drop(st);
    sw.stop();
    if failures.is_empty() {
        eprintln!(
            "PASS ({})",
            if want_outbound { "outbound" } else { "inbound" }
        );
    } else {
        eprintln!("FAIL: {failures:?}");
        std::process::exit(1);
    }
}
