//! ABCI request envelope types.

use crate::validation::{validate_address_bytes, validate_hash_bytes, validate_positive_height};
use crate::{
    CommitInfo, Error, ExtendedCommitInfo, Misbehavior, Result, Snapshot, ValidatorUpdate, proto,
};
use core::fmt;
use prost::Message;

/// ABCI request envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Request variant.
    pub value: RequestValue,
}

/// ABCI request variants.
#[derive(Clone, PartialEq)]
pub enum RequestValue {
    /// Echo request.
    Echo(proto::abci::RequestEcho),
    /// Flush request.
    Flush,
    /// Info request.
    Info(proto::abci::RequestInfo),
    /// InitChain request.
    InitChain(proto::abci::RequestInitChain),
    /// Query request.
    Query(proto::abci::RequestQuery),
    /// CheckTx request.
    CheckTx(proto::abci::RequestCheckTx),
    /// Commit request.
    Commit,
    /// ListSnapshots request.
    ListSnapshots,
    /// OfferSnapshot request.
    OfferSnapshot(proto::abci::RequestOfferSnapshot),
    /// LoadSnapshotChunk request.
    LoadSnapshotChunk(proto::abci::RequestLoadSnapshotChunk),
    /// ApplySnapshotChunk request.
    ApplySnapshotChunk(proto::abci::RequestApplySnapshotChunk),
    /// PrepareProposal request.
    PrepareProposal(proto::abci::RequestPrepareProposal),
    /// ProcessProposal request.
    ProcessProposal(proto::abci::RequestProcessProposal),
    /// ExtendVote request.
    ExtendVote(proto::abci::RequestExtendVote),
    /// VerifyVoteExtension request.
    VerifyVoteExtension(proto::abci::RequestVerifyVoteExtension),
    /// FinalizeBlock request.
    FinalizeBlock(proto::abci::RequestFinalizeBlock),
}

impl Eq for RequestValue {}

impl fmt::Debug for RequestValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Echo(_) => "Echo",
            Self::Flush => "Flush",
            Self::Info(_) => "Info",
            Self::InitChain(_) => "InitChain",
            Self::Query(_) => "Query",
            Self::CheckTx(_) => "CheckTx",
            Self::Commit => "Commit",
            Self::ListSnapshots => "ListSnapshots",
            Self::OfferSnapshot(_) => "OfferSnapshot",
            Self::LoadSnapshotChunk(_) => "LoadSnapshotChunk",
            Self::ApplySnapshotChunk(_) => "ApplySnapshotChunk",
            Self::PrepareProposal(_) => "PrepareProposal",
            Self::ProcessProposal(_) => "ProcessProposal",
            Self::ExtendVote(_) => "ExtendVote",
            Self::VerifyVoteExtension(_) => "VerifyVoteExtension",
            Self::FinalizeBlock(_) => "FinalizeBlock",
        };
        f.write_str(name)
    }
}

impl Request {
    /// Perform stateless request validation.
    pub fn validate_basic(&self) -> Result<()> {
        match &self.value {
            RequestValue::Echo(_)
            | RequestValue::Flush
            | RequestValue::Info(_)
            | RequestValue::Query(_)
            | RequestValue::Commit
            | RequestValue::ListSnapshots
            | RequestValue::ApplySnapshotChunk(_) => Ok(()),
            RequestValue::InitChain(request) => validate_init_chain(request),
            RequestValue::CheckTx(request) => validate_check_tx(request),
            RequestValue::OfferSnapshot(request) => validate_offer_snapshot(request),
            RequestValue::LoadSnapshotChunk(request) => validate_load_snapshot_chunk(request),
            RequestValue::PrepareProposal(request) => validate_prepare_proposal(request),
            RequestValue::ProcessProposal(request) => validate_process_proposal(request),
            RequestValue::ExtendVote(request) => validate_extend_vote(request),
            RequestValue::VerifyVoteExtension(request) => validate_verify_vote_extension(request),
            RequestValue::FinalizeBlock(request) => validate_finalize_block(request),
        }
    }

    /// Convert protobuf into a domain request envelope.
    pub fn from_proto(proto: proto::abci::Request) -> Result<Self> {
        let value = match proto.value.ok_or(Error::InvalidRequest)? {
            proto::abci::request::Value::Echo(value) => RequestValue::Echo(value),
            proto::abci::request::Value::Flush(_) => RequestValue::Flush,
            proto::abci::request::Value::Info(value) => RequestValue::Info(value),
            proto::abci::request::Value::InitChain(value) => RequestValue::InitChain(value),
            proto::abci::request::Value::Query(value) => RequestValue::Query(value),
            proto::abci::request::Value::CheckTx(value) => RequestValue::CheckTx(value),
            proto::abci::request::Value::Commit(_) => RequestValue::Commit,
            proto::abci::request::Value::ListSnapshots(_) => RequestValue::ListSnapshots,
            proto::abci::request::Value::OfferSnapshot(value) => RequestValue::OfferSnapshot(value),
            proto::abci::request::Value::LoadSnapshotChunk(value) => {
                RequestValue::LoadSnapshotChunk(value)
            }
            proto::abci::request::Value::ApplySnapshotChunk(value) => {
                RequestValue::ApplySnapshotChunk(value)
            }
            proto::abci::request::Value::PrepareProposal(value) => {
                RequestValue::PrepareProposal(value)
            }
            proto::abci::request::Value::ProcessProposal(value) => {
                RequestValue::ProcessProposal(value)
            }
            proto::abci::request::Value::ExtendVote(value) => RequestValue::ExtendVote(value),
            proto::abci::request::Value::VerifyVoteExtension(value) => {
                RequestValue::VerifyVoteExtension(value)
            }
            proto::abci::request::Value::FinalizeBlock(value) => RequestValue::FinalizeBlock(value),
        };
        let request = Self { value };
        request.validate_basic()?;
        Ok(request)
    }

    /// Convert this request envelope to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Request {
        let value = match &self.value {
            RequestValue::Echo(value) => proto::abci::request::Value::Echo(value.clone()),
            RequestValue::Flush => proto::abci::request::Value::Flush(proto::abci::RequestFlush {}),
            RequestValue::Info(value) => proto::abci::request::Value::Info(value.clone()),
            RequestValue::InitChain(value) => proto::abci::request::Value::InitChain(value.clone()),
            RequestValue::Query(value) => proto::abci::request::Value::Query(value.clone()),
            RequestValue::CheckTx(value) => proto::abci::request::Value::CheckTx(value.clone()),
            RequestValue::Commit => {
                proto::abci::request::Value::Commit(proto::abci::RequestCommit {})
            }
            RequestValue::ListSnapshots => {
                proto::abci::request::Value::ListSnapshots(proto::abci::RequestListSnapshots {})
            }
            RequestValue::OfferSnapshot(value) => {
                proto::abci::request::Value::OfferSnapshot(value.clone())
            }
            RequestValue::LoadSnapshotChunk(value) => {
                proto::abci::request::Value::LoadSnapshotChunk(*value)
            }
            RequestValue::ApplySnapshotChunk(value) => {
                proto::abci::request::Value::ApplySnapshotChunk(value.clone())
            }
            RequestValue::PrepareProposal(value) => {
                proto::abci::request::Value::PrepareProposal(value.clone())
            }
            RequestValue::ProcessProposal(value) => {
                proto::abci::request::Value::ProcessProposal(value.clone())
            }
            RequestValue::ExtendVote(value) => {
                proto::abci::request::Value::ExtendVote(value.clone())
            }
            RequestValue::VerifyVoteExtension(value) => {
                proto::abci::request::Value::VerifyVoteExtension(value.clone())
            }
            RequestValue::FinalizeBlock(value) => {
                proto::abci::request::Value::FinalizeBlock(value.clone())
            }
        };
        proto::abci::Request { value: Some(value) }
    }

    /// Decode a protobuf request envelope.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_proto(
            proto::abci::Request::decode(bytes)
                .map_err(|error| Error::Decode(error.to_string()))?,
        )
    }

    /// Encode this request envelope as protobuf bytes.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        Ok(self.to_proto().encode_to_vec())
    }
}

fn validate_init_chain(request: &proto::abci::RequestInitChain) -> Result<()> {
    for validator in &request.validators {
        ValidatorUpdate::from_proto(validator.clone())?;
    }
    Ok(())
}

fn validate_check_tx(request: &proto::abci::RequestCheckTx) -> Result<()> {
    proto::abci::CheckTxType::try_from(request.r#type)
        .map(|_| ())
        .map_err(|_| Error::InvalidStatus)
}

fn validate_offer_snapshot(request: &proto::abci::RequestOfferSnapshot) -> Result<()> {
    request
        .snapshot
        .clone()
        .map(Snapshot::from_proto)
        .transpose()?
        .ok_or(Error::InvalidSnapshot)?;
    Ok(())
}

fn validate_load_snapshot_chunk(request: &proto::abci::RequestLoadSnapshotChunk) -> Result<()> {
    if request.height == 0 || request.format == 0 {
        return Err(Error::InvalidSnapshot);
    }
    Ok(())
}

fn validate_prepare_proposal(request: &proto::abci::RequestPrepareProposal) -> Result<()> {
    validate_positive_height(request.height)?;
    validate_hash_bytes(&request.next_validators_hash)?;
    validate_address_bytes(&request.proposer_address)?;
    if let Some(commit) = request.local_last_commit.clone() {
        ExtendedCommitInfo::from_proto(commit)?;
    }
    validate_misbehavior(&request.misbehavior)
}

fn validate_process_proposal(request: &proto::abci::RequestProcessProposal) -> Result<()> {
    validate_positive_height(request.height)?;
    validate_hash_bytes(&request.hash)?;
    validate_hash_bytes(&request.next_validators_hash)?;
    validate_address_bytes(&request.proposer_address)?;
    if let Some(commit) = request.proposed_last_commit.clone() {
        CommitInfo::from_proto(commit)?;
    }
    validate_misbehavior(&request.misbehavior)
}

fn validate_extend_vote(request: &proto::abci::RequestExtendVote) -> Result<()> {
    validate_positive_height(request.height)?;
    validate_hash_bytes(&request.hash)?;
    validate_hash_bytes(&request.next_validators_hash)?;
    validate_address_bytes(&request.proposer_address)?;
    if let Some(commit) = request.proposed_last_commit.clone() {
        CommitInfo::from_proto(commit)?;
    }
    validate_misbehavior(&request.misbehavior)
}

fn validate_verify_vote_extension(request: &proto::abci::RequestVerifyVoteExtension) -> Result<()> {
    validate_positive_height(request.height)?;
    validate_hash_bytes(&request.hash)?;
    validate_address_bytes(&request.validator_address)
}

fn validate_finalize_block(request: &proto::abci::RequestFinalizeBlock) -> Result<()> {
    validate_positive_height(request.height)?;
    validate_hash_bytes(&request.hash)?;
    validate_hash_bytes(&request.next_validators_hash)?;
    validate_address_bytes(&request.proposer_address)?;
    if let Some(commit) = request.decided_last_commit.clone() {
        CommitInfo::from_proto(commit)?;
    }
    validate_misbehavior(&request.misbehavior)
}

fn validate_misbehavior(values: &[proto::abci::Misbehavior]) -> Result<()> {
    for value in values {
        Misbehavior::from_proto(value.clone())?;
    }
    Ok(())
}
