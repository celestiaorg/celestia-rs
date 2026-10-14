//! The consensus thread: Go's `receiveRoutine` + `syncData` around the
//! single-threaded [`ConsensusState`], wired to the reactor.
//!
//! Inputs (peer messages from the reactor, proposals and parts from the
//! propagation reactor, `VoteSetMaj23` claims, txs-available signals,
//! `SwitchToConsensus`) arrive on one channel; timeouts fire from the
//! state machine's ticker. After each batch the state machine's events are
//! turned into reactor broadcasts (`NewRoundStep`, `NewValidBlock`,
//! `HasVote`) and the round state snapshot is republished.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use ctypes::BlockId;
use nodestore::state::State;
use p2p::NodeId;

use crate::peer_state::NewRoundStep;
use crate::reactor::{Reactor, Snapshot};
use crate::state::{ConsensusState, Event};
use crate::types::ConsensusMessage;

/// An input of the consensus thread.
#[derive(Debug)]
pub enum Input {
    /// A proposal, block part or vote from a peer (`peerMsgQueue`).
    Peer {
        /// The message.
        msg: ConsensusMessage,
        /// Sender (empty for local).
        peer: String,
    },
    /// A proposal or part from the propagation reactor.
    Propagation(propagation::Output),
    /// A peer claims +2/3 for a block (`VoteSetMaj23Message`).
    PeerMaj23 {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Vote type.
        vote_type: i32,
        /// The peer.
        peer: NodeId,
        /// Claimed block.
        block_id: BlockId,
    },
    /// The mempool has transactions.
    TxsAvailable,
    /// Block / state sync is done (`SwitchToConsensus`).
    SwitchToConsensus {
        /// The synced state.
        state: Box<State>,
        /// Skip the WAL catch-up.
        skip_wal: bool,
    },
    /// Stop the thread.
    Stop,
}

/// What the node wants after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep running.
    Continue,
    /// Stop the consensus thread (e.g. halt height reached).
    Stop,
}

/// The reactor's view of the state machine.
#[must_use]
pub fn snapshot_of(cs: &ConsensusState) -> Snapshot {
    Snapshot {
        rs: cs.round_state().clone(),
        initial_height: cs.state().initial_height,
        abci: cs.state().consensus_params.abci,
    }
}

fn dispatch(reactor: &Reactor, ev: &Event) {
    match ev {
        Event::NewRoundStep {
            height,
            round,
            step,
            start_time,
            last_commit_round,
        } => reactor.broadcast_new_round_step(NewRoundStep {
            height: *height,
            round: *round,
            step: *step,
            seconds_since_start_time: nodestore::state::sub_times(
                crate::ticker::system_now(),
                *start_time,
            ) / 1_000_000_000,
            last_commit_round: *last_commit_round,
        }),
        Event::ValidBlock {
            height,
            round,
            parts_header,
            is_commit,
            block_parts,
        } => reactor.broadcast_new_valid_block(
            *height,
            *round,
            parts_header,
            block_parts,
            *is_commit,
        ),
        Event::Vote(v) => {
            reactor.broadcast_has_vote(v.height, v.round, v.vote_type, v.validator_index);
        }
        Event::ConflictingVotes { vote_a, vote_b } => {
            tracing::error!(?vote_a, ?vote_b, "conflicting votes");
        }
        Event::Broadcast(_) | Event::NewBlock { .. } => {}
    }
}

fn handle(cs: &mut ConsensusState, reactor: &Reactor, input: Input) -> bool {
    match input {
        Input::Peer { msg, peer } => cs.receive_peer(msg, &peer),
        Input::Propagation(propagation::Output::Proposal { proposal, from }) => {
            cs.receive_peer(ConsensusMessage::Proposal(proposal), &from);
        }
        Input::Propagation(propagation::Output::Part {
            height,
            round,
            part,
        }) => {
            // syncData: only parts of the current height and round the state
            // machine does not have yet.
            let rs = cs.round_state();
            if height != rs.height || round != rs.round {
                return true;
            }
            if let Some(parts) = &rs.proposal_block_parts
                && (parts.is_complete() || parts.has_part(part.index as usize))
            {
                return true;
            }
            cs.receive_peer(
                ConsensusMessage::BlockPart {
                    height,
                    round,
                    part,
                },
                "",
            );
        }
        Input::PeerMaj23 {
            height,
            round,
            vote_type,
            peer,
            block_id,
        } => match cs.set_peer_maj23(height, round, vote_type, &peer.to_string(), &block_id) {
            Ok(Some(votes)) => {
                reactor.send_vote_set_bits(&peer, height, round, vote_type, block_id, votes);
            }
            Ok(None) => {}
            Err(e) => reactor.stop_peer_for_error(&peer, &e),
        },
        Input::TxsAvailable => cs.txs_available(),
        Input::SwitchToConsensus { state, skip_wal } => {
            if let Err(e) = cs.switch_to_consensus(*state, skip_wal) {
                tracing::error!(err = %e, "Failed to start consensus state");
                return false;
            }
            reactor.set_wait_sync(false);
            cs.propagator().start_processing();
            reactor.set_snapshot(snapshot_of(cs));
            reactor.broadcast_current_round_step();
        }
        Input::Stop => return false,
    }
    true
}

/// Runs the state machine until a failure, [`Input::Stop`], a disconnected
/// input channel or `on_event` returning [`Flow::Stop`]. `cs` must already
/// be started unless the reactor waits for sync.
pub fn run(
    cs: &mut ConsensusState,
    reactor: &Arc<Reactor>,
    inputs: &Receiver<Input>,
    mut on_event: impl FnMut(&ConsensusState, &Event) -> Flow,
) -> Result<(), String> {
    const MAX_WAIT_NS: i64 = 100_000_000;
    reactor.set_snapshot(snapshot_of(cs));
    loop {
        let now = crate::ticker::system_now();
        let wait = cs.next_timeout().map_or(MAX_WAIT_NS, |d| {
            nodestore::state::sub_times(d, now).clamp(0, MAX_WAIT_NS)
        });
        let mut dirty = false;
        match inputs.recv_timeout(Duration::from_nanos(u64::try_from(wait).unwrap_or(0))) {
            Ok(i) => {
                dirty = true;
                if !handle(cs, reactor, i) {
                    return Ok(());
                }
                for _ in 0..1024 {
                    let Ok(i) = inputs.try_recv() else { break };
                    if !handle(cs, reactor, i) {
                        return Ok(());
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let steps = cs.n_steps;
        cs.tick();
        let events = cs.take_events();
        if dirty || !events.is_empty() || steps != cs.n_steps {
            reactor.set_snapshot(snapshot_of(cs));
        }
        let mut stop = false;
        for ev in &events {
            dispatch(reactor, ev);
            if on_event(cs, ev) == Flow::Stop {
                stop = true;
            }
        }
        if let Some(f) = cs.failure() {
            return Err(format!("CONSENSUS FAILURE: {f}"));
        }
        if stop {
            return Ok(());
        }
    }
}
