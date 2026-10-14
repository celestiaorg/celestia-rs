//! A byzantine peer for `conformance/devnet/double-sign-evidence.sh`: it
//! connects to one node over P2P, waits until that node reaches
//! `--at-height`, then sends it two conflicting prevotes of one validator
//! (signed with that validator's `priv_validator_key.json`) for the node's
//! current height and round. The receiving node must detect the duplicate
//! vote, add `DuplicateVoteEvidence` to its evidence pool and gossip it, so
//! that it is committed in a block by the network.
//!
//! usage: double_sign --key <priv_validator_key.json> --val-index <i>
//!   --chain-id <id> --dial <id@host:port> --listen <host:port> --at-height <h>

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::panic
)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use consensus::ConsensusMessage;
use ctypes::{BlockId, PartSetHeader, Vote};
use p2p::{ChannelDescriptor, Envelope, NodeInfo, NodeInfoOther, ProtocolVersion, Switch};

/// Advertises the channels celestia-core peers require and records the
/// target's round state from its `NewRoundStep` messages.
struct Byzantine {
    height: AtomicI64,
    round: AtomicI64,
    peer: Mutex<Option<Arc<p2p::Peer>>>,
}

impl p2p::Reactor for Byzantine {
    fn name(&self) -> String {
        "BYZANTINE".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        [0x20u8, 0x22, 0x23, 0x30, 0x31, 0x32, 0x50, 0x51]
            .iter()
            .map(|&id| ChannelDescriptor {
                send_queue_capacity: 100,
                recv_message_capacity: 64 << 20,
                ..ChannelDescriptor::new(id, 5)
            })
            .collect()
    }
    fn add_peer(&self, peer: &Arc<p2p::Peer>) {
        *self.peer.lock().unwrap() = Some(peer.clone());
    }
    fn receive(&self, env: Envelope) -> Result<(), String> {
        if env.channel_id == 0x20
            && let Ok(ConsensusMessage::NewRoundStep { height, round, .. }) =
                ConsensusMessage::decode(&env.message)
            && height >= self.height.load(Ordering::SeqCst)
        {
            self.height.store(height, Ordering::SeqCst);
            self.round.store(i64::from(round), Ordering::SeqCst);
        }
        Ok(())
    }
    fn queue_size(&self) -> usize {
        1000
    }
}

fn arg(name: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    let i = args
        .iter()
        .position(|a| a == name)
        .unwrap_or_else(|| panic!("missing {name}"));
    args[i + 1].clone()
}

fn prevote(
    key: &privval::FilePvKey,
    chain_id: &str,
    height: i64,
    round: i32,
    index: i32,
    seed: &[u8],
) -> Vote {
    let mut vote = Vote {
        vote_type: ctypes::vote::PREVOTE_TYPE,
        height,
        round,
        block_id: BlockId {
            hash: hash::sha256(seed).to_vec(),
            part_set_header: PartSetHeader {
                total: 1,
                hash: hash::sha256(&[seed, b"/parts"].concat()).to_vec(),
            },
        },
        timestamp: consensus::ticker::system_now(),
        validator_address: key.address.clone(),
        validator_index: index,
        ..Vote::default()
    };
    let sb = ctypes::canonical::vote_sign_bytes(chain_id, &vote.to_proto()).unwrap();
    vote.signature = crypto::ed25519::sign(&key.priv_key, &sb).unwrap().to_vec();
    vote
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();
    let key = privval::FilePvKey::from_json(&std::fs::read(arg("--key")).unwrap()).unwrap();
    let index: i32 = arg("--val-index").parse().unwrap();
    let chain_id = arg("--chain-id");
    let at: i64 = arg("--at-height").parse().unwrap();
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        network: chain_id.clone(),
        version: p2p::node_info::TM_CORE_SEMVER.into(),
        moniker: "byzantine".into(),
        other: NodeInfoOther {
            tx_index: "off".into(),
            rpc_address: String::new(),
        },
        ..NodeInfo::default()
    };
    let sw = Switch::new(
        p2p::SwitchConfig {
            listen_address: Some(format!("tcp://{}", arg("--listen"))),
            allow_duplicate_ip: true,
            persistent_peers: vec![arg("--dial")],
            ..p2p::SwitchConfig::default()
        },
        p2p::NodeKey::generate(),
        info,
    )
    .unwrap();
    let byz = Arc::new(Byzantine {
        height: AtomicI64::new(0),
        round: AtomicI64::new(0),
        peer: Mutex::new(None),
    });
    sw.add_reactor(byz.clone()).unwrap();
    sw.start().await.unwrap();
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let h = byz.height.load(Ordering::SeqCst);
        let peer = byz.peer.lock().unwrap().clone();
        let Some(peer) = peer else { continue };
        if h < at {
            continue;
        }
        let r = i32::try_from(byz.round.load(Ordering::SeqCst)).unwrap();
        let a = prevote(&key, &chain_id, h, r, index, b"double-sign-a");
        let b = prevote(&key, &chain_id, h, r, index, b"double-sign-b");
        for v in [a, b] {
            let ok = peer.send(0x22, ConsensusMessage::Vote(v).encode()).await;
            println!("sent conflicting prevote h={h} r={r} ok={ok}");
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        sw.stop();
        return;
    }
}
