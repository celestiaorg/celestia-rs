//! The server's gRPC codec (Go `internal/grpc/codec*.go`, server side).
//!
//! Before the generated decoder allocates for an `UploadShardRequest`, the
//! codec counts its rows, then the proof segments of each row, straight off
//! the wire, and rejects requests over the limits (Go's
//! `RowCountUploadShard` / `ProofCountUploadShard` stubs). A
//! `DownloadShardRequest` over 1024 bytes is rejected before it is decoded.
//! grpc-go reports a codec failure as `Internal` `grpc: error unmarshalling
//! request: <err>`; so does this codec.
//!
//! Messages are decoded from tonic's receive buffer in place: the only copy
//! of an upload's row data is the one into the decoded message.

use std::marker::PhantomData;
use std::sync::Arc;

use bytes::Buf;
use celestia_proto::celestia::fibre::v1::{
    DownloadShardRequest, DownloadShardResponse, UploadShardRequest, UploadShardResponse,
};
use tonic::Status;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};

use crate::metrics::{RejectReason, ServerMetrics};
use crate::proto::{
    HealthCheckRequest, HealthCheckResponse, HealthListRequest, HealthListResponse,
};

/// Go `maxDownloadShardRequestSize`: a valid request is 35 bytes.
pub const MAX_DOWNLOAD_SHARD_REQUEST_SIZE: usize = 1024;

/// The pre-allocation limits (Go `NewServerCodec(maxShardRows,
/// maxProofSegments)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecLimits {
    /// Maximum rows in one uploaded shard (`MaxRowsPerValidator`).
    pub max_shard_rows: usize,
    /// Maximum proof segments per row (`MerkleProofDepth`).
    pub max_proof_segments: usize,
}

/// A codec rejection, as text (`fibre-proto codec: ...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError {
    /// The error text.
    pub message: String,
    /// The request exceeded a limit (rather than being malformed).
    pub limit: bool,
}

impl CodecError {
    fn malformed(err: impl std::fmt::Display) -> Self {
        Self {
            message: format!("fibre-proto codec: {err}"),
            limit: false,
        }
    }

    fn limit(message: String) -> Self {
        Self {
            message,
            limit: true,
        }
    }
}

const ERR_EOF: &str = "unexpected EOF";
const ERR_OVERFLOW: &str = "proto: integer overflow";
const ERR_NEG_LEN: &str = "proto: negative length found during unmarshaling";
const ERR_END_GROUP: &str = "proto: unexpected end of group";

/// A gogoproto-style reader over one message's bytes.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn varint(&mut self) -> Result<u64, &'static str> {
        let mut v: u64 = 0;
        let mut shift = 0u32;
        loop {
            if shift >= 64 {
                return Err(ERR_OVERFLOW);
            }
            let b = *self.data.get(self.pos).ok_or(ERR_EOF)?;
            self.pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    /// A length-delimited payload (after the tag).
    fn bytes(&mut self) -> Result<&'a [u8], &'static str> {
        let len = self.varint()?;
        #[allow(clippy::cast_possible_wrap)]
        if (len as i64) < 0 {
            return Err(ERR_NEG_LEN);
        }
        let end = usize::try_from(len)
            .ok()
            .and_then(|l| self.pos.checked_add(l))
            .ok_or(ERR_NEG_LEN)?;
        if end > self.data.len() {
            return Err(ERR_EOF);
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// gogoproto `skip<File>` from the field's tag.
    fn skip(&mut self, wire_type: u64) -> Result<(), String> {
        let mut depth = 0i32;
        let mut wt = wire_type;
        loop {
            match wt {
                0 => {
                    self.varint()?;
                }
                1 => self.fixed(8)?,
                2 => {
                    self.bytes()?;
                }
                3 => depth += 1,
                4 => {
                    if depth == 0 {
                        return Err(ERR_END_GROUP.into());
                    }
                    depth -= 1;
                }
                5 => self.fixed(4)?,
                other => return Err(format!("proto: illegal wireType {other}")),
            }
            if depth == 0 {
                return Ok(());
            }
            let tag = self.varint()?;
            wt = tag & 7;
        }
    }

    fn fixed(&mut self, n: usize) -> Result<(), &'static str> {
        let end = self.pos.checked_add(n).ok_or(ERR_EOF)?;
        if end > self.data.len() {
            return Err(ERR_EOF);
        }
        self.pos = end;
        Ok(())
    }
}

/// Walks the fields of one message (gogoproto generated `Unmarshal`),
/// calling `field(num, wire_type, reader)` for the fields it knows (it
/// returns `false` for others, which are skipped).
fn walk<'a>(
    data: &'a [u8],
    message: &str,
    mut field: impl FnMut(i32, u64, &mut Reader<'a>) -> Result<bool, String>,
) -> Result<(), String> {
    let mut r = Reader::new(data);
    while !r.done() {
        let pre = r.pos;
        let tag = r.varint()?;
        #[allow(clippy::cast_possible_truncation)]
        let num = (tag >> 3) as i32;
        let wt = tag & 7;
        if wt == 4 {
            return Err(format!(
                "proto: {message}: wiretype end group for non-group"
            ));
        }
        if num <= 0 {
            return Err(format!(
                "proto: {message}: illegal tag {num} (wire type {tag})"
            ));
        }
        if !field(num, wt, &mut r)? {
            r.pos = pre;
            r.varint()?;
            r.skip(wt)?;
        }
    }
    Ok(())
}

/// A repeated `bytes`/message field: checks the wire type, returns the
/// payload.
fn len_field<'a>(r: &mut Reader<'a>, wt: u64, name: &str) -> Result<&'a [u8], String> {
    if wt != 2 {
        return Err(format!("proto: wrong wireType = {wt} for field {name}"));
    }
    Ok(r.bytes()?)
}

/// Go `pooledCodec.validateUploadShard`: counts rows (across repeated
/// `shard` fields, which protobuf merges), checks the row limit, then
/// counts each row's proof segments.
pub fn validate_upload_shard(data: &[u8], limits: &CodecLimits) -> Result<(), CodecError> {
    // RowCountUploadShard.
    let mut shards: Vec<&[u8]> = Vec::new();
    let mut rows = 0usize;
    walk(data, "RowCountUploadShard", |num, wt, r| {
        if num != 2 {
            return Ok(false);
        }
        let shard = len_field(r, wt, "Shard")?;
        walk(shard, "RowCountBlobShard", |num, wt, r| {
            if num != 1 {
                return Ok(false);
            }
            len_field(r, wt, "Rows")?;
            rows += 1;
            Ok(true)
        })?;
        shards.push(shard);
        Ok(true)
    })
    .map_err(CodecError::malformed)?;
    if !shards.is_empty() && rows > limits.max_shard_rows {
        return Err(CodecError::limit(format!(
            "fibre-proto codec: shard exceeds {} rows",
            limits.max_shard_rows
        )));
    }

    // ProofCountUploadShard. The bytes passed the walk above, so only the
    // inner row walk can fail here.
    for shard in shards {
        let mut over = false;
        walk(shard, "ProofCountBlobShard", |num, wt, r| {
            if num != 1 {
                return Ok(false);
            }
            let row = len_field(r, wt, "Rows")?;
            let mut proofs = 0usize;
            walk(row, "ProofCountBlobRow", |num, wt, r| {
                if num != 3 {
                    return Ok(false);
                }
                len_field(r, wt, "Proof")?;
                proofs += 1;
                Ok(true)
            })?;
            if proofs > limits.max_proof_segments {
                over = true;
            }
            Ok(true)
        })
        .map_err(CodecError::malformed)?;
        if over {
            return Err(CodecError::limit(format!(
                "fibre-proto codec: row exceeds {} proof segments",
                limits.max_proof_segments
            )));
        }
    }
    Ok(())
}

/// A request message with Go's pre-decode checks.
pub(crate) trait Inbound: prost::Message + Default + Send + 'static {
    /// Checks on the wire length alone, before the bytes are touched.
    fn check_len(_len: usize) -> Result<(), CodecError> {
        Ok(())
    }

    /// Checks on the wire bytes before decoding.
    fn check_bytes(_data: &[u8], _limits: &CodecLimits) -> Result<(), CodecError> {
        Ok(())
    }
}

impl Inbound for UploadShardRequest {
    fn check_bytes(data: &[u8], limits: &CodecLimits) -> Result<(), CodecError> {
        validate_upload_shard(data, limits)
    }
}

impl Inbound for DownloadShardRequest {
    fn check_len(len: usize) -> Result<(), CodecError> {
        if len > MAX_DOWNLOAD_SHARD_REQUEST_SIZE {
            return Err(CodecError::malformed(format!(
                "download request exceeds {MAX_DOWNLOAD_SHARD_REQUEST_SIZE} bytes"
            )));
        }
        Ok(())
    }
}

impl Inbound for HealthCheckRequest {}
impl Inbound for HealthListRequest {}

/// A response message.
pub(crate) trait Outbound: prost::Message + Send + 'static {}
impl Outbound for UploadShardResponse {}
impl Outbound for DownloadShardResponse {}
impl Outbound for HealthCheckResponse {}
impl Outbound for HealthListResponse {}

/// The tonic codec for one method.
pub(crate) struct FibreCodec<E, D> {
    limits: CodecLimits,
    max_send: usize,
    metrics: Option<Arc<ServerMetrics>>,
    _types: PhantomData<fn(E) -> D>,
}

impl<E, D> FibreCodec<E, D> {
    pub(crate) fn new(
        limits: CodecLimits,
        max_send: usize,
        metrics: Option<Arc<ServerMetrics>>,
    ) -> Self {
        Self {
            limits,
            max_send,
            metrics,
            _types: PhantomData,
        }
    }
}

pub(crate) struct FibreEncoder<E> {
    max_send: usize,
    _types: PhantomData<fn(E)>,
}

pub(crate) struct FibreDecoder<D> {
    limits: CodecLimits,
    metrics: Option<Arc<ServerMetrics>>,
    _types: PhantomData<fn() -> D>,
}

impl<E: Outbound> Encoder for FibreEncoder<E> {
    type Item = E;
    type Error = Status;

    fn encode(&mut self, item: E, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        let len = item.encoded_len();
        if len > self.max_send {
            // Rewritten to grpc-go's ResourceExhausted by
            // `grpc_transport::response`.
            return Err(Status::resource_exhausted(format!(
                "grpc: trying to send message larger than max ({len} vs. {})",
                self.max_send
            )));
        }
        dst.reserve(len);
        item.encode(dst)
            .map_err(|e| Status::internal(format!("grpc: error while marshaling: {e}")))
    }
}

impl<D: Inbound> FibreDecoder<D> {
    fn fail(&self, err: &CodecError) -> Status {
        if err.limit
            && let Some(m) = &self.metrics
        {
            m.reject(RejectReason::CodecLimit);
        }
        tracing::debug!(error = %err.message, "rejected request in codec");
        Status::internal(format!(
            "grpc: error unmarshalling request: {}",
            err.message
        ))
    }
}

impl<D: Inbound> Decoder for FibreDecoder<D> {
    type Item = D;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<D>, Status> {
        let len = src.remaining();
        D::check_len(len).map_err(|e| self.fail(&e))?;
        let chunk = src.chunk();
        let msg = if chunk.len() == len {
            D::check_bytes(chunk, &self.limits).map_err(|e| self.fail(&e))?;
            D::decode(src)
        } else {
            // tonic's buffer is contiguous; this is only a fallback.
            let bytes = src.copy_to_bytes(len);
            D::check_bytes(&bytes, &self.limits).map_err(|e| self.fail(&e))?;
            D::decode(bytes)
        };
        msg.map(Some)
            .map_err(|e| self.fail(&CodecError::malformed(e)))
    }
}

impl<E: Outbound, D: Inbound> Codec for FibreCodec<E, D> {
    type Encode = E;
    type Decode = D;
    type Encoder = FibreEncoder<E>;
    type Decoder = FibreDecoder<D>;

    fn encoder(&mut self) -> FibreEncoder<E> {
        FibreEncoder {
            max_send: self.max_send,
            _types: PhantomData,
        }
    }

    fn decoder(&mut self) -> FibreDecoder<D> {
        FibreDecoder {
            limits: self.limits,
            metrics: self.metrics.clone(),
            _types: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::params::DEFAULT_PROTOCOL_PARAMS;
    use celestia_proto::celestia::fibre::v1::{BlobRow, BlobShard};
    use prost::Message;

    const LIMITS: CodecLimits = CodecLimits {
        max_shard_rows: 3,
        max_proof_segments: 2,
    };

    fn make(rows: usize, proofs: usize) -> UploadShardRequest {
        UploadShardRequest {
            promise: None,
            shard: Some(BlobShard {
                rows: (0..rows)
                    .map(|i| BlobRow {
                        index: u32::try_from(i).unwrap(),
                        data: vec![0xff],
                        proof: vec![vec![0u8; 32]; proofs],
                    })
                    .collect(),
                rlcs: vec![0; 16],
            }),
        }
    }

    fn check(req: &UploadShardRequest) -> Result<(), CodecError> {
        validate_upload_shard(&req.encode_to_vec(), &LIMITS)
    }

    fn append_tag(buf: &mut Vec<u8>, num: u64, wt: u64) {
        prost::encoding::encode_varint((num << 3) | wt, buf);
    }

    // Go TestValidateUploadShard.
    #[test]
    fn validate_upload_shard_cases() {
        check(&UploadShardRequest::default()).unwrap();
        check(&make(3, 2)).unwrap();
        assert!(check(&make(4, 1)).unwrap_err().message.contains("rows"));
        assert_eq!(
            check(&make(4, 1)).unwrap_err().message,
            "fibre-proto codec: shard exceeds 3 rows"
        );
        assert_eq!(
            check(&make(1, 3)).unwrap_err().message,
            "fibre-proto codec: row exceeds 2 proof segments"
        );

        // Only the last row over the proof limit.
        let mut req = make(3, 1);
        req.shard.as_mut().unwrap().rows[2].proof = vec![vec![]; 3];
        assert!(check(&req).unwrap_err().message.contains("proof segments"));

        // Empty rows are counted.
        let req = UploadShardRequest {
            promise: None,
            shard: Some(BlobShard {
                rows: vec![BlobRow::default(); 4],
                rlcs: vec![],
            }),
        };
        assert!(check(&req).unwrap_err().message.contains("rows"));

        // Empty proof segments are counted.
        let req = UploadShardRequest {
            promise: None,
            shard: Some(BlobShard {
                rows: vec![BlobRow {
                    index: 0,
                    data: vec![],
                    proof: vec![vec![]; 3],
                }],
                rlcs: vec![],
            }),
        };
        assert!(check(&req).unwrap_err().message.contains("proof segments"));

        // Rows split across duplicate shard fields accumulate.
        let one = make(2, 0).encode_to_vec();
        let doubled = [one.clone(), one].concat();
        let real = UploadShardRequest::decode(doubled.as_slice()).unwrap();
        assert_eq!(real.shard.unwrap().rows.len(), 4);
        assert!(
            validate_upload_shard(&doubled, &LIMITS)
                .unwrap_err()
                .message
                .contains("rows")
        );

        // Unknown trailing field skipped.
        let mut buf = make(1, 1).encode_to_vec();
        append_tag(&mut buf, 7, 0);
        prost::encoding::encode_varint(42, &mut buf);
        validate_upload_shard(&buf, &LIMITS).unwrap();

        // Malformed bytes rejected.
        let full = make(2, 2).encode_to_vec();
        let mut tag_only = Vec::new();
        append_tag(&mut tag_only, 7, 0);
        for bad in [&full[..full.len() - 1], &[0x80][..], tag_only.as_slice()] {
            let err = validate_upload_shard(bad, &LIMITS).unwrap_err();
            assert!(!err.limit);
            assert!(err.message.starts_with("fibre-proto codec: "), "{err:?}");
        }
        assert_eq!(
            validate_upload_shard(&[0x80], &LIMITS).unwrap_err().message,
            "fibre-proto codec: unexpected EOF"
        );
        // Wrong wire type of the shard field.
        let mut wrong = Vec::new();
        append_tag(&mut wrong, 2, 0);
        prost::encoding::encode_varint(1, &mut wrong);
        assert_eq!(
            validate_upload_shard(&wrong, &LIMITS).unwrap_err().message,
            "fibre-proto codec: proto: wrong wireType = 0 for field Shard"
        );
    }

    // Go TestServerCodecLimits: the server's protocol limits.
    #[test]
    fn server_codec_limits() {
        let p = DEFAULT_PROTOCOL_PARAMS;
        let limits = CodecLimits {
            max_shard_rows: p.max_rows_per_validator(),
            max_proof_segments: p.merkle_proof_depth(),
        };
        let req = |rows: usize, proofs: usize| UploadShardRequest {
            promise: None,
            shard: Some(BlobShard {
                rows: (0..rows)
                    .map(|i| BlobRow {
                        index: u32::try_from(i).unwrap(),
                        data: vec![],
                        proof: vec![vec![0; 32]; proofs],
                    })
                    .collect(),
                rlcs: vec![],
            }),
        };
        validate_upload_shard(
            &req(limits.max_shard_rows, limits.max_proof_segments).encode_to_vec(),
            &limits,
        )
        .unwrap();
        assert!(
            validate_upload_shard(&req(limits.max_shard_rows + 1, 0).encode_to_vec(), &limits)
                .unwrap_err()
                .message
                .contains("rows")
        );
        assert!(
            validate_upload_shard(
                &req(1, limits.max_proof_segments + 1).encode_to_vec(),
                &limits
            )
            .unwrap_err()
            .message
            .contains("proof segments")
        );
    }

    #[test]
    fn download_request_limit() {
        DownloadShardRequest::check_len(35).unwrap();
        DownloadShardRequest::check_len(MAX_DOWNLOAD_SHARD_REQUEST_SIZE).unwrap();
        assert_eq!(
            DownloadShardRequest::check_len(MAX_DOWNLOAD_SHARD_REQUEST_SIZE + 1)
                .unwrap_err()
                .message,
            "fibre-proto codec: download request exceeds 1024 bytes"
        );
    }
}
