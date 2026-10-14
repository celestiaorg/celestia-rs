//! A minimal gossip layer over an abstract [`Network`], standing in for the
//! consensus reactor (`consensus/reactor.go`) until it is wired to the
//! `p2p` crate.
//!
//! Own proposals, block parts and votes are broadcast as soon as the state
//! machine creates them; new round steps are announced. On every gossip
//! tick each peer is sent what it may be missing given its announced
//! height: at our height the current proposal, its parts and every vote we
//! hold; a peer behind us gets the commit precommits and block parts of
//! its height from the block store (`gossipDataForCatchup`,
//! `gossipVotesForHeight`). Messages are idempotent, so re-sending is safe
//! under message loss.

use std::collections::BTreeMap;

use crate::state::{ConsensusState, Event};
use crate::types::{ConsensusMessage, RoundStep};

/// What the gossip layer needs from the network.
pub trait Network {
    /// Connected peers.
    fn peers(&self) -> Vec<String>;
    /// Sends a message to a peer (best effort).
    fn send(&mut self, peer: &str, msg: ConsensusMessage);
}

/// `PeerRoundState` (the parts this layer uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerState {
    /// Announced height.
    pub height: i64,
    /// Announced round.
    pub round: i32,
    /// Announced step.
    pub step: RoundStep,
}

/// The gossip layer of one node.
#[derive(Debug, Default)]
pub struct Gossip {
    peers: BTreeMap<String, PeerState>,
}

impl Gossip {
    /// A fresh gossip layer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Known state of a peer.
    #[must_use]
    pub fn peer_state(&self, peer: &str) -> Option<PeerState> {
        self.peers.get(peer).copied()
    }

    /// Handles a message from a peer.
    pub fn receive(&mut self, cs: &mut ConsensusState, peer: &str, msg: ConsensusMessage) {
        match msg {
            ConsensusMessage::NewRoundStep {
                height,
                round,
                step,
                ..
            } => {
                self.peers.insert(
                    peer.to_owned(),
                    PeerState {
                        height,
                        round,
                        step,
                    },
                );
            }
            m @ (ConsensusMessage::Proposal(_)
            | ConsensusMessage::BlockPart { .. }
            | ConsensusMessage::Vote(_)) => cs.receive_peer(m, peer),
            _ => {}
        }
    }

    /// Broadcasts own messages and round steps from state-machine events.
    pub fn on_events(&mut self, events: &[Event], net: &mut dyn Network) {
        for ev in events {
            match ev {
                Event::Broadcast(m) => {
                    for p in net.peers() {
                        net.send(&p, m.clone());
                    }
                }
                Event::NewRoundStep {
                    height,
                    round,
                    step,
                } => {
                    let m = ConsensusMessage::NewRoundStep {
                        height: *height,
                        round: *round,
                        step: *step,
                        seconds_since_start_time: 0,
                        last_commit_round: -1,
                    };
                    for p in net.peers() {
                        net.send(&p, m.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Periodic re-gossip (see the module docs).
    pub fn tick(&mut self, cs: &ConsensusState, net: &mut dyn Network) {
        let rs = cs.round_state();
        let me = ConsensusMessage::NewRoundStep {
            height: rs.height,
            round: rs.round,
            step: rs.step,
            seconds_since_start_time: 0,
            last_commit_round: -1,
        };
        for peer in net.peers() {
            net.send(&peer, me.clone());
            let ph = self.peers.get(&peer).map_or(rs.height, |p| p.height);
            if ph == rs.height {
                if let Some(p) = &rs.proposal {
                    net.send(&peer, ConsensusMessage::Proposal(p.clone()));
                }
                if let Some(parts) = &rs.proposal_block_parts {
                    for i in 0..parts.total() as usize {
                        if let Some(part) = parts.get_part(i) {
                            net.send(
                                &peer,
                                ConsensusMessage::BlockPart {
                                    height: rs.height,
                                    round: rs.round,
                                    part,
                                },
                            );
                        }
                    }
                }
                for v in cs.known_votes() {
                    net.send(&peer, ConsensusMessage::Vote(v));
                }
            } else if ph < rs.height && ph >= cs.block_store().base() && ph > 0 {
                Self::send_catchup(cs, &peer, ph, net);
            }
        }
    }

    fn send_catchup(cs: &ConsensusState, peer: &str, h: i64, net: &mut dyn Network) {
        let bs = cs.block_store();
        let Ok(Some(commit)) = bs.load_seen_commit(h) else {
            return;
        };
        for i in 0..commit.signatures.len() {
            if commit.signatures[i].block_id_flag == ctypes::BLOCK_ID_FLAG_ABSENT {
                continue;
            }
            if let Ok(v) = commit.get_vote(i32::try_from(i).unwrap_or(i32::MAX)) {
                net.send(peer, ConsensusMessage::Vote(v));
            }
        }
        if let Ok(Some((parts, _))) = bs.load_part_set(h) {
            for i in 0..parts.total() as usize {
                if let Some(part) = parts.get_part(i) {
                    net.send(
                        peer,
                        ConsensusMessage::BlockPart {
                            height: h,
                            round: commit.round,
                            part,
                        },
                    );
                }
            }
        }
    }
}
