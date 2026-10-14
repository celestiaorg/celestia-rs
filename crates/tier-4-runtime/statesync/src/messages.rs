//! State-sync wire messages (`statesync/messages.go`, proto
//! `tendermint.statesync.Message`).

use celestia_core_proto::tendermint::statesync as pb;
use prost::Message as _;

/// `SnapshotChannel`.
pub const SNAPSHOT_CHANNEL: u8 = 0x60;
/// `ChunkChannel`.
pub const CHUNK_CHANNEL: u8 = 0x61;
/// `snapshotMsgSize`.
pub const SNAPSHOT_MSG_SIZE: usize = 4_000_000;
/// `chunkMsgSize`.
pub const CHUNK_MSG_SIZE: usize = 16_000_000;
/// `recentSnapshots`.
pub const RECENT_SNAPSHOTS: usize = 10;

/// A decoded state-sync message.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// `SnapshotsRequest`.
    SnapshotsRequest,
    /// `SnapshotsResponse`.
    SnapshotsResponse(pb::SnapshotsResponse),
    /// `ChunkRequest`.
    ChunkRequest(pb::ChunkRequest),
    /// `ChunkResponse`.
    ChunkResponse(pb::ChunkResponse),
}

impl Msg {
    /// Wraps and encodes the message (`Message{sum}`).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        use pb::message::Sum;
        let sum = match self {
            Self::SnapshotsRequest => Sum::SnapshotsRequest(pb::SnapshotsRequest {}),
            Self::SnapshotsResponse(m) => Sum::SnapshotsResponse(m.clone()),
            Self::ChunkRequest(m) => Sum::ChunkRequest(*m),
            Self::ChunkResponse(m) => Sum::ChunkResponse(m.clone()),
        };
        pb::Message { sum: Some(sum) }.encode_to_vec()
    }

    /// Decodes and unwraps a message.
    pub fn decode(bz: &[u8]) -> Result<Self, String> {
        use pb::message::Sum;
        let m = pb::Message::decode(bz).map_err(|e| e.to_string())?;
        match m.sum {
            Some(Sum::SnapshotsRequest(_)) => Ok(Self::SnapshotsRequest),
            Some(Sum::SnapshotsResponse(m)) => Ok(Self::SnapshotsResponse(m)),
            Some(Sum::ChunkRequest(m)) => Ok(Self::ChunkRequest(m)),
            Some(Sum::ChunkResponse(m)) => Ok(Self::ChunkResponse(m)),
            None => Err("unknown message: <nil>".into()),
        }
    }
}

/// `ErrExceedsMaxSnapshotChunks` (prefix of the error message).
pub const ERR_EXCEEDS_MAX_SNAPSHOT_CHUNKS: &str =
    "amount of chunks in the snapshot exceeds the maximum allowed number of chunks";

/// `validateMsg`. A proto3 `bytes` field cannot be told apart from an
/// absent one after decoding, so "chunk cannot be nil" covers empty chunks
/// (Go never encodes an empty, non-nil chunk either).
pub fn validate_msg(msg: &Msg, max_snapshot_chunks: u32) -> Result<(), String> {
    match msg {
        Msg::ChunkRequest(m) => {
            if m.height == 0 {
                return Err("height cannot be 0".into());
            }
        }
        Msg::ChunkResponse(m) => {
            if m.height == 0 {
                return Err("height cannot be 0".into());
            }
            if m.missing && !m.chunk.is_empty() {
                return Err("missing chunk cannot have contents".into());
            }
            if !m.missing && m.chunk.is_empty() {
                return Err("chunk cannot be nil".into());
            }
        }
        Msg::SnapshotsRequest => {}
        Msg::SnapshotsResponse(m) => {
            if m.height == 0 {
                return Err("height cannot be 0".into());
            }
            if m.hash.is_empty() {
                return Err("snapshot has no hash".into());
            }
            if m.chunks == 0 {
                return Err("snapshot has no chunks".into());
            }
            if m.chunks > max_snapshot_chunks {
                return Err(format!(
                    "{ERR_EXCEEDS_MAX_SNAPSHOT_CHUNKS}: snapshot response chunk count: {}, maximum chunks: {max_snapshot_chunks}",
                    m.chunks
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire encodings from Go's `messages_test.go` (`TestStateSyncVectors`).
    #[test]
    fn go_vectors() {
        let cases: [(Msg, &str); 4] = [
            (Msg::SnapshotsRequest, "0a00"),
            (
                Msg::SnapshotsResponse(pb::SnapshotsResponse {
                    height: 1,
                    format: 2,
                    chunks: 3,
                    hash: b"chuck hash".to_vec(),
                    metadata: b"snapshot metadata".to_vec(),
                }),
                "1225080110021803220a636875636b20686173682a11736e617073686f74206d65746164617461",
            ),
            (
                Msg::ChunkRequest(pb::ChunkRequest {
                    height: 1,
                    format: 2,
                    index: 3,
                }),
                "1a06080110021803",
            ),
            (
                Msg::ChunkResponse(pb::ChunkResponse {
                    height: 1,
                    format: 2,
                    index: 3,
                    chunk: b"it's a chunk".to_vec(),
                    missing: false,
                }),
                "2214080110021803220c697427732061206368756e6b",
            ),
        ];
        for (m, want) in &cases {
            assert_eq!(hex(&m.encode()), *want);
            assert_eq!(Msg::decode(&m.encode()).ok().as_ref(), Some(m));
        }
    }

    fn hex(b: &[u8]) -> String {
        ::hex::encode(b)
    }

    #[test]
    fn validate() {
        let ok = |m: Msg| validate_msg(&m, 10);
        assert_eq!(
            ok(Msg::ChunkResponse(pb::ChunkResponse {
                height: 1,
                missing: true,
                chunk: vec![1],
                ..Default::default()
            })),
            Err("missing chunk cannot have contents".into())
        );
        assert_eq!(
            ok(Msg::ChunkResponse(pb::ChunkResponse {
                height: 1,
                ..Default::default()
            })),
            Err("chunk cannot be nil".into())
        );
        assert!(
            ok(Msg::SnapshotsResponse(pb::SnapshotsResponse {
                height: 1,
                chunks: 11,
                hash: vec![1],
                ..Default::default()
            }))
            .is_err_and(|e| e.starts_with(ERR_EXCEEDS_MAX_SNAPSHOT_CHUNKS))
        );
    }
}
