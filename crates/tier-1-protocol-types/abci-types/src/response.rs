//! ABCI response envelope types.

use crate::{Error, Event, ExecTxResult, Result, Snapshot, ValidatorUpdate, proto};
use core::fmt;
use prost::Message;

/// ABCI response envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Response variant.
    pub value: ResponseValue,
}

/// ABCI response variants.
#[derive(Clone, PartialEq)]
pub enum ResponseValue {
    /// Exception response.
    Exception(proto::abci::ResponseException),
    /// Echo response.
    Echo(proto::abci::ResponseEcho),
    /// Flush response.
    Flush,
    /// Info response.
    Info(proto::abci::ResponseInfo),
    /// InitChain response.
    InitChain(proto::abci::ResponseInitChain),
    /// Query response.
    Query(proto::abci::ResponseQuery),
    /// CheckTx response.
    CheckTx(proto::abci::ResponseCheckTx),
    /// Commit response.
    Commit(proto::abci::ResponseCommit),
    /// ListSnapshots response.
    ListSnapshots(proto::abci::ResponseListSnapshots),
    /// OfferSnapshot response.
    OfferSnapshot(proto::abci::ResponseOfferSnapshot),
    /// LoadSnapshotChunk response.
    LoadSnapshotChunk(proto::abci::ResponseLoadSnapshotChunk),
    /// ApplySnapshotChunk response.
    ApplySnapshotChunk(proto::abci::ResponseApplySnapshotChunk),
    /// PrepareProposal response.
    PrepareProposal(proto::abci::ResponsePrepareProposal),
    /// ProcessProposal response.
    ProcessProposal(proto::abci::ResponseProcessProposal),
    /// ExtendVote response.
    ExtendVote(proto::abci::ResponseExtendVote),
    /// VerifyVoteExtension response.
    VerifyVoteExtension(proto::abci::ResponseVerifyVoteExtension),
    /// FinalizeBlock response.
    FinalizeBlock(proto::abci::ResponseFinalizeBlock),
}

impl Eq for ResponseValue {}

impl fmt::Debug for ResponseValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Exception(_) => "Exception",
            Self::Echo(_) => "Echo",
            Self::Flush => "Flush",
            Self::Info(_) => "Info",
            Self::InitChain(_) => "InitChain",
            Self::Query(_) => "Query",
            Self::CheckTx(_) => "CheckTx",
            Self::Commit(_) => "Commit",
            Self::ListSnapshots(_) => "ListSnapshots",
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

impl Response {
    /// Perform stateless response validation.
    pub fn validate_basic(&self) -> Result<()> {
        match &self.value {
            ResponseValue::Exception(_)
            | ResponseValue::Echo(_)
            | ResponseValue::Flush
            | ResponseValue::Info(_)
            | ResponseValue::Query(_)
            | ResponseValue::Commit(_)
            | ResponseValue::LoadSnapshotChunk(_)
            | ResponseValue::PrepareProposal(_)
            | ResponseValue::ExtendVote(_) => Ok(()),
            ResponseValue::InitChain(response) => validate_init_chain(response),
            ResponseValue::CheckTx(response) => validate_check_tx(response),
            ResponseValue::ListSnapshots(response) => validate_list_snapshots(response),
            ResponseValue::OfferSnapshot(response) => validate_offer_snapshot(*response),
            ResponseValue::ApplySnapshotChunk(response) => validate_apply_snapshot_chunk(response),
            ResponseValue::ProcessProposal(response) => validate_process_proposal(*response),
            ResponseValue::VerifyVoteExtension(response) => {
                validate_verify_vote_extension(*response)
            }
            ResponseValue::FinalizeBlock(response) => validate_finalize_block(response),
        }
    }

    /// Convert protobuf into a domain response envelope.
    pub fn from_proto(proto: proto::abci::Response) -> Result<Self> {
        let value = match proto.value.ok_or(Error::InvalidResponse)? {
            proto::abci::response::Value::Exception(value) => ResponseValue::Exception(value),
            proto::abci::response::Value::Echo(value) => ResponseValue::Echo(value),
            proto::abci::response::Value::Flush(_) => ResponseValue::Flush,
            proto::abci::response::Value::Info(value) => ResponseValue::Info(value),
            proto::abci::response::Value::InitChain(value) => ResponseValue::InitChain(value),
            proto::abci::response::Value::Query(value) => ResponseValue::Query(value),
            proto::abci::response::Value::CheckTx(value) => ResponseValue::CheckTx(value),
            proto::abci::response::Value::Commit(value) => ResponseValue::Commit(value),
            proto::abci::response::Value::ListSnapshots(value) => {
                ResponseValue::ListSnapshots(value)
            }
            proto::abci::response::Value::OfferSnapshot(value) => {
                ResponseValue::OfferSnapshot(value)
            }
            proto::abci::response::Value::LoadSnapshotChunk(value) => {
                ResponseValue::LoadSnapshotChunk(value)
            }
            proto::abci::response::Value::ApplySnapshotChunk(value) => {
                ResponseValue::ApplySnapshotChunk(value)
            }
            proto::abci::response::Value::PrepareProposal(value) => {
                ResponseValue::PrepareProposal(value)
            }
            proto::abci::response::Value::ProcessProposal(value) => {
                ResponseValue::ProcessProposal(value)
            }
            proto::abci::response::Value::ExtendVote(value) => ResponseValue::ExtendVote(value),
            proto::abci::response::Value::VerifyVoteExtension(value) => {
                ResponseValue::VerifyVoteExtension(value)
            }
            proto::abci::response::Value::FinalizeBlock(value) => {
                ResponseValue::FinalizeBlock(value)
            }
        };
        let response = Self { value };
        response.validate_basic()?;
        Ok(response)
    }

    /// Convert this response envelope to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Response {
        let value = match &self.value {
            ResponseValue::Exception(value) => {
                proto::abci::response::Value::Exception(value.clone())
            }
            ResponseValue::Echo(value) => proto::abci::response::Value::Echo(value.clone()),
            ResponseValue::Flush => {
                proto::abci::response::Value::Flush(proto::abci::ResponseFlush {})
            }
            ResponseValue::Info(value) => proto::abci::response::Value::Info(value.clone()),
            ResponseValue::InitChain(value) => {
                proto::abci::response::Value::InitChain(value.clone())
            }
            ResponseValue::Query(value) => proto::abci::response::Value::Query(value.clone()),
            ResponseValue::CheckTx(value) => proto::abci::response::Value::CheckTx(value.clone()),
            ResponseValue::Commit(value) => proto::abci::response::Value::Commit(*value),
            ResponseValue::ListSnapshots(value) => {
                proto::abci::response::Value::ListSnapshots(value.clone())
            }
            ResponseValue::OfferSnapshot(value) => {
                proto::abci::response::Value::OfferSnapshot(*value)
            }
            ResponseValue::LoadSnapshotChunk(value) => {
                proto::abci::response::Value::LoadSnapshotChunk(value.clone())
            }
            ResponseValue::ApplySnapshotChunk(value) => {
                proto::abci::response::Value::ApplySnapshotChunk(value.clone())
            }
            ResponseValue::PrepareProposal(value) => {
                proto::abci::response::Value::PrepareProposal(value.clone())
            }
            ResponseValue::ProcessProposal(value) => {
                proto::abci::response::Value::ProcessProposal(*value)
            }
            ResponseValue::ExtendVote(value) => {
                proto::abci::response::Value::ExtendVote(value.clone())
            }
            ResponseValue::VerifyVoteExtension(value) => {
                proto::abci::response::Value::VerifyVoteExtension(*value)
            }
            ResponseValue::FinalizeBlock(value) => {
                proto::abci::response::Value::FinalizeBlock(value.clone())
            }
        };
        proto::abci::Response { value: Some(value) }
    }

    /// Decode a protobuf response envelope.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_proto(
            proto::abci::Response::decode(bytes)
                .map_err(|error| Error::Decode(error.to_string()))?,
        )
    }

    /// Encode this response envelope as protobuf bytes.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        Ok(self.to_proto().encode_to_vec())
    }
}

fn validate_init_chain(response: &proto::abci::ResponseInitChain) -> Result<()> {
    for validator in &response.validators {
        ValidatorUpdate::from_proto(validator.clone())?;
    }
    Ok(())
}

fn validate_check_tx(response: &proto::abci::ResponseCheckTx) -> Result<()> {
    validate_events(&response.events)
}

fn validate_list_snapshots(response: &proto::abci::ResponseListSnapshots) -> Result<()> {
    for snapshot in &response.snapshots {
        Snapshot::from_proto(snapshot.clone())?;
    }
    Ok(())
}

fn validate_offer_snapshot(response: proto::abci::ResponseOfferSnapshot) -> Result<()> {
    match proto::abci::response_offer_snapshot::Result::try_from(response.result)
        .map_err(|_| Error::InvalidStatus)?
    {
        proto::abci::response_offer_snapshot::Result::Unknown => Err(Error::InvalidStatus),
        _ => Ok(()),
    }
}

fn validate_apply_snapshot_chunk(response: &proto::abci::ResponseApplySnapshotChunk) -> Result<()> {
    match proto::abci::response_apply_snapshot_chunk::Result::try_from(response.result)
        .map_err(|_| Error::InvalidStatus)?
    {
        proto::abci::response_apply_snapshot_chunk::Result::Unknown => Err(Error::InvalidStatus),
        _ => Ok(()),
    }
}

fn validate_process_proposal(response: proto::abci::ResponseProcessProposal) -> Result<()> {
    match proto::abci::response_process_proposal::ProposalStatus::try_from(response.status)
        .map_err(|_| Error::InvalidStatus)?
    {
        proto::abci::response_process_proposal::ProposalStatus::Unknown => {
            Err(Error::InvalidStatus)
        }
        _ => Ok(()),
    }
}

fn validate_verify_vote_extension(
    response: proto::abci::ResponseVerifyVoteExtension,
) -> Result<()> {
    match proto::abci::response_verify_vote_extension::VerifyStatus::try_from(response.status)
        .map_err(|_| Error::InvalidStatus)?
    {
        proto::abci::response_verify_vote_extension::VerifyStatus::Unknown => {
            Err(Error::InvalidStatus)
        }
        _ => Ok(()),
    }
}

fn validate_finalize_block(response: &proto::abci::ResponseFinalizeBlock) -> Result<()> {
    validate_events(&response.events)?;
    for result in &response.tx_results {
        ExecTxResult::from_proto(result.clone())?;
    }
    for validator in &response.validator_updates {
        ValidatorUpdate::from_proto(validator.clone())?;
    }
    Ok(())
}

fn validate_events(events: &[proto::abci::Event]) -> Result<()> {
    for event in events {
        Event::from_proto(event.clone())?;
    }
    Ok(())
}
