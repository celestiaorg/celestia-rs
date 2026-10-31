//! Shared gRPC message limits for the application's and core's listeners.
//!
//! Tonic checks the declared receive length before reserving a message buffer.
//! Its size errors use a different code and wording than grpc-go; translate only
//! those transport errors, preserving all application statuses and metadata.

use axum::body::Body;
use axum::http::{HeaderMap, Response};
use bytes::{Buf, BufMut};
use http_body_util::BodyExt as _;
use tonic::Status;
use tonic::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};

/// Maximum uncompressed protobuf message sizes, excluding the five-byte frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum request message size.
    pub max_receive: usize,
    /// Maximum response message size (each message of a stream separately).
    pub max_send: usize,
}

impl Limits {
    /// celestia-app v10.5.0's initialized app.toml defaults (64 MiB each).
    #[must_use]
    pub const fn sdk() -> Self {
        Self {
            max_receive: 64 << 20,
            max_send: 64 << 20,
        }
    }

    /// grpc-go's standalone core defaults (4 MiB receive, MaxInt32 send).
    #[must_use]
    pub const fn core() -> Self {
        Self {
            max_receive: 4 << 20,
            max_send: i32::MAX as usize,
        }
    }

    /// Cosmos SDK's server fallback when a configured limit is zero.
    #[must_use]
    pub const fn sdk_zero() -> Self {
        Self {
            max_receive: 10 << 20,
            max_send: i32::MAX as usize,
        }
    }

    /// A raw-protobuf server with allocation checked before message decoding.
    #[must_use]
    pub fn grpc(self) -> tonic::server::Grpc<RawCodec> {
        self.server(false)
    }

    /// A streaming server. grpc-go's stream send-limit message omits the unary
    /// handler's `grpc: ` prefix; receive limits are identical for both.
    #[must_use]
    pub fn streaming_grpc(self) -> tonic::server::Grpc<RawCodec> {
        self.server(true)
    }

    fn server(self, streaming: bool) -> tonic::server::Grpc<RawCodec> {
        tonic::server::Grpc::new(RawCodec {
            limits: self,
            streaming,
        })
        .max_decoding_message_size(self.max_receive)
        .max_encoding_message_size(self.max_send)
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::sdk()
    }
}

/// A codec passing protobuf bytes through unchanged.
#[derive(Debug, Clone, Copy)]
pub struct RawCodec {
    limits: Limits,
    streaming: bool,
}

impl RawCodec {
    /// Construct a codec with per-message limits.
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self {
            limits,
            streaming: false,
        }
    }
}

impl Default for RawCodec {
    fn default() -> Self {
        Self::new(Limits::core())
    }
}

/// Raw bytes encoder and decoder.
#[derive(Debug, Clone, Copy)]
pub struct RawCoder {
    max_send: usize,
    streaming: bool,
}

impl Encoder for RawCoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn buffer_settings(&self) -> BufferSettings {
        // Yield each complete frame. Otherwise tonic may batch a prior valid
        // message with the next one and discard both if encoding the latter
        // fails its limit check; grpc-go has already sent the first message.
        BufferSettings::new(1024, 1)
    }

    fn encode(&mut self, item: Vec<u8>, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        // Check before copying into tonic's output buffer, including streaming
        // responses. grpc-go also marshals before checking its send limit.
        if item.len() > self.max_send {
            let prefix = if self.streaming { "" } else { "grpc: " };
            return Err(Status::resource_exhausted(format!(
                "{prefix}trying to send message larger than max ({} vs. {})",
                item.len(),
                self.max_send
            )));
        }
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for RawCoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Vec<u8>>, Status> {
        // tonic has already checked the declared frame length.
        let mut out = vec![0u8; src.remaining()];
        src.copy_to_slice(&mut out);
        Ok(Some(out))
    }
}

impl Codec for RawCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = RawCoder;
    type Decoder = RawCoder;

    fn encoder(&mut self) -> RawCoder {
        RawCoder {
            max_send: self.limits.max_send,
            streaming: self.streaming,
        }
    }
    fn decoder(&mut self) -> RawCoder {
        RawCoder {
            max_send: self.limits.max_send,
            streaming: self.streaming,
        }
    }
}

fn normalize(headers: &mut HeaderMap) {
    let Some(status) = Status::from_header_map(headers) else {
        return;
    };
    let message = if status.code() == tonic::Code::OutOfRange {
        let Some(rest) = status
            .message()
            .strip_prefix("Error, decoded message length too large: found ")
        else {
            return;
        };
        let Some((received, max)) = rest.split_once(" bytes, the limit is: ") else {
            return;
        };
        let Some(max) = max.strip_suffix(" bytes") else {
            return;
        };
        let (Ok(received), Ok(max)) = (received.parse::<usize>(), max.parse::<usize>()) else {
            return;
        };
        format!("grpc: received message larger than max ({received} vs. {max})")
    } else if status.code() == tonic::Code::Internal {
        // tonic 0.12.3 wraps Encoder errors in Internal after formatting their
        // Status. Match only our own numeric send-limit error. Checking in the
        // encoder prevents allocating a second oversized message buffer.
        let Some(rest) = status
            .message()
            .strip_prefix("Error encoding: status: ResourceExhausted, message: \"")
        else {
            return;
        };
        let (prefix, rest) = rest
            .strip_prefix("grpc: ")
            .map_or(("", rest), |rest| ("grpc: ", rest));
        let Some(rest) = rest.strip_prefix("trying to send message larger than max (") else {
            return;
        };
        let Some((sent, rest)) = rest.split_once(" vs. ") else {
            return;
        };
        let Some((max, _)) = rest.split_once(")\", details: ") else {
            return;
        };
        let (Ok(sent), Ok(max)) = (sent.parse::<usize>(), max.parse::<usize>()) else {
            return;
        };
        format!("{prefix}trying to send message larger than max ({sent} vs. {max})")
    } else {
        return;
    };
    let replacement = Status::resource_exhausted(message).into_http();
    for key in ["grpc-status", "grpc-message"] {
        if let Some(value) = replacement.headers().get(key) {
            headers.insert(key, value.clone());
        }
    }
}

/// Preserve body streaming and metadata while translating size-limit statuses.
#[must_use]
pub fn response(mut response: Response<tonic::body::BoxBody>) -> Response<Body> {
    normalize(response.headers_mut());
    response.map(|body| {
        Body::new(body.map_frame(|mut frame| {
            if let Some(trailers) = frame.trailers_mut() {
                normalize(trailers);
            }
            frame
        }))
    })
}
