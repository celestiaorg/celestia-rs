//! The Reed-Solomon code klauspost/reedsolomon v1.14.2 uses up to 256
//! shards (`reedsolomon.New(n, n)` with default options): a systematic
//! code over GF(2^8) (polynomial `x^8 + x^4 + x^3 + x^2 + 1`) built from
//! the Vandermonde matrix `V[r][c] = r^c`, multiplied by the inverse of
//! its top square so the first `n` rows are the identity.
//!
//! The code is MDS, so the encoding matrix alone fixes every byte: parity
//! is `M[n..] * data`, and any `n` shards give back the same data. The Go
//! vectors (`conformance/consensusvectors/propagation`) check the result.

/// Largest shard count of the GF(2^8) code (data plus parity).
pub(crate) const MAX_SHARDS: usize = 256;

/// `a * b` in GF(2^8), bit by bit (only used to build [`MUL`]).
const fn mul_slow(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            // x^8 = x^4 + x^3 + x^2 + 1.
            a ^= 0x1d;
        }
        b >>= 1;
    }
    r
}

/// Only evaluated at compile time (the initializer of [`MUL`]).
#[expect(clippy::large_stack_arrays, reason = "evaluated at compile time")]
const fn mul_table() -> [[u8; 256]; 256] {
    let mut t = [[0u8; 256]; 256];
    let mut a: u8 = 0;
    loop {
        let mut b: u8 = 0;
        loop {
            t[a as usize][b as usize] = mul_slow(a, b);
            if b == u8::MAX {
                break;
            }
            b += 1;
        }
        if a == u8::MAX {
            break;
        }
        a += 1;
    }
    t
}

/// `MUL[a][b] = a * b`.
static MUL: [[u8; 256]; 256] = mul_table();

fn mul(a: u8, b: u8) -> u8 {
    MUL[usize::from(a)][usize::from(b)]
}

/// `1 / a` (`a^254`); 0 for 0.
fn inv(a: u8) -> u8 {
    let mut r = 1;
    for _ in 0..254 {
        r = mul(r, a);
    }
    r
}

/// `out += c * input`.
fn mul_add(c: u8, input: &[u8], out: &mut [u8]) {
    if c == 0 {
        return;
    }
    let row = &MUL[usize::from(c)];
    let at = |i: u8| row[usize::from(i)];
    // Eight bytes a step: one 64-bit XOR into the output.
    let (out_words, out_tail) = out.as_chunks_mut::<8>();
    let (in_words, in_tail) = input.as_chunks::<8>();
    for (o, i) in out_words.iter_mut().zip(in_words) {
        let product = [
            at(i[0]),
            at(i[1]),
            at(i[2]),
            at(i[3]),
            at(i[4]),
            at(i[5]),
            at(i[6]),
            at(i[7]),
        ];
        *o = (u64::from_ne_bytes(*o) ^ u64::from_ne_bytes(product)).to_ne_bytes();
    }
    for (o, i) in out_tail.iter_mut().zip(in_tail) {
        *o ^= at(*i);
    }
}

type Matrix = Vec<Vec<u8>>;

/// Inverse of a square matrix (Gauss-Jordan).
fn invert(mut m: Matrix) -> Result<Matrix, String> {
    let n = m.len();
    let mut out: Matrix = (0..n)
        .map(|r| (0..n).map(|c| u8::from(r == c)).collect())
        .collect();
    for col in 0..n {
        let pivot = (col..n)
            .find(|&r| m[r][col] != 0)
            .ok_or("singular matrix")?;
        m.swap(col, pivot);
        out.swap(col, pivot);
        let scale = inv(m[col][col]);
        for v in m[col].iter_mut().chain(out[col].iter_mut()) {
            *v = mul(*v, scale);
        }
        let (m_row, out_row) = (m[col].clone(), out[col].clone());
        for r in 0..n {
            let f = m[r][col];
            if r != col && f != 0 {
                mul_add(f, &m_row, &mut m[r]);
                mul_add(f, &out_row, &mut out[r]);
            }
        }
    }
    Ok(out)
}

/// `rows * shards`: one output shard of `len` bytes for each row.
///
/// The loops go input shard by input shard, in blocks, so the input block
/// and the output blocks it updates stay in the cache.
fn apply(rows: &[&[u8]], shards: &[&[u8]], len: usize) -> Vec<Vec<u8>> {
    const BLOCK: usize = 8 * 1024;
    let mut out = vec![vec![0u8; len]; rows.len()];
    for start in (0..len).step_by(BLOCK) {
        let end = len.min(start + BLOCK);
        for (c, shard) in shards.iter().enumerate() {
            for (row, o) in rows.iter().zip(&mut out) {
                mul_add(row[c], &shard[start..end], &mut o[start..end]);
            }
        }
    }
    out
}

/// The common length of `shards`; an error when they differ or are empty.
fn shard_len<'a>(mut shards: impl Iterator<Item = &'a Vec<u8>>) -> Result<usize, String> {
    let len = shards.next().map_or(0, Vec::len);
    if len == 0 {
        return Err("empty shard".into());
    }
    if shards.any(|s| s.len() != len) {
        return Err("shards of different sizes".into());
    }
    Ok(len)
}

/// A `n` data + `n` parity code.
pub(crate) struct Code {
    /// Data (and parity) shard count.
    n: usize,
    /// The `2n x n` encoding matrix (identity on top).
    matrix: Matrix,
}

impl Code {
    /// The code with `n` data shards and `n` parity shards.
    pub(crate) fn new(n: usize) -> Result<Self, String> {
        if n == 0 || 2 * n > MAX_SHARDS {
            return Err(format!("invalid shard count: {n} data, {n} parity"));
        }
        // V[r][c] = r^c (0^0 = 1).
        let vandermonde: Matrix = (0..=u8::MAX)
            .take(2 * n)
            .map(|r| {
                let mut v = 1;
                (0..n)
                    .map(|_| {
                        let cur = v;
                        v = mul(v, r);
                        cur
                    })
                    .collect()
            })
            .collect();
        let top_inverse = invert(vandermonde[..n].to_vec())?;
        let matrix = vandermonde
            .iter()
            .map(|row| {
                (0..n)
                    .map(|c| {
                        row.iter()
                            .zip(&top_inverse)
                            .fold(0, |acc, (v, inv_row)| acc ^ mul(*v, inv_row[c]))
                    })
                    .collect()
            })
            .collect();
        Ok(Self { n, matrix })
    }

    /// Parity shards of `data` (`n` shards of one length).
    pub(crate) fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, String> {
        if data.len() != self.n {
            return Err(format!("{} data shards, want {}", data.len(), self.n));
        }
        let len = shard_len(data.iter())?;
        let rows: Vec<&[u8]> = self.matrix[self.n..].iter().map(Vec::as_slice).collect();
        let data: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
        Ok(apply(&rows, &data, len))
    }

    /// Fills the missing shards of `shards` (`n` data then `n` parity)
    /// from any `n` present ones.
    pub(crate) fn reconstruct(&self, shards: &mut [Option<Vec<u8>>]) -> Result<(), String> {
        let n = self.n;
        if shards.len() != 2 * n {
            return Err(format!("{} shards, want {}", shards.len(), 2 * n));
        }
        let len = shard_len(shards.iter().flatten())?;
        if shards[..n].iter().any(Option::is_none) {
            // The first n present shards are an invertible combination of
            // the data shards.
            let present: Vec<usize> = (0..2 * n)
                .filter(|&i| shards[i].is_some())
                .take(n)
                .collect();
            if present.len() < n {
                return Err("too few shards".into());
            }
            let decode = invert(present.iter().map(|&i| self.matrix[i].clone()).collect())?;
            let missing: Vec<usize> = (0..n).filter(|&i| shards[i].is_none()).collect();
            let rows: Vec<&[u8]> = missing.iter().map(|&i| decode[i].as_slice()).collect();
            let inputs: Vec<&[u8]> = present
                .iter()
                .filter_map(|&i| shards[i].as_deref())
                .collect();
            let restored = apply(&rows, &inputs, len);
            for (i, shard) in missing.into_iter().zip(restored) {
                shards[i] = Some(shard);
            }
        }
        let missing: Vec<usize> = (n..2 * n).filter(|&i| shards[i].is_none()).collect();
        let rows: Vec<&[u8]> = missing.iter().map(|&i| self.matrix[i].as_slice()).collect();
        let data: Vec<&[u8]> = shards[..n].iter().filter_map(Option::as_deref).collect();
        let restored = apply(&rows, &data, len);
        for (i, shard) in missing.into_iter().zip(restored) {
            shards[i] = Some(shard);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Deterministic shards: `n` of `len` bytes.
    fn shards(n: usize, len: usize) -> Vec<Vec<u8>> {
        let mut x: u32 = 0x9e37_79b9 ^ u32::try_from(n * 31 + len).unwrap();
        (0..n)
            .map(|_| {
                (0..len)
                    .map(|_| {
                        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        x.to_be_bytes()[0]
                    })
                    .collect()
            })
            .collect()
    }

    fn parity_hex(n: usize, len: usize) -> String {
        let parity = Code::new(n).unwrap().encode(&shards(n, len)).unwrap();
        hex::encode(parity.concat())
    }

    /// Parity recorded from reed-solomon-erasure 6.0.0 (`galois_8`), which
    /// the Go vectors showed byte-identical to klauspost.
    #[test]
    fn parity_matches_recorded_vectors() {
        assert_eq!(parity_hex(1, 4), GOLDEN_1_4);
        assert_eq!(parity_hex(2, 4), GOLDEN_2_4);
        assert_eq!(parity_hex(3, 4), GOLDEN_3_4);
        assert_eq!(parity_hex(5, 3), GOLDEN_5_3);
        assert_eq!(
            hex::encode(hash::sha256(
                &Code::new(128)
                    .unwrap()
                    .encode(&shards(128, 33))
                    .unwrap()
                    .concat()
            )),
            GOLDEN_128_33_SHA256,
        );
    }

    const GOLDEN_1_4: &str = "3f19053a";
    const GOLDEN_2_4: &str = "62ec820af966fb66";
    const GOLDEN_3_4: &str = "9e7faf42fabb869bc1400db0";
    const GOLDEN_5_3: &str = "893a2cf6e565a5a7d5cfa3d0402a0c";
    const GOLDEN_128_33_SHA256: &str =
        "44c80fb5f86c59aaf392787e929ea41ef3a16d8134cc6262f8a32d7e6ea5cd56";

    /// Every way to lose up to `n` of the `2n` shards gives them back.
    #[test]
    fn reconstructs_every_erasure_pattern() {
        for n in 1..=4usize {
            let code = Code::new(n).unwrap();
            let data = shards(n, 7);
            let mut full = data.clone();
            full.extend(code.encode(&data).unwrap());
            for mask in 0u32..(1 << (2 * n)) {
                if mask.count_ones() as usize > n {
                    continue;
                }
                let mut got: Vec<Option<Vec<u8>>> = full
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (mask & (1 << i) == 0).then(|| s.clone()))
                    .collect();
                code.reconstruct(&mut got).unwrap();
                let got: Vec<Vec<u8>> = got.into_iter().map(Option::unwrap).collect();
                assert_eq!(got, full, "n {n} mask {mask:b}");
            }
        }
    }

    #[test]
    fn reconstructs_at_the_largest_size() {
        let n = MAX_SHARDS / 2;
        let code = Code::new(n).unwrap();
        let data = shards(n, 16);
        let mut full = data.clone();
        full.extend(code.encode(&data).unwrap());
        // Keep the odd data shards and the first parity shards.
        let mut got: Vec<Option<Vec<u8>>> = full
            .iter()
            .enumerate()
            .map(|(i, s)| ((i < n && i % 2 == 1) || (n..n + n / 2).contains(&i)).then(|| s.clone()))
            .collect();
        code.reconstruct(&mut got).unwrap();
        let got: Vec<Vec<u8>> = got.into_iter().map(Option::unwrap).collect();
        assert_eq!(got, full);
    }

    #[test]
    fn rejects_invalid_input() {
        assert!(Code::new(0).is_err());
        assert!(Code::new(MAX_SHARDS / 2 + 1).is_err());
        let code = Code::new(2).unwrap();
        // Wrong shard count, uneven shards, empty shards.
        assert!(code.encode(&[vec![1, 2]]).is_err());
        assert!(code.encode(&[vec![1, 2], vec![3]]).is_err());
        assert!(code.encode(&[vec![], vec![]]).is_err());
        // Too few shards to reconstruct.
        let mut few = vec![Some(vec![1, 2]), None, None, None];
        assert!(code.reconstruct(&mut few).is_err());
        let mut uneven = vec![Some(vec![1, 2]), Some(vec![3]), None, None];
        assert!(code.reconstruct(&mut uneven).is_err());
        let mut short = vec![Some(vec![1, 2]), Some(vec![3, 4])];
        assert!(code.reconstruct(&mut short).is_err());
    }
}
