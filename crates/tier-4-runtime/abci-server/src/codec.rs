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
        if shift >= 64 {
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_frames() -> io::Result<()> {
        let payloads = [
            vec![],
            vec![1u8; 1],
            vec![2u8; 127],
            vec![3u8; 128],
            vec![4u8; 70_000],
        ];
        let mut wire = Vec::new();
        for p in &payloads {
            wire.extend(encode_frame(p));
        }
        let mut r = wire.as_slice();
        for p in &payloads {
            assert_eq!(read_frame(&mut r).await?.as_ref(), Some(p));
        }
        assert_eq!(read_frame(&mut r).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn truncated_prefix_is_error() {
        let mut r: &[u8] = &[0x80];
        assert!(read_frame(&mut r).await.is_err());
    }

    #[test]
    fn prost_compatible_prefix() {
        let payload = vec![9u8; 300];
        let mut expected = Vec::new();
        prost::encoding::encode_varint(300, &mut expected);
        expected.extend_from_slice(&payload);
        assert_eq!(encode_frame(&payload), expected);
    }
}
