//! `types.Encode` / `types.Decode` (`types/part_set.go`): Reed-Solomon
//! parity for a block's part set, as klauspost/reedsolomon v1.14.2 computes
//! it with `reedsolomon.New(total, total)`: the classic Vandermonde code
//! over GF(2^8) up to 256 shards (`reed-solomon-erasure`, byte-identical)
//! and leopard over GF(2^16) above (`reed-solomon-simd`, byte-identical).
//! Both are checked against Go vectors (`conformance/consensusvectors/propagation`).

use ctypes::merkle_proof::proofs_from_byte_slices;
use ctypes::{Part, PartSet, PartSetHeader};

/// Shard count up to which klauspost uses the GF(2^8) code.
const MAX_GF8_SHARDS: usize = 256;

fn gf8(total: usize) -> Result<reed_solomon_erasure::galois_8::ReedSolomon, String> {
    reed_solomon_erasure::galois_8::ReedSolomon::new(total, total).map_err(|e| format!("{e:?}"))
}

/// Parity shards of `data` (all of `part_size` bytes).
fn encode_shards(data: &[Vec<u8>], part_size: usize) -> Result<Vec<Vec<u8>>, String> {
    let total = data.len();
    if 2 * total <= MAX_GF8_SHARDS {
        let mut shards: Vec<Vec<u8>> = data.to_vec();
        shards.extend((0..total).map(|_| vec![0u8; part_size]));
        gf8(total)?
            .encode(&mut shards)
            .map_err(|e| format!("{e:?}"))?;
        Ok(shards.split_off(total))
    } else {
        reed_solomon_simd::encode(total, total, data).map_err(|e| e.to_string())
    }
}

/// A complete part set from chunks with known proofs (`newPartSetFromChunks`).
fn part_set_from_chunks(
    chunks: &[Vec<u8>],
    root: Vec<u8>,
    proofs: Vec<ctypes::Proof>,
    part_size: u32,
) -> Result<PartSet, String> {
    let total = u32::try_from(chunks.len()).map_err(|e| e.to_string())?;
    let mut ps = PartSet::from_header(&PartSetHeader { total, hash: root }, part_size);
    for (i, (c, p)) in chunks.iter().zip(proofs).enumerate() {
        ps.add_part_without_proof(&Part {
            index: u32::try_from(i).map_err(|e| e.to_string())?,
            bytes: c.clone(),
            proof: p,
        })
        .map_err(|e| e.to_string())?;
    }
    Ok(ps)
}

/// `types.Encode`: the parity part set of a complete part set and the
/// length of its last original part.
pub fn encode(ops: &PartSet, part_size: u32) -> Result<(PartSet, usize), String> {
    let total = ops.total() as usize;
    if total == 0 {
        return Err("cannot encode an empty part set".into());
    }
    let ps = part_size as usize;
    let mut chunks = Vec::with_capacity(total);
    for i in 0..total {
        let b = ops
            .get_part_bytes(i)
            .ok_or_else(|| format!("missing part {i}"))?;
        chunks.push(b.to_vec());
    }
    let last_len = chunks[total - 1].len();
    if last_len < ps {
        chunks[total - 1].resize(ps, 0);
    }
    let parity = encode_shards(&chunks, ps)?;
    let (root, proofs) = proofs_from_byte_slices(&parity);
    let eps = part_set_from_chunks(&parity, root, proofs, part_size)?;
    Ok((eps, last_len))
}

/// `types.Decode`: reconstructs the missing original and parity parts of
/// `ops` / `eps` (with proofs) from any `total` of the `2 * total` parts.
/// Errors when the result does not match the part-set roots.
pub fn decode(ops: &mut PartSet, eps: &mut PartSet, last_part_len: usize) -> Result<(), String> {
    let total = ops.total() as usize;
    if total == 0 || eps.total() as usize != total {
        return Err(format!(
            "invalid shard counts: {total} original, {} parity",
            eps.total()
        ));
    }
    let ps = ctypes::BLOCK_PART_SIZE_BYTES as usize;
    let mut originals: Vec<Option<Vec<u8>>> = (0..total)
        .map(|i| {
            ops.get_part_bytes(i).map(|b| {
                let mut v = b.to_vec();
                v.resize(ps, 0);
                v
            })
        })
        .collect();
    let mut parity: Vec<Option<Vec<u8>>> = (0..total)
        .map(|i| eps.get_part_bytes(i).map(<[u8]>::to_vec))
        .collect();
    if 2 * total <= MAX_GF8_SHARDS {
        let mut shards: Vec<Option<Vec<u8>>> = originals;
        shards.append(&mut parity);
        gf8(total)?
            .reconstruct(&mut shards)
            .map_err(|e| format!("{e:?}"))?;
        parity = shards.split_off(total);
        originals = shards;
    } else {
        let restored = reed_solomon_simd::decode(
            total,
            total,
            originals
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.as_deref().map(|s| (i, s))),
            parity
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.as_deref().map(|s| (i, s))),
        )
        .map_err(|e| e.to_string())?;
        for (i, s) in restored {
            if let Some(slot) = originals.get_mut(i) {
                *slot = Some(s);
            }
        }
        if parity.iter().any(Option::is_none) {
            let full: Vec<Vec<u8>> = originals
                .iter()
                .map(|s| s.clone().ok_or("too few shards"))
                .collect::<Result<_, _>>()?;
            parity = encode_shards(&full, ps)?.into_iter().map(Some).collect();
        }
    }
    let mut data: Vec<Vec<u8>> = originals
        .into_iter()
        .map(|s| s.ok_or_else(|| "too few shards".to_string()))
        .collect::<Result<_, _>>()?;
    data[total - 1].truncate(last_part_len);
    let (root, proofs) = proofs_from_byte_slices(&data);
    if root != ops.hash() {
        return Err(format!(
            "reconstructed data has different hash!! want: {}, got: {}",
            hex::encode_upper(ops.hash()),
            hex::encode_upper(&root)
        ));
    }
    for (i, (d, p)) in data.into_iter().zip(proofs).enumerate() {
        if !ops.has_part(i) {
            let part = Part {
                index: u32::try_from(i).map_err(|e| e.to_string())?,
                bytes: d,
                proof: p,
            };
            if !ops.add_part(&part).map_err(|e| e.to_string())? {
                return Err(format!("couldn't add original part {i} when decoding"));
            }
        }
    }
    let parity: Vec<Vec<u8>> = parity
        .into_iter()
        .map(|s| s.ok_or_else(|| "too few shards".to_string()))
        .collect::<Result<_, _>>()?;
    let (eroot, eproofs) = proofs_from_byte_slices(&parity);
    if eroot != eps.hash() {
        return Err(format!(
            "reconstructed parity data has different hash!! want: {}, got: {}",
            hex::encode_upper(eps.hash()),
            hex::encode_upper(&eroot)
        ));
    }
    for (i, (d, p)) in parity.into_iter().zip(eproofs).enumerate() {
        if !eps.has_part(i) {
            let part = Part {
                index: u32::try_from(i).map_err(|e| e.to_string())?,
                bytes: d,
                proof: p,
            };
            if !eps.add_part(&part).map_err(|e| e.to_string())? {
                return Err(format!("couldn't add parity part {i} when decoding"));
            }
        }
    }
    Ok(())
}
