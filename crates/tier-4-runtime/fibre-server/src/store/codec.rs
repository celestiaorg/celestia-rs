//! On-disk shard file format (Go `store_codec.go`), byte-identical to Go.
//!
//! All integers big-endian:
//!
//! ```text
//! u32 version (= 1)
//! u32 rlcs_len; [u8] rlcs
//! u32 num_rows
//!   per row: u32 index; u32 data_len; [u8] data;
//!            u32 num_proof_segments; per segment: u32 len; [u8] segment
//! ```
//!
//! Row payloads are written from their own buffers (no whole-shard
//! serialization buffer) and read straight into their final `Vec`s.

use std::io::{self, Read, Write};

use celestia_proto::celestia::fibre::v1::{BlobRow, BlobShard};

/// Shard file format version.
pub(crate) const SHARD_CODEC_VERSION: u32 = 1;
/// Cap on any single byte-length prefix (rejects corrupt files before
/// allocating).
pub(crate) const SHARD_LENGTH_LIMIT: u32 = 1 << 30;
/// Cap on the row count (4x the protocol's total rows).
pub(crate) const MAX_SHARD_ROWS: u32 = 1 << 16;
/// Cap on proof segments per row (covers 2^64-leaf trees).
pub(crate) const MAX_ROW_PROOF_SEGMENTS: u32 = 64;

/// Exact number of bytes the shard file of `shard` takes (Go
/// `shardBinarySize`); also the size recorded in the shard marker and
/// reserved in occupancy.
#[must_use]
pub fn shard_binary_size(shard: &BlobShard) -> i64 {
    // version + rlcs-len prefix + rlcs + row-count prefix
    let mut size = 12 + len_i64(shard.rlcs.len());
    for row in &shard.rows {
        // index + data-len prefix + data + proof-count prefix
        size += 12 + len_i64(row.data.len());
        for seg in &row.proof {
            size += 4 + len_i64(seg.len());
        }
    }
    size
}

fn len_i64(len: usize) -> i64 {
    i64::try_from(len).unwrap_or(i64::MAX)
}

fn len_u32(len: usize, what: &str) -> io::Result<u32> {
    u32::try_from(len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} length {len} does not fit the shard format"),
        )
    })
}

/// Serializes `shard` to `w` (Go `writeShardBinary`).
pub(crate) fn write_shard(w: &mut impl Write, shard: &BlobShard) -> io::Result<()> {
    let mut head = [0u8; 12];
    head[..4].copy_from_slice(&SHARD_CODEC_VERSION.to_be_bytes());
    head[4..8].copy_from_slice(&len_u32(shard.rlcs.len(), "rlcs")?.to_be_bytes());
    w.write_all(&head[..8])?;
    w.write_all(&shard.rlcs)?;
    w.write_all(&len_u32(shard.rows.len(), "row count")?.to_be_bytes())?;
    for row in &shard.rows {
        head[..4].copy_from_slice(&row.index.to_be_bytes());
        head[4..8].copy_from_slice(&len_u32(row.data.len(), "row data")?.to_be_bytes());
        w.write_all(&head[..8])?;
        w.write_all(&row.data)?;
        w.write_all(&len_u32(row.proof.len(), "proof count")?.to_be_bytes())?;
        for seg in &row.proof {
            w.write_all(&len_u32(seg.len(), "proof segment")?.to_be_bytes())?;
            w.write_all(seg)?;
        }
    }
    Ok(())
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn read_bytes(r: &mut impl Read, n: u32) -> Result<Vec<u8>, String> {
    if n > SHARD_LENGTH_LIMIT {
        return Err(format!(
            "length {n} exceeds shard limit {SHARD_LENGTH_LIMIT}"
        ));
    }
    let mut out = vec![0u8; n as usize];
    r.read_exact(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Decodes a shard file (Go `readShardBinary`). Errors are plain strings,
/// as in Go (they are not integrity errors).
pub(crate) fn read_shard(r: &mut impl Read) -> Result<BlobShard, String> {
    let version = read_u32(r).map_err(|e| format!("reading version: {e}"))?;
    if version != SHARD_CODEC_VERSION {
        return Err(format!(
            "unsupported shard codec version {version} (want {SHARD_CODEC_VERSION})"
        ));
    }
    let rlcs_len = read_u32(r).map_err(|e| format!("reading rlcs len: {e}"))?;
    let rlcs = read_bytes(r, rlcs_len).map_err(|e| format!("reading rlcs: {e}"))?;
    let num_rows = read_u32(r).map_err(|e| format!("reading num rows: {e}"))?;
    if num_rows > MAX_SHARD_ROWS {
        return Err(format!(
            "num rows {num_rows} exceeds limit {MAX_SHARD_ROWS}"
        ));
    }
    let mut rows = Vec::with_capacity(num_rows as usize);
    for i in 0..num_rows {
        let index = read_u32(r).map_err(|e| format!("reading row {i} index: {e}"))?;
        let data_len = read_u32(r).map_err(|e| format!("reading row {i} data len: {e}"))?;
        let data = read_bytes(r, data_len).map_err(|e| format!("reading row {i} data: {e}"))?;
        let num_proof = read_u32(r).map_err(|e| format!("reading row {i} num proof: {e}"))?;
        if num_proof > MAX_ROW_PROOF_SEGMENTS {
            return Err(format!(
                "row {i} num proof {num_proof} exceeds limit {MAX_ROW_PROOF_SEGMENTS}"
            ));
        }
        let mut proof = Vec::with_capacity(num_proof as usize);
        for j in 0..num_proof {
            let seg_len = read_u32(r).map_err(|e| format!("reading row {i} proof {j} len: {e}"))?;
            proof.push(
                read_bytes(r, seg_len).map_err(|e| format!("reading row {i} proof {j}: {e}"))?,
            );
        }
        rows.push(BlobRow { index, data, proof });
    }
    Ok(BlobShard { rows, rlcs })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn sample() -> BlobShard {
        BlobShard {
            rlcs: vec![0xcd; 64],
            rows: vec![
                BlobRow {
                    index: 0,
                    data: vec![0x01; 1024],
                    proof: vec![b"seg-a".to_vec(), b"seg-b".to_vec()],
                },
                BlobRow {
                    index: 7,
                    data: vec![0x02; 2048],
                    proof: vec![],
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let shard = sample();
        let mut buf = Vec::new();
        write_shard(&mut buf, &shard).unwrap();
        assert_eq!(read_shard(&mut buf.as_slice()).unwrap(), shard);
    }

    #[test]
    fn binary_size_matches_encoding() {
        let cases = [
            BlobShard::default(),
            BlobShard {
                rlcs: vec![0xcd; 64],
                rows: vec![],
            },
            sample(),
        ];
        for shard in cases {
            let mut buf = Vec::new();
            write_shard(&mut buf, &shard).unwrap();
            assert_eq!(shard_binary_size(&shard), i64::try_from(buf.len()).unwrap());
        }
    }

    /// The bytes Go's `writeShardBinary` produces for a small shard.
    #[test]
    fn matches_go_layout() {
        let shard = BlobShard {
            rlcs: vec![0xaa, 0xbb],
            rows: vec![BlobRow {
                index: 3,
                data: b"data".to_vec(),
                proof: vec![vec![0x01]],
            }],
        };
        let mut buf = Vec::new();
        write_shard(&mut buf, &shard).unwrap();
        let want: Vec<u8> = [
            &[0, 0, 0, 1][..],
            &[0, 0, 0, 2],
            &[0xaa, 0xbb],
            &[0, 0, 0, 1],
            &[0, 0, 0, 3],
            &[0, 0, 0, 4],
            b"data",
            &[0, 0, 0, 1],
            &[0, 0, 0, 1],
            &[0x01],
        ]
        .concat();
        assert_eq!(buf, want);
    }

    #[test]
    fn rejects_bombs_and_truncation() {
        let v = SHARD_CODEC_VERSION.to_be_bytes();
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (
                [
                    &v[..],
                    &0u32.to_be_bytes(),
                    &(MAX_SHARD_ROWS + 1).to_be_bytes(),
                ]
                .concat(),
                "num rows",
            ),
            (
                [
                    &v[..],
                    &0u32.to_be_bytes(),
                    &1u32.to_be_bytes(),
                    &0u32.to_be_bytes(),
                    &0u32.to_be_bytes(),
                    &(MAX_ROW_PROOF_SEGMENTS + 1).to_be_bytes(),
                ]
                .concat(),
                "num proof",
            ),
            (
                [&v[..], &(SHARD_LENGTH_LIMIT + 1).to_be_bytes()].concat(),
                "exceeds shard limit",
            ),
            (
                99u32.to_be_bytes().to_vec(),
                "unsupported shard codec version",
            ),
            (vec![0, 0], "reading version"),
            (
                [&v[..], &4u32.to_be_bytes(), &[1, 2]].concat(),
                "reading rlcs",
            ),
        ];
        for (bytes, want) in cases {
            let err = read_shard(&mut bytes.as_slice()).unwrap_err();
            assert!(err.contains(want), "{err:?} should contain {want:?}");
        }
    }
}
