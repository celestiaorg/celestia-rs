//! Frame boundary and malformed-input regressions.

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

#[tokio::test]
async fn overflowing_tenth_byte_is_rejected() {
    // Only one payload bit fits in byte ten of a u64. In particular,
    // 0x02 must not truncate to zero and become an empty frame.
    for last in 2..=255 {
        let mut prefix = [0x80; 10];
        prefix[9] = last;
        let error = read_frame(&mut prefix.as_slice()).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "byte {last}");
    }
}

#[tokio::test]
async fn oversized_frames_are_rejected_before_reading_payload() {
    for len in [MAX_FRAME_LEN + 1, u64::MAX] {
        let mut prefix = Vec::new();
        prost::encoding::encode_varint(len, &mut prefix);
        let error = read_frame(&mut prefix.as_slice()).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[tokio::test]
async fn truncated_payload_is_error() {
    let mut wire = encode_frame(b"payload");
    wire.pop();
    let error = read_frame(&mut wire.as_slice()).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn write_frame_roundtrip() -> io::Result<()> {
    let mut wire = Vec::new();
    write_frame(&mut wire, b"payload").await?;
    assert_eq!(wire, encode_frame(b"payload"));
    assert_eq!(
        read_frame(&mut wire.as_slice()).await?,
        Some(b"payload".to_vec())
    );
    Ok(())
}

#[test]
fn prost_compatible_prefix() {
    let payload = vec![9u8; 300];
    let mut expected = Vec::new();
    prost::encoding::encode_varint(300, &mut expected);
    expected.extend_from_slice(&payload);
    assert_eq!(encode_frame(&payload), expected);
}
