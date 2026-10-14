//! Exact port of celestia-core v0.42.3 `types/` for the Rust consensus node
//! (ADR 0003).
//!
//! Everything consensus-visible matches Go byte for byte and is checked
//! against Go-generated vectors (`conformance/ctypesvectors`): header and
//! block hashes, part sets and proofs, canonical vote/proposal/extension
//! sign bytes, commit verification outcomes (including which error the
//! batch and single paths report), validator-set proposer-priority
//! evolution and change sets, evidence hashes, consensus-param hashes and
//! update validation, tx hashing (celestia's index-wrapper / blob-tx aware
//! `Tx.Hash`), results hashes and `ValidateBasic` error strings.
//!
//! Where Go panics this crate returns an [`Error`] carrying the panic
//! message. Go methods that memoize into the value (`Block.Hash`,
//! `Data.Hash`, `Block.ValidateBasic`) take `&mut self`.
//!
//! Not ported here: `VoteSet`/`BitArray` (consensus crate), the
//! Reed-Solomon part-set `Encode`/`Decode` (propagation crate),
//! `GenesisDoc`, events/event bus, `PrivValidator`, and the share/row proof
//! RPC types.

#![forbid(unsafe_code)]

pub mod block;
pub mod block_id;
pub mod block_meta;
pub mod canonical;
pub mod commit;
pub mod error;
pub mod evidence;
pub mod gofmt;
pub mod header;
pub mod light;
pub mod merkle_proof;
pub mod params;
pub mod part_set;
pub mod proposal;
pub mod pubkey;
pub mod results;
pub mod time;
pub mod tx;
pub mod validation;
pub mod validator;
pub mod validator_set;
pub mod vote;

/// The celestia-core protobuf bindings used by this crate.
pub use celestia_core_proto::tendermint as proto;

pub use block::{Block, Data, EvidenceData, max_data_bytes, max_data_bytes_no_evidence};
pub use block_id::{BlockId, validate_hash};
pub use block_meta::BlockMeta;
pub use commit::{
    BLOCK_ID_FLAG_ABSENT, BLOCK_ID_FLAG_COMMIT, BLOCK_ID_FLAG_NIL, Commit, CommitSig,
    ExtendedCommit, ExtendedCommitSig, max_commit_bytes,
};
pub use error::{Error, Result};
pub use evidence::{DuplicateVoteEvidence, Evidence, LightClientAttackEvidence};
pub use header::{Consensus, Header};
pub use light::{LightBlock, SignedHeader};
pub use merkle_proof::Proof;
pub use params::ConsensusParams;
pub use part_set::{BLOCK_PART_SIZE_BYTES, Part, PartSet, PartSetHeader, TxPosition};
pub use proposal::Proposal;
pub use pubkey::PubKey;
pub use results::results_hash;
pub use time::Time;
pub use tx::{Tx, txs_hash};
pub use validation::{
    Fraction, verify_commit, verify_commit_light, verify_commit_light_all_signatures,
    verify_commit_light_trusting, verify_commit_light_trusting_all_signatures,
};
pub use validator::Validator;
pub use validator_set::ValidatorSet;
pub use vote::Vote;
