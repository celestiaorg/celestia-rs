//! The evidence reactor (`evidence/reactor.go`): channel 0x38 carries
//! `tendermint.types.EvidenceList`. Received evidence is verified and
//! added to the pool (invalid evidence disconnects the sender); pending
//! evidence is gossiped to every peer whose consensus height is past it.
//!
//! Go runs one `broadcastEvidenceRoutine` goroutine per peer walking the
//! pool's `clist`; here one thread advances a cursor per peer through the
//! pool's insertion-ordered list with the same rules: send the element
//! (if the peer is ready for it), then move to the next element when one
//! appears, or start over from the front after 10 s; a failed send is
//! retried after 100 ms.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctypes::Evidence;
use ctypes::proto::types as pb;
use p2p::{ChannelDescriptor, Envelope, Peer};
use prost::Message as _;

use crate::pool::{AddError, Pool, evidence_string};

/// `EvidenceChannel`.
pub const EVIDENCE_CHANNEL: u8 = 0x38;
/// `maxMsgSize`.
pub const MAX_MSG_SIZE: usize = 1_048_576;
/// `broadcastEvidenceIntervalS`.
pub const BROADCAST_EVIDENCE_INTERVAL: Duration = Duration::from_secs(10);
/// `peerRetryMessageIntervalMS`.
pub const PEER_RETRY_MESSAGE_INTERVAL: Duration = Duration::from_millis(100);
/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 1;

/// A peer as the gossip routine sees it (implemented for [`p2p::Peer`]).
pub trait GossipPeer: Send + Sync {
    /// ID.
    fn id(&self) -> String;
    /// Whether the connection is up.
    fn is_running(&self) -> bool;
    /// The peer's consensus height, if the consensus reactor published it.
    fn consensus_height(&self) -> Option<i64>;
    /// Sends on the evidence channel.
    fn send(&self, msg: Vec<u8>) -> bool;
}

impl GossipPeer for Peer {
    fn id(&self) -> String {
        Peer::id(self).to_string()
    }
    fn is_running(&self) -> bool {
        Peer::is_running(self)
    }
    fn consensus_height(&self) -> Option<i64> {
        let data = self.get(mempool::cat::PEER_STATE_KEY)?;
        let h = data.downcast_ref::<mempool::cat::PeerStateHandle>()?;
        Some(mempool::cat::PeerState::get_height(h.0.as_ref()))
    }
    fn send(&self, msg: Vec<u8>) -> bool {
        self.try_send(EVIDENCE_CHANNEL, msg)
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum Cursor {
    /// `next == nil`: wait for the front of the list.
    Front,
    /// At an element.
    At {
        seq: u64,
        ev: Evidence,
        /// None until the element was handled; then the 10 s deadline.
        deadline: Option<Instant>,
        retry_at: Option<Instant>,
    },
}

struct PeerRoutine {
    peer: Arc<dyn GossipPeer>,
    cursor: Cursor,
}

/// `evidenceListToProto`.
pub fn evidence_list_to_proto(evis: &[Evidence]) -> Result<pb::EvidenceList, String> {
    Ok(pb::EvidenceList {
        evidence: evis
            .iter()
            .map(|e| e.to_proto().map_err(|x| x.to_string()))
            .collect::<Result<_, _>>()?,
    })
}

/// `evidenceListFromProto` (decoding validates every item).
pub fn evidence_list_from_proto(m: &pb::EvidenceList) -> Result<Vec<Evidence>, String> {
    let evis = m
        .evidence
        .iter()
        .map(|e| Evidence::from_proto(e).map_err(|x| x.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    for (i, ev) in evis.iter().enumerate() {
        if let Err(e) = ev.validate_basic() {
            return Err(format!("invalid evidence (#{i}): {e}"));
        }
    }
    Ok(evis)
}

/// The evidence reactor.
pub struct Reactor {
    pool: Pool,
    peers: Mutex<HashMap<String, PeerRoutine>>,
    running: AtomicBool,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EvidenceReactor")
    }
}

impl Reactor {
    /// `NewReactor`.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            peers: Mutex::new(HashMap::new()),
            running: AtomicBool::new(false),
        }
    }

    /// The pool.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// `AddPeer`: starts gossiping to the peer.
    pub fn add_gossip_peer(&self, peer: Arc<dyn GossipPeer>) {
        if let Ok(mut g) = self.peers.lock() {
            g.insert(
                peer.id(),
                PeerRoutine {
                    peer,
                    cursor: Cursor::Front,
                },
            );
        }
    }

    /// Stops gossiping to a peer.
    pub fn remove_gossip_peer(&self, id: &str) {
        if let Ok(mut g) = self.peers.lock() {
            g.remove(id);
        }
    }

    /// `Receive` of raw bytes from `peer`; `Err` disconnects it.
    pub fn receive_bytes(&self, peer: &str, bytes: &[u8]) -> Result<(), String> {
        let list = ctypes::gogo_decode::<pb::EvidenceList>(bytes).map_err(|e| e.to_string())?;
        let evis = evidence_list_from_proto(&list).map_err(|e| {
            tracing::error!(src = peer, err = %e, "Error decoding message");
            e
        })?;
        for ev in evis {
            match self.pool.add_evidence(ev) {
                Ok(()) => {}
                Err(AddError::Invalid(e)) => {
                    tracing::error!("{e}");
                    return Err(e);
                }
                Err(AddError::Other(e)) => {
                    tracing::error!(err = %e, "Evidence has not been added");
                }
            }
        }
        Ok(())
    }

    /// `prepareEvidenceMessage`: the evidence if the peer is ready for it
    /// (`peerHeight - maxAge < evHeight < peerHeight`).
    fn prepare_evidence_message(&self, peer: &dyn GossipPeer, ev: &Evidence) -> Option<Evidence> {
        let ev_height = ev.height();
        let peer_height = peer.consensus_height()?;
        let params = self.pool.state().consensus_params.evidence;
        let age_num_blocks = peer_height - ev_height;
        if peer_height <= ev_height {
            return None;
        }
        if age_num_blocks > params.max_age_num_blocks {
            tracing::info!(
                peer_height,
                ev_height,
                max_age_num_blocks = params.max_age_num_blocks,
                peer = %peer.id(),
                "Not sending peer old evidence"
            );
            return None;
        }
        Some(ev.clone())
    }

    /// One pass of every peer's broadcast routine.
    pub fn gossip_step(&self, now: Instant) {
        let Ok(mut peers) = self.peers.lock() else {
            return;
        };
        peers.retain(|_, r| r.peer.is_running());
        for r in peers.values_mut() {
            loop {
                match &mut r.cursor {
                    Cursor::Front => match self.pool.next_after(None) {
                        Some((seq, ev)) => {
                            r.cursor = Cursor::At {
                                seq,
                                ev,
                                deadline: None,
                                retry_at: None,
                            };
                        }
                        None => break,
                    },
                    Cursor::At {
                        seq,
                        ev,
                        deadline,
                        retry_at,
                    } => match deadline {
                        None => {
                            if retry_at.is_some_and(|t| now < t) {
                                break;
                            }
                            if let Some(e) = self.prepare_evidence_message(r.peer.as_ref(), ev) {
                                tracing::debug!(ev = %evidence_string(&e), peer = %r.peer.id(), "Gossiping evidence to peer");
                                let msg = match evidence_list_to_proto(&[e]) {
                                    Ok(m) => m.encode_to_vec(),
                                    Err(err) => {
                                        tracing::error!(err = %err, "evidence to proto");
                                        break;
                                    }
                                };
                                if !r.peer.send(msg) {
                                    *retry_at = Some(now + PEER_RETRY_MESSAGE_INTERVAL);
                                    break;
                                }
                            }
                            *deadline = Some(now + BROADCAST_EVIDENCE_INTERVAL);
                        }
                        Some(d) => {
                            if now >= *d {
                                r.cursor = Cursor::Front;
                                continue;
                            }
                            match self.pool.next_after(Some(*seq)) {
                                Some((s, e)) => {
                                    r.cursor = Cursor::At {
                                        seq: s,
                                        ev: e,
                                        deadline: None,
                                        retry_at: None,
                                    };
                                }
                                None => break,
                            }
                        }
                    },
                }
            }
        }
    }

    fn gossip_routine(self: &Arc<Self>) {
        while self.running.load(Ordering::SeqCst) {
            self.gossip_step(Instant::now());
            self.pool.wait_added(PEER_RETRY_MESSAGE_INTERVAL);
        }
    }
}

impl p2p::Reactor for Reactor {
    fn name(&self) -> String {
        "EVIDENCE".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            send_queue_capacity: 10,
            recv_message_capacity: MAX_MSG_SIZE,
            ..ChannelDescriptor::new(EVIDENCE_CHANNEL, 6)
        }]
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        self.running.store(true, Ordering::SeqCst);
        let me = Arc::clone(&self);
        std::thread::Builder::new()
            .name("evidence-gossip".into())
            .spawn(move || me.gossip_routine())
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        self.add_gossip_peer(Arc::clone(peer) as Arc<dyn GossipPeer>);
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        self.remove_gossip_peer(&peer.id().to_string());
    }

    fn receive(&self, envelope: Envelope) -> Result<(), String> {
        self.receive_bytes(&envelope.src.id().to_string(), &envelope.message)
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
