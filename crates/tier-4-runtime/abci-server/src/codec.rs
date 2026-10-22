//! ABCI socket wire codec: each message is a protobuf `Request`/`Response`
//! prefixed by its length as an unsigned varint (celestia-core `protoio`).

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest frame accepted from a peer (celestia-core's `maxMsgSize` is
/// 100 MiB; blocks are far below this).
pub const MAX_FRAME_LEN: u64 = 128 * 1024 * 1024;

/// Reads one length-delimited frame. Returns `Ok(None)` on a clean EOF at a
/// frame boundary.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len: u64 = 0;
    let mut shift = 0u32;
    loop {
        let mut b = [0u8; 1];
        let n = r.read(&mut b).await?;
        if n == 0 {
            if shift == 0 {
                return Ok(None);
            }
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        // Byte ten has room for only the top bit of a u64. Reject both
        // excess payload bits and a continuation bit before shifting;
        // otherwise Rust discards the high bits and accepts a short frame.
        if shift == 63 && b[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "varint overflow",
            ));
        }
        len |= u64::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds limit"),
        ));
    }
    let len = usize::try_from(len).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Encodes `payload` with its uvarint length prefix.
#[must_use]
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    let mut v = payload.len() as u64;
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
    out.extend_from_slice(payload);
    out
}

/// Writes one length-delimited frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    w.write_all(&encode_frame(payload)).await
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
