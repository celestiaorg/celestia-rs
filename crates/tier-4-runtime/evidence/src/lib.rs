//! Exact port of celestia-core v0.42.3 `evidence/` (ADR 0003).
//!
//! - [`pool`]: the evidence [`Pool`] on a [`kvdb::KvBackend`] (Go's key
//!   layout: `0x01 ‖ "%016X/%X"` pending, `0x00 ‖ …` committed), with the
//!   consensus buffer of conflicting votes turned into
//!   `DuplicateVoteEvidence` once their height is committed, pending /
//!   committed tracking, expiry by max age (blocks *and* duration) and
//!   recovery of pending evidence after a restart. [`Pool`] implements
//!   [`nodestore::EvidencePool`] for block execution and consensus.
//! - [`verify`]: `verify` against the stored headers, commits and
//!   historical validator sets: `VerifyDuplicateVote` and
//!   `VerifyLightClientAttack` (lunatic, equivocation and amnesia attacks,
//!   forward lunatic attacks, ABCI byzantine-validator checks) using the
//!   `ctypes` commit verification.
//! - [`reactor`]: gossip on channel 0x38 — every peer is sent each piece
//!   of pending evidence once its consensus height is past the evidence
//!   height (and within the max age), and everything again every 10 s.

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability.
#![allow(clippy::too_many_lines)]

pub mod pool;
pub mod reactor;
pub mod verify;

pub use pool::{AddError, Pool};
pub use reactor::{EVIDENCE_CHANNEL, Reactor};
pub use verify::{verify_duplicate_vote, verify_light_client_attack};
