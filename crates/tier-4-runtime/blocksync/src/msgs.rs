//! Wire messages (`blocksync/msgs.go`, `blocksync/errors.go`): message
//! validation and the allocation-bounded signature-count precheck that runs
//! on every received message before it is decoded.

use std::fmt;

use celestia_core_proto::tendermint::blocksync as bcproto;
use ctypes::commit::MAX_VOTES_COUNT;
use prost::Message as _;

/// `BlocksyncChannel`.
pub const BLOCKSYNC_CHANNEL: u8 = 0x40;

/// `BlockResponseMessagePrefixSize`.
pub const BLOCK_RESPONSE_MESSAGE_PREFIX_SIZE: usize = 4;
/// `BlockResponseMessageFieldKeySize`.
pub const BLOCK_RESPONSE_MESSAGE_FIELD_KEY_SIZE: usize = 1;

/// `MaxMsgSize`: the largest block plus the response envelope.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const MAX_MSG_SIZE: usize = ctypes::part_set::MAX_BLOCK_SIZE_BYTES as usize
    + BLOCK_RESPONSE_MESSAGE_PREFIX_SIZE
    + BLOCK_RESPONSE_MESSAGE_FIELD_KEY_SIZE;

/// `maxEvidenceSigs`.
pub const MAX_EVIDENCE_SIGS: usize = MAX_VOTES_COUNT * 100;

/// Errors of message validation (`errors.go` and the precheck errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgError {
    /// `ErrNilMessage`.
    Nil,
    /// `ErrInvalidHeight`.
    InvalidHeight {
        /// Height.
        height: i64,
        /// Reason.
        reason: String,
    },
    /// `ErrInvalidBase`.
    InvalidBase {
        /// Base.
        base: i64,
        /// Reason.
        reason: String,
    },
    /// `ErrUnknownMessageType`.
    UnknownMessageType,
    /// The precheck: malformed bytes or too many signatures.
    Precheck(String),
}

impl fmt::Display for MsgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nil => f.write_str("message cannot be nil"),
            Self::InvalidHeight { height, reason } => {
                write!(f, "invalid height {height}: {reason}")
            }
            Self::InvalidBase { base, reason } => write!(f, "invalid base {base}: {reason}"),
            Self::UnknownMessageType => f.write_str("unknown message type"),
            Self::Precheck(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for MsgError {}

fn too_many_sigs(what: &str, got: usize) -> MsgError {
    MsgError::Precheck(format!(
        "too many signatures (max: {MAX_VOTES_COUNT}) ({what}got {got})"
    ))
}

/// `validateBlockSyncBytes`: rejects block responses that encode too many
/// commit / extended-commit / evidence signatures before the full message
/// is decoded (the `SigCount*` stubs keep only the repeated fields).
pub fn validate_block_sync_bytes(msg: &[u8]) -> Result<(), MsgError> {
    let stub = celestia_core_proto::gogo_decode::<bcproto::SigCountMessage>(msg)
        .map_err(|e| MsgError::Precheck(format!("malformed blocksync message {e}")))?;
    match stub.block_response {
        None => Ok(()),
        Some(br) => validate_max_votes(&br),
    }
}

/// `validateMaxVotes`.
pub fn validate_max_votes(br: &bcproto::SigCountBlockResponse) -> Result<(), MsgError> {
    let mut commit_sigs = 0;
    let mut ext_sigs = 0;
    if let Some(b) = &br.block {
        if let Some(lc) = &b.last_commit {
            commit_sigs = lc.signatures.len();
        }
        validate_evidence_sigs(b.evidence.as_ref())?;
    }
    if let Some(ec) = &br.ext_commit {
        ext_sigs = ec.extended_signatures.len();
    }
    if commit_sigs > MAX_VOTES_COUNT {
        return Err(too_many_sigs("", commit_sigs));
    }
    if ext_sigs > MAX_VOTES_COUNT {
        return Err(MsgError::Precheck(format!(
            "too many extended signatures (max: {MAX_VOTES_COUNT}) (got {ext_sigs})"
        )));
    }
    Ok(())
}

/// `validateEvidenceSigs`.
fn validate_evidence_sigs(el: Option<&bcproto::SigCountEvidenceList>) -> Result<(), MsgError> {
    let Some(el) = el else { return Ok(()) };
    let too_many_ev = |got: usize| {
        MsgError::Precheck(format!(
            "too many evidence signatures (max: {MAX_EVIDENCE_SIGS}) (evidence got {got})"
        ))
    };
    let mut total = el.evidence.len();
    if total > MAX_EVIDENCE_SIGS {
        return Err(too_many_ev(total));
    }
    for ev in &el.evidence {
        let Some(lcae) = &ev.light_client_attack_evidence else {
            continue;
        };
        let n = lcae.byzantine_validators.len();
        if n > MAX_VOTES_COUNT {
            return Err(too_many_sigs("byzantine validators ", n));
        }
        total += n;
        if let Some(cb) = &lcae.conflicting_block {
            if let Some(c) = cb.signed_header.as_ref().and_then(|sh| sh.commit.as_ref()) {
                let n = c.signatures.len();
                if n > MAX_VOTES_COUNT {
                    return Err(too_many_sigs("evidence commit ", n));
                }
                total += n;
            }
            if let Some(vs) = &cb.validator_set {
                let n = vs.validators.len();
                if n > MAX_VOTES_COUNT {
                    return Err(too_many_sigs("validator set ", n));
                }
                total += n;
            }
        }
        if total > MAX_EVIDENCE_SIGS {
            return Err(too_many_ev(total));
        }
    }
    Ok(())
}

/// `ValidateMsg`.
pub fn validate_msg(msg: &bcproto::Message) -> Result<(), MsgError> {
    use bcproto::message::Sum;
    match &msg.sum {
        None => Err(MsgError::Nil),
        Some(Sum::BlockRequest(m)) if m.height < 0 => Err(MsgError::InvalidHeight {
            height: m.height,
            reason: "negative height".into(),
        }),
        Some(Sum::NoBlockResponse(m)) if m.height < 0 => Err(MsgError::InvalidHeight {
            height: m.height,
            reason: "negative height".into(),
        }),
        Some(Sum::StatusResponse(m)) => {
            if m.base < 0 {
                return Err(MsgError::InvalidBase {
                    base: m.base,
                    reason: "negative base".into(),
                });
            }
            if m.height < 0 {
                return Err(MsgError::InvalidHeight {
                    height: m.height,
                    reason: "negative height".into(),
                });
            }
            if m.base > m.height {
                return Err(MsgError::InvalidHeight {
                    height: m.height,
                    reason: format!("base {} cannot be greater than height", m.base),
                });
            }
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

/// Encodes a message on the channel.
#[must_use]
pub fn encode(sum: bcproto::message::Sum) -> Vec<u8> {
    bcproto::Message { sum: Some(sum) }.encode_to_vec()
}

/// `StatusResponse{Height, Base}`.
#[must_use]
pub fn status_response(base: i64, height: i64) -> Vec<u8> {
    encode(bcproto::message::Sum::StatusResponse(
        bcproto::StatusResponse { height, base },
    ))
}

/// `StatusRequest{}`.
#[must_use]
pub fn status_request() -> Vec<u8> {
    encode(bcproto::message::Sum::StatusRequest(
        bcproto::StatusRequest {},
    ))
}

/// `BlockRequest{Height}`.
#[must_use]
pub fn block_request(height: i64) -> Vec<u8> {
    encode(bcproto::message::Sum::BlockRequest(bcproto::BlockRequest {
        height,
    }))
}

/// `NoBlockResponse{Height}`.
#[must_use]
pub fn no_block_response(height: i64) -> Vec<u8> {
    encode(bcproto::message::Sum::NoBlockResponse(
        bcproto::NoBlockResponse { height },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcproto::message::Sum;

    #[test]
    fn validate_basic() {
        // TestBcBlockRequestMessageValidateBasic & co.
        let req = |h| bcproto::Message {
            sum: Some(Sum::BlockRequest(bcproto::BlockRequest { height: h })),
        };
        assert!(validate_msg(&req(0)).is_ok());
        assert!(validate_msg(&req(1)).is_ok());
        assert_eq!(
            validate_msg(&req(-1)).unwrap_err().to_string(),
            "invalid height -1: negative height"
        );
        let nb = |h| bcproto::Message {
            sum: Some(Sum::NoBlockResponse(bcproto::NoBlockResponse { height: h })),
        };
        assert!(validate_msg(&nb(0)).is_ok());
        assert!(validate_msg(&nb(-1)).is_err());
        let st = |base, height| bcproto::Message {
            sum: Some(Sum::StatusResponse(bcproto::StatusResponse {
                height,
                base,
            })),
        };
        assert!(validate_msg(&st(0, 0)).is_ok());
        assert!(validate_msg(&st(0, 1)).is_ok());
        assert_eq!(
            validate_msg(&st(-1, 0)).unwrap_err().to_string(),
            "invalid base -1: negative base"
        );
        assert!(validate_msg(&st(0, -1)).is_err());
        assert_eq!(
            validate_msg(&st(2, 1)).unwrap_err().to_string(),
            "invalid height 1: base 2 cannot be greater than height"
        );
        assert_eq!(
            validate_msg(&bcproto::Message { sum: None }).unwrap_err(),
            MsgError::Nil
        );
    }

    #[test]
    fn message_vectors() {
        // TestBlocksyncMessageVectors (Go's expected encodings).
        assert_eq!(hex::encode(block_request(1)), "0a020801");
        assert_eq!(
            hex::encode(block_request(i64::MAX)),
            "0a0a08ffffffffffffffff7f"
        );
        assert_eq!(hex::encode(no_block_response(1)), "12020801");
        assert_eq!(
            hex::encode(no_block_response(i64::MAX)),
            "120a08ffffffffffffffff7f"
        );
        assert_eq!(hex::encode(status_request()), "2200");
        assert_eq!(hex::encode(status_response(0, 1)), "2a020801");
        assert_eq!(
            hex::encode(status_response(0, i64::MAX)),
            "2a0a08ffffffffffffffff7f"
        );
    }

    fn sigs(n: usize) -> Vec<ctypes::proto::types::CommitSig> {
        (0..n)
            .map(|_| ctypes::proto::types::CommitSig::default())
            .collect()
    }

    fn block_response_bytes(commit_sigs: usize, ext_sigs: usize) -> Vec<u8> {
        let block = ctypes::proto::types::Block {
            last_commit: Some(ctypes::proto::types::Commit {
                signatures: sigs(commit_sigs),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ext = ctypes::proto::types::ExtendedCommit {
            extended_signatures: (0..ext_sigs)
                .map(|_| ctypes::proto::types::ExtendedCommitSig::default())
                .collect(),
            ..Default::default()
        };
        encode(Sum::BlockResponse(bcproto::BlockResponse {
            block: Some(block),
            ext_commit: Some(ext),
        }))
    }

    #[test]
    fn precheck_counts_signatures() {
        // TestValidateBlockSyncBytes.
        assert!(validate_block_sync_bytes(&block_response_bytes(MAX_VOTES_COUNT, 0)).is_ok());
        assert!(validate_block_sync_bytes(&block_response_bytes(0, MAX_VOTES_COUNT)).is_ok());
        let e = validate_block_sync_bytes(&block_response_bytes(MAX_VOTES_COUNT + 1, 0))
            .unwrap_err()
            .to_string();
        assert_eq!(e, "too many signatures (max: 10000) (got 10001)");
        let e = validate_block_sync_bytes(&block_response_bytes(0, MAX_VOTES_COUNT + 1))
            .unwrap_err()
            .to_string();
        assert_eq!(e, "too many extended signatures (max: 10000) (got 10001)");
        // other messages pass, malformed bytes fail
        assert!(validate_block_sync_bytes(&status_request()).is_ok());
        assert!(validate_block_sync_bytes(&block_request(5)).is_ok());
        assert!(validate_block_sync_bytes(&[0x1a, 0x05, 0x01]).is_err());
    }

    #[test]
    fn precheck_merges_duplicated_fields() {
        // TestValidateBlockSyncBytes_DuplicatedFields: repeated occurrences
        // of the BlockResponse field are merged, so their signatures add up.
        let half = block_response_bytes(MAX_VOTES_COUNT / 2 + 1, 0);
        let mut twice = half.clone();
        twice.extend_from_slice(&half);
        assert!(validate_block_sync_bytes(&half).is_ok());
        assert!(validate_block_sync_bytes(&twice).is_err());
    }

    #[test]
    fn precheck_bounds_evidence() {
        use ctypes::proto::types as pb;
        let lcae = |vals: usize, sigs_n: usize, byz: usize| pb::Evidence {
            sum: Some(pb::evidence::Sum::LightClientAttackEvidence(
                pb::LightClientAttackEvidence {
                    conflicting_block: Some(pb::LightBlock {
                        signed_header: Some(pb::SignedHeader {
                            header: None,
                            commit: Some(pb::Commit {
                                signatures: sigs(sigs_n),
                                ..Default::default()
                            }),
                        }),
                        validator_set: Some(pb::ValidatorSet {
                            validators: (0..vals).map(|_| pb::Validator::default()).collect(),
                            ..Default::default()
                        }),
                    }),
                    byzantine_validators: (0..byz).map(|_| pb::Validator::default()).collect(),
                    ..Default::default()
                },
            )),
        };
        let msg = |evs: Vec<pb::Evidence>| {
            encode(Sum::BlockResponse(bcproto::BlockResponse {
                block: Some(pb::Block {
                    evidence: Some(pb::EvidenceList { evidence: evs }),
                    ..Default::default()
                }),
                ext_commit: None,
            }))
        };
        assert!(validate_block_sync_bytes(&msg(vec![lcae(10, 10, 10)])).is_ok());
        assert_eq!(
            validate_block_sync_bytes(&msg(vec![lcae(0, MAX_VOTES_COUNT + 1, 0)]))
                .unwrap_err()
                .to_string(),
            "too many signatures (max: 10000) (evidence commit got 10001)"
        );
        assert_eq!(
            validate_block_sync_bytes(&msg(vec![lcae(MAX_VOTES_COUNT + 1, 0, 0)]))
                .unwrap_err()
                .to_string(),
            "too many signatures (max: 10000) (validator set got 10001)"
        );
        assert_eq!(
            validate_block_sync_bytes(&msg(vec![lcae(0, 0, MAX_VOTES_COUNT + 1)]))
                .unwrap_err()
                .to_string(),
            "too many signatures (max: 10000) (byzantine validators got 10001)"
        );
        // the aggregate bound across evidence items
        let many: Vec<_> = (0..101)
            .map(|_| lcae(MAX_VOTES_COUNT / 2, MAX_VOTES_COUNT / 2, 0))
            .collect();
        assert!(
            validate_block_sync_bytes(&msg(many))
                .unwrap_err()
                .to_string()
                .starts_with("too many evidence signatures (max: 1000000)")
        );
    }
}
