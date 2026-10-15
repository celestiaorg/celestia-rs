//! Square construction, erasure coding and the data root from arbitrary
//! block txs: the input is a protobuf `{repeated bytes txs = 1; uint32
//! max_square_size = 2}` (the differential fuzzer's `eds` op encoding).
//! Blob txs are classified with txdecode's go-square unmarshalling (fibre
//! classification lives in the app crate, which needs RocksDB, so it is
//! covered by the differential fuzzer instead).
#![no_main]

use libfuzzer_sys::fuzz_target;
use square::v4::{BlobTx, ClassifiedTx};

fn varint(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..10 {
        let (&x, rest) = b.split_first()?;
        *b = rest;
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

fn parse(mut b: &[u8]) -> Option<(Vec<Vec<u8>>, usize)> {
    let mut txs = Vec::new();
    let mut max = 0usize;
    while !b.is_empty() {
        match varint(&mut b)? {
            0x0a => {
                let n = usize::try_from(varint(&mut b)?).ok()?;
                if n > b.len() {
                    return None;
                }
                txs.push(b[..n].to_vec());
                b = &b[n..];
            }
            0x10 => max = usize::try_from(varint(&mut b)?).ok()?,
            _ => return None,
        }
    }
    Some((txs, max))
}

fn to_blob_tx(u: &txdecode::UnwrappedBlobTx) -> Option<BlobTx> {
    let mut blobs = Vec::new();
    for b in &u.blobs {
        let mut ns = vec![u8::try_from(b.namespace_version).ok()?];
        ns.extend_from_slice(&b.namespace_id);
        let ns = shares::Namespace::from_slice(&ns).ok()?;
        let signer = if b.signer.is_empty() {
            None
        } else {
            Some(b.signer.as_slice().try_into().ok()?)
        };
        blobs.push(
            shares::Blob::new_with_signer(
                ns,
                b.data.clone(),
                u8::try_from(b.share_version).ok()?,
                signer,
            )
            .ok()?,
        );
    }
    Some(BlobTx {
        tx: u.tx.clone(),
        blobs,
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((txs, max)) = parse(data) else {
        return;
    };
    // Keep squares small enough for the fuzzer's throughput.
    let max = if max == 0 { 16 } else { max.min(64) };
    let mut classified = Vec::new();
    for t in &txs {
        classified.push(match txdecode::unmarshal_blob_tx(t) {
            Some(Ok(u)) => match to_blob_tx(&u) {
                Some(b) => ClassifiedTx::Blob(b),
                None => return,
            },
            Some(Err(_)) => return,
            None => ClassifiedTx::Normal(t.clone()),
        });
    }
    let Ok(sq) = square::v4::construct(&classified, max, 64) else {
        return;
    };
    let Ok(eds) = square::extend_shares(sq.as_shares()) else {
        return;
    };
    let _ = square::new_data_availability_header(&eds);
    let _ = shares::parse_txs(sq.as_shares());
    let _ = shares::parse_blobs(sq.as_shares());
});
