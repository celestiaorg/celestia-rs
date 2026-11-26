//! Shard validation of `UploadShard` (steps 10-11 of the server spec):
//! `Server.verifyShard`, `parseRowSize` and `parseRow` of celestia-app
//! v11.0.0 `fibre/server_upload.go`, and `rsema1d.Verifier.Verify` of
//! `pkg/rsema1d/verifier.go`. The assignment check (step 10) is
//! [`crate::assignment::ShardMap::verify`].

use crate::params::{BlobConfig, GF128_SIZE, LEOPARD_CHUNK_SIZE, MERKLE_NODE_SIZE};
use rsema1d::GF128;
use rsema1d::crypto::{hash_internal, hash_leaf};

/// A borrowed view of one `celestia.fibre.v1.BlobRow`.
#[derive(Debug, Clone, Copy)]
pub struct ShardRow<'a> {
    /// Row index in `[0, K+N)`.
    pub index: u32,
    /// Row data.
    pub data: &'a [u8],
    /// Merkle proof, leaf to root.
    pub proof: &'a [Vec<u8>],
}

/// A shard validation failure. `Display` is Go's error text, which the
/// `UploadShard` handler wraps as `shard verification failed: {err}`
/// (gRPC `InvalidArgument`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShardError {
    /// The shard has no rows.
    #[error("no rows provided")]
    NoRows,
    /// The first row is empty.
    #[error("row size cannot be zero")]
    ZeroRowSize,
    /// A row (by position) differs in size from the first.
    #[error(
        "row {position} has size {size}, expected {expected} (all rows must have the same size)"
    )]
    UnequalRowSize {
        /// Position of the row in the shard.
        position: usize,
        /// Its size.
        size: usize,
        /// The first row's size.
        expected: usize,
    },
    /// Rows are larger than the blob configuration allows.
    #[error("row size {size} exceeds maximum {max}")]
    RowTooLarge {
        /// Row size.
        size: usize,
        /// `BlobConfig.MaxRowSize`.
        max: usize,
    },
    /// The promise's blob size is not row size times original rows.
    #[error(
        "upload size mismatch: promise has {upload_size}, but row size {row_size} * {original_rows} original rows = {expected}"
    )]
    UploadSizeMismatch {
        /// `PaymentPromise.UploadSize`.
        upload_size: u32,
        /// Row size.
        row_size: usize,
        /// K.
        original_rows: usize,
        /// `row_size * original_rows`.
        expected: usize,
    },
    /// The RLC vector is not one GF128 value per original row.
    #[error("expected {expected} RLC bytes, got {got}")]
    RlcLength {
        /// `K * 16`.
        expected: usize,
        /// Received bytes.
        got: usize,
    },
    /// `rlc.Unmarshal` rejected the RLC bytes.
    #[error("GF128 byte length must be a multiple of {GF128_SIZE}, got {0}")]
    RlcMisaligned(usize),
    /// A row has no proof.
    #[error("row {0} missing proof")]
    MissingProof(u32),
    /// A row has no data.
    #[error("row {0} missing data")]
    MissingData(u32),
    /// A proof segment is not 32 bytes.
    #[error("row {index} proof segment {segment} must be {MERKLE_NODE_SIZE} bytes, got {size}")]
    ProofSegmentSize {
        /// Row index.
        index: u32,
        /// Segment position.
        segment: usize,
        /// Its size.
        size: usize,
    },
    /// `rsema1d.Verifier.Verify` rejected the rows.
    #[error("shard row verification failed: {0}")]
    Rows(#[from] RowVerifyError),
}

/// A rejection by `rsema1d.Verifier.Verify`; the text is Go's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RowVerifyError {
    /// Not K RLC values.
    #[error("expected {expected} RLC values, got {got}")]
    RlcCount {
        /// K.
        expected: usize,
        /// Received values.
        got: usize,
    },
    /// The Reed-Solomon extension of the RLC values failed.
    #[error("extending RLC: {0}")]
    ExtendRlc(String),
    /// No row proofs.
    #[error("no proofs provided")]
    NoProofs,
    /// A row index outside `[0, K+N)`.
    #[error("index {index} out of range [0, {total})")]
    IndexOutOfRange {
        /// Row index.
        index: u32,
        /// K + N.
        total: usize,
    },
    /// A row differs in size from the first.
    #[error(
        "batched verify requires equal-sized rows: row {index} has {size} bytes, expected {expected}"
    )]
    UnequalRows {
        /// Row index.
        index: u32,
        /// Its size.
        size: usize,
        /// The first row's size.
        expected: usize,
    },
    /// A proof (by position) has the wrong number of segments.
    #[error("row {position}: proof depth mismatch: expected {expected}, got {got}")]
    ProofDepth {
        /// Position of the row in the shard.
        position: usize,
        /// log2(K + N).
        expected: usize,
        /// Segments.
        got: usize,
    },
    /// The row size is not a positive multiple of 64.
    #[error("row size must be a positive multiple of {LEOPARD_CHUNK_SIZE}, got {0}")]
    RowSize(usize),
    /// A proof leads to a different root than the first of its chunk.
    #[error("verifying row proofs: input {input} (tree index {index}): root mismatch")]
    RootMismatch {
        /// Position of the row in the shard.
        input: usize,
        /// Row index.
        index: u32,
    },
    /// A chunk of proofs leads to a different root than the first chunk.
    #[error("verifying row proofs: proof chunk {0}: root mismatch")]
    ChunkRootMismatch(usize),
    /// `sha256(rowRoot || rlcRoot)` is not the commitment.
    #[error("commitment verification failed")]
    Commitment,
    /// A row's RLC differs from the extended RLC value at its index.
    #[error("row {0}: computed RLC does not match expected value")]
    RlcMismatch(u32),
}

/// Validates uploaded shards against one blob layout (Go's
/// `Server.verifyShard` with a pooled `rsema1d.Verifier`).
///
/// Go's pool only bounds how many verifications run at once; that limit
/// is left to the caller. A `ShardVerifier` holds no per-call state and may
/// be shared across threads.
#[derive(Debug, Clone)]
pub struct ShardVerifier {
    original_rows: usize,
    parity_rows: usize,
    max_row_size: usize,
    merkle_workers: usize,
}

impl ShardVerifier {
    /// A verifier for `cfg`. The Merkle worker count defaults to the
    /// available parallelism, Go's `GOMAXPROCS`.
    #[must_use]
    pub fn new(cfg: &BlobConfig) -> Self {
        Self {
            original_rows: cfg.original_rows,
            parity_rows: cfg.parity_rows,
            max_row_size: cfg.max_row_size,
            merkle_workers: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        }
    }

    /// Sets the worker count that Go's `merkle.RootFromProofs` would split
    /// the row proofs across (`GOMAXPROCS`). It only selects which
    /// root-mismatch error text a tampered shard gets; the proofs are
    /// checked on the calling thread.
    #[must_use]
    pub fn with_merkle_workers(mut self, workers: usize) -> Self {
        self.merkle_workers = workers;
        self
    }

    /// `Server.verifyShard`: the row-size, upload-size and RLC-length
    /// checks, the per-row shape checks, then [`verify_row_proofs`].
    pub fn verify_shard(
        &self,
        commitment: &[u8; 32],
        upload_size: u32,
        rows: &[ShardRow<'_>],
        rlcs: &[u8],
    ) -> Result<(), ShardError> {
        let row_size = parse_row_size(rows)?;
        if self.max_row_size > 0 && row_size > self.max_row_size {
            return Err(ShardError::RowTooLarge {
                size: row_size,
                max: self.max_row_size,
            });
        }
        let expected = row_size * self.original_rows;
        if upload_size as usize != expected {
            return Err(ShardError::UploadSizeMismatch {
                upload_size,
                row_size,
                original_rows: self.original_rows,
                expected,
            });
        }
        let expected_rlc_bytes = self.original_rows * GF128_SIZE;
        if rlcs.len() != expected_rlc_bytes {
            return Err(ShardError::RlcLength {
                expected: expected_rlc_bytes,
                got: rlcs.len(),
            });
        }
        let rlcs = unmarshal_rlcs(rlcs)?;
        for row in rows {
            parse_row(row)?;
        }
        verify_row_proofs(
            self.original_rows,
            self.parity_rows,
            commitment,
            rows,
            &rlcs,
            self.merkle_workers,
        )?;
        Ok(())
    }
}

/// `parseRowSize`.
fn parse_row_size(rows: &[ShardRow<'_>]) -> Result<usize, ShardError> {
    let first = rows.first().ok_or(ShardError::NoRows)?;
    let row_size = first.data.len();
    if row_size == 0 {
        return Err(ShardError::ZeroRowSize);
    }
    for (position, row) in rows.iter().enumerate().skip(1) {
        if row.data.len() != row_size {
            return Err(ShardError::UnequalRowSize {
                position,
                size: row.data.len(),
                expected: row_size,
            });
        }
    }
    Ok(row_size)
}

/// `parseRow`.
fn parse_row(row: &ShardRow<'_>) -> Result<(), ShardError> {
    if row.proof.is_empty() {
        return Err(ShardError::MissingProof(row.index));
    }
    if row.data.is_empty() {
        return Err(ShardError::MissingData(row.index));
    }
    for (segment, seg) in row.proof.iter().enumerate() {
        if seg.len() != MERKLE_NODE_SIZE {
            return Err(ShardError::ProofSegmentSize {
                index: row.index,
                segment,
                size: seg.len(),
            });
        }
    }
    Ok(())
}

/// `rlc.Unmarshal`: little-endian GF128 values, 16 bytes each.
pub fn unmarshal_rlcs(src: &[u8]) -> Result<Vec<GF128>, ShardError> {
    if src.len() % GF128_SIZE != 0 {
        return Err(ShardError::RlcMisaligned(src.len()));
    }
    Ok(src
        .chunks_exact(GF128_SIZE)
        .map(|c| {
            let mut b = [0u8; GF128_SIZE];
            b.copy_from_slice(c);
            GF128::from_bytes(&b)
        })
        .collect())
}

/// `merkle.RootFromProof`. Segments must be 32 bytes (checked by
/// `parse_row` before this runs).
fn root_from_proof(leaf: &[u8], index: u32, proof: &[Vec<u8>]) -> [u8; 32] {
    let mut current = hash_leaf(leaf);
    let mut pos = index;
    for seg in proof {
        let mut sibling = [0u8; 32];
        sibling.copy_from_slice(seg);
        current = if pos % 2 == 0 {
            hash_internal(&current, &sibling)
        } else {
            hash_internal(&sibling, &current)
        };
        pos /= 2;
    }
    current
}

/// `merkle.reduceProofs` over `rows[start..end]`.
fn reduce_proofs(
    rows: &[ShardRow<'_>],
    start: usize,
    end: usize,
) -> Result<[u8; 32], RowVerifyError> {
    let first = &rows[start];
    let want = root_from_proof(first.data, first.index, first.proof);
    for (input, row) in rows.iter().enumerate().take(end).skip(start + 1) {
        if root_from_proof(row.data, row.index, row.proof) != want {
            return Err(RowVerifyError::RootMismatch {
                input,
                index: row.index,
            });
        }
    }
    Ok(want)
}

/// `merkle.RootFromProofs`: the root all proofs share. Go splits the
/// proofs into `min(workers, len/32)` contiguous chunks reduced in
/// parallel and reports the first failing chunk in order; this reproduces
/// that split (and so Go's error text) on one thread.
fn root_from_proofs(rows: &[ShardRow<'_>], workers: usize) -> Result<[u8; 32], RowVerifyError> {
    const PROOF_GRAIN: usize = 32;
    let count = rows.len();
    let nw = if workers <= 1 {
        1
    } else {
        workers.min((count / PROOF_GRAIN).max(1))
    };
    if nw <= 1 {
        return reduce_proofs(rows, 0, count);
    }
    let w = nw.min(count);
    let chunk = count.div_ceil(w);
    let chunks = count.div_ceil(chunk);
    let results: Vec<_> = (0..chunks)
        .map(|c| reduce_proofs(rows, c * chunk, ((c + 1) * chunk).min(count)))
        .collect();
    let mut first = None;
    for (c, r) in results.into_iter().enumerate() {
        let root = r?;
        match first {
            None => first = Some(root),
            Some(f) if f != root => return Err(RowVerifyError::ChunkRootMismatch(c)),
            Some(_) => {}
        }
    }
    first.ok_or(RowVerifyError::NoProofs)
}

/// Merkle root over the K serialized RLC values (`computeRLCRoot`; K is
/// a power of two).
fn rlc_root(rlcs: &[GF128]) -> [u8; 32] {
    let mut level: Vec<[u8; 32]> = rlcs.iter().map(|g| hash_leaf(&g.to_bytes())).collect();
    while level.len() > 1 {
        level = level
            .chunks_exact(2)
            .map(|p| hash_internal(&p[0], &p[1]))
            .collect();
    }
    level.first().copied().unwrap_or_else(|| hash_leaf(&[]))
}

/// `rsema1d.Verifier.Verify` for a `K`/`N` layout: extends the K RLC values
/// with Reed-Solomon, checks the proof shapes, recovers the shared row root
/// from the proofs, checks `sha256(rowRoot || rlcRoot)` against the
/// commitment and compares each row's RLC (under the Fiat-Shamir
/// coefficients of the row root) with the extended RLC value at its index.
///
/// `merkle_workers` is Go's `GOMAXPROCS`; see
/// [`ShardVerifier::with_merkle_workers`].
pub fn verify_row_proofs(
    k: usize,
    n: usize,
    commitment: &[u8; 32],
    rows: &[ShardRow<'_>],
    rlcs: &[GF128],
    merkle_workers: usize,
) -> Result<(), RowVerifyError> {
    // setRLC
    if rlcs.len() != k {
        return Err(RowVerifyError::RlcCount {
            expected: k,
            got: rlcs.len(),
        });
    }
    let extended = rsema1d::codec::extend_rlcs(rlcs, k, n)
        .map_err(|e| RowVerifyError::ExtendRlc(e.to_string()))?;
    let rlc_root = rlc_root(rlcs);

    // validateProofs
    let total = k + n;
    let first = rows.first().ok_or(RowVerifyError::NoProofs)?;
    let expected_depth = (usize::BITS - total.leading_zeros()) as usize - 1;
    let row_size = first.data.len();
    for (position, row) in rows.iter().enumerate() {
        if row.index as usize >= total {
            return Err(RowVerifyError::IndexOutOfRange {
                index: row.index,
                total,
            });
        }
        if row.data.len() != row_size {
            return Err(RowVerifyError::UnequalRows {
                index: row.index,
                size: row.data.len(),
                expected: row_size,
            });
        }
        if row.proof.len() != expected_depth {
            return Err(RowVerifyError::ProofDepth {
                position,
                expected: expected_depth,
                got: row.proof.len(),
            });
        }
    }
    if row_size == 0 || row_size % LEOPARD_CHUNK_SIZE != 0 {
        return Err(RowVerifyError::RowSize(row_size));
    }

    // verify
    let row_root = root_from_proofs(rows, merkle_workers)?;
    let mut h = hash::Sha256Hasher::new();
    h.update(&row_root);
    h.update(&rlc_root);
    if h.finalize() != *commitment {
        return Err(RowVerifyError::Commitment);
    }
    let coeffs = rsema1d::crypto::derive_coefficients(&row_root, k, n, row_size);
    for row in rows {
        if rsema1d::codec::compute_rlc(row.data, &coeffs) != extended[row.index as usize] {
            return Err(RowVerifyError::RlcMismatch(row.index));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::assign;
    use crate::params::DEFAULT_PROTOCOL_PARAMS;
    use serde::Deserialize;

    /// Go marshals nil slices as `null`.
    fn null_vec<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Vec<T>, D::Error> {
        Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
    }

    #[derive(Deserialize)]
    struct File {
        gomaxprocs: usize,
        cases: Vec<Case>,
    }
    #[derive(Deserialize, Clone)]
    struct RowJson {
        index: u32,
        data: String,
        #[serde(default, deserialize_with = "null_vec")]
        proof: Vec<String>,
    }
    #[derive(Deserialize)]
    struct ShardJson {
        rows: Vec<RowJson>,
        rlcs: String,
    }
    #[derive(Deserialize)]
    struct Mutation {
        name: String,
        op: String,
        #[serde(default)]
        row: usize,
        #[serde(default)]
        arg: i64,
        assignment_error: String,
        shard_error: String,
    }
    #[derive(Deserialize)]
    struct Case {
        name: String,
        commitment: String,
        upload_size: u32,
        validator_index: i64,
        #[serde(default, deserialize_with = "null_vec")]
        voting_powers: Vec<i64>,
        shard: ShardJson,
        mutations: Vec<Mutation>,
        #[serde(default)]
        alt_rows_start: usize,
        #[serde(default, deserialize_with = "null_vec")]
        alt_rows: Vec<RowJson>,
    }

    #[derive(Clone)]
    struct Row {
        index: u32,
        data: Vec<u8>,
        proof: Vec<Vec<u8>>,
    }

    fn row(r: &RowJson) -> Row {
        Row {
            index: r.index,
            data: hex::decode(&r.data).expect("hex"),
            proof: r
                .proof
                .iter()
                .map(|p| hex::decode(p).expect("hex"))
                .collect(),
        }
    }

    struct State {
        commitment: [u8; 32],
        upload_size: u32,
        rows: Vec<Row>,
        rlcs: Vec<u8>,
    }

    /// The mutations of conformance/fibre-server/main.go `apply`.
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap
    )]
    fn apply(st: &mut State, m: &Mutation, unassigned: u32, alt: &[Row], alt_start: usize) {
        let arg = m.arg as usize;
        let rows = &mut st.rows;
        match m.op.as_str() {
            "none" => {}
            "reverse_rows" => rows.reverse(),
            "drop_last_row" => {
                rows.pop();
            }
            "drop_row" => {
                rows.remove(m.row);
            }
            "drop_all_rows" => rows.clear(),
            "set_index_unassigned" => rows[m.row].index = unassigned,
            "set_index" => rows[m.row].index = m.arg as u32,
            "duplicate_index" => rows[m.row].index = rows[arg].index,
            "truncate_row_data" => {
                let n = rows[m.row].data.len() - arg;
                rows[m.row].data.truncate(n);
            }
            "clear_all_data" => rows.iter_mut().for_each(|r| r.data.clear()),
            "clear_row_data" => rows[m.row].data.clear(),
            "resize_all_rows" => {
                rows.iter_mut().for_each(|r| r.data.resize(arg, 0));
                st.upload_size = (arg * DEFAULT_PROTOCOL_PARAMS.rows) as u32;
            }
            "upload_size_delta" => st.upload_size = (i64::from(st.upload_size) + m.arg) as u32,
            "truncate_rlcs" => {
                let n = st.rlcs.len() - arg;
                st.rlcs.truncate(n);
            }
            "flip_rlc_byte" => st.rlcs[arg] ^= 1,
            "clear_proof" => rows[m.row].proof.clear(),
            "truncate_proof_segment" => rows[m.row].proof[arg].truncate(31),
            "extend_proof_segment" => rows[m.row].proof[arg].push(0),
            "drop_last_proof_segment" => {
                rows[m.row].proof.pop();
            }
            "flip_proof_byte" => rows[m.row].proof[arg][0] ^= 1,
            "flip_row_byte" => rows[m.row].data[arg] ^= 1,
            "flip_commitment_byte" => st.commitment[arg] ^= 1,
            "graft_alt_rows" => {
                for i in m.row..arg {
                    rows[i] = alt[i - alt_start].clone();
                }
            }
            op => panic!("unknown op {op}"),
        }
    }

    #[test]
    fn row_verifier_shape_errors() {
        let c = [0u8; 32];
        assert_eq!(
            verify_row_proofs(4, 12, &c, &[], &[GF128::zero(); 3], 1)
                .unwrap_err()
                .to_string(),
            "expected 4 RLC values, got 3"
        );
        assert_eq!(
            verify_row_proofs(4, 12, &c, &[], &[GF128::zero(); 4], 1)
                .unwrap_err()
                .to_string(),
            "no proofs provided"
        );
        assert_eq!(
            unmarshal_rlcs(&[0; 17]).unwrap_err().to_string(),
            "GF128 byte length must be a multiple of 16, got 17"
        );
        let proof = vec![vec![0u8; 32]; 4];
        let data = [0u8; 64];
        let row = ShardRow {
            index: 3,
            data: &data,
            proof: &proof,
        };
        let rows = vec![row; 64];
        // Sequential (one worker): the first differing proof is reported by
        // its position; identical proofs pass the Merkle step.
        assert_eq!(
            verify_row_proofs(4, 12, &c, &rows, &[GF128::zero(); 4], 1).unwrap_err(),
            RowVerifyError::Commitment
        );
    }

    #[test]
    fn shard_verification_matches_go() {
        let raw = std::fs::read(format!(
            "{}/../../../conformance/fixtures/fibre-server/shard-verify.json.zst",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture");
        let f: File =
            serde_json::from_slice(&zstd::decode_all(raw.as_slice()).expect("zstd")).expect("json");
        let cfg = BlobConfig::default_v0();
        let verifier = ShardVerifier::new(&cfg).with_merkle_workers(f.gomaxprocs);
        let p = DEFAULT_PROTOCOL_PARAMS;
        let mut checked = 0;
        for c in &f.cases {
            let commitment: [u8; 32] = hex::decode(&c.commitment)
                .expect("hex")
                .try_into()
                .expect("32");
            let base_rows: Vec<Row> = c.shard.rows.iter().map(row).collect();
            let alt: Vec<Row> = c.alt_rows.iter().map(row).collect();
            let rlcs = hex::decode(&c.shard.rlcs).expect("hex");
            let assigned = assign(
                &commitment,
                &c.voting_powers,
                cfg.total_rows(),
                cfg.original_rows,
                p.min_rows_per_validator(),
                p.liveness_threshold,
            );
            let unassigned = if c.validator_index >= 0 {
                let mine = assigned.rows(0).expect("validator 0");
                (0u32..)
                    .find(|i| !mine.contains(i))
                    .expect("unassigned row")
            } else {
                0
            };
            for m in &c.mutations {
                let mut st = State {
                    commitment,
                    upload_size: c.upload_size,
                    rows: base_rows.clone(),
                    rlcs: rlcs.clone(),
                };
                apply(&mut st, m, unassigned, &alt, c.alt_rows_start);
                if let Ok(idx) = usize::try_from(c.validator_index) {
                    let sm = assign(
                        &st.commitment,
                        &c.voting_powers,
                        cfg.total_rows(),
                        cfg.original_rows,
                        p.min_rows_per_validator(),
                        p.liveness_threshold,
                    );
                    let indices: Vec<u32> = st.rows.iter().map(|r| r.index).collect();
                    let got = sm
                        .verify(idx, &indices)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    assert_eq!(
                        got, m.assignment_error,
                        "{} / {} assignment",
                        c.name, m.name
                    );
                }
                let views: Vec<ShardRow<'_>> = st
                    .rows
                    .iter()
                    .map(|r| ShardRow {
                        index: r.index,
                        data: &r.data,
                        proof: &r.proof,
                    })
                    .collect();
                let got = verifier
                    .verify_shard(&st.commitment, st.upload_size, &views, &st.rlcs)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                assert_eq!(got, m.shard_error, "{} / {} shard", c.name, m.name);
                checked += 1;
            }
        }
        assert_eq!(checked, 31);
    }
}
