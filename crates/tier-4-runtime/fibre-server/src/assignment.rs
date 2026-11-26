//! Validator row assignment: `fibre/validator/set.go` of celestia-app
//! v11.0.0 (`AssignedRows`, `Assign`, `ShardMap.Verify`) and the parts of
//! Go's `math/rand/v2` it relies on (`rand.New(rand.NewChaCha8(seed))`,
//! `Rand.Shuffle`, `Rand.Perm`).
//!
//! Validators are addressed by their position in the validator set, which
//! must be in CometBFT order (voting power descending, then address
//! ascending), as `core.ValidatorSet.Validators` is.

use crate::params::Fraction;
use chacha8rand::ChaCha8Rand;
use std::collections::HashMap;

/// `rand.New(rand.NewChaCha8(seed))` of Go's `math/rand/v2` on a 64-bit
/// platform.
#[derive(Debug, Clone)]
pub struct GoChaCha8Rand {
    inner: ChaCha8Rand,
}

impl GoChaCha8Rand {
    /// `rand.New(rand.NewChaCha8(seed))`.
    #[must_use]
    pub fn new(seed: &[u8; 32]) -> Self {
        Self {
            inner: ChaCha8Rand::new(seed),
        }
    }

    /// `Rand.Uint64` (`ChaCha8.Uint64`).
    pub fn uint64(&mut self) -> u64 {
        self.inner.read_u64()
    }

    /// `Rand.uint64n`: a uniform value in `[0, n)` (Lemire's method, as Go).
    pub fn uint64n(&mut self, n: u64) -> u64 {
        if n & n.wrapping_sub(1) == 0 {
            return self.uint64() & n.wrapping_sub(1);
        }
        let mul = |a: u64| {
            let full = u128::from(a) * u128::from(n);
            #[allow(clippy::cast_possible_truncation)]
            ((full >> 64) as u64, full as u64)
        };
        let (mut hi, mut lo) = mul(self.uint64());
        if lo < n {
            let thresh = n.wrapping_neg() % n;
            while lo < thresh {
                (hi, lo) = mul(self.uint64());
            }
        }
        hi
    }

    /// `Rand.Shuffle`: Fisher-Yates from the end.
    pub fn shuffle(&mut self, n: usize, mut swap: impl FnMut(usize, usize)) {
        for i in (1..n).rev() {
            #[allow(clippy::cast_possible_truncation)]
            let j = self.uint64n(i as u64 + 1) as usize;
            swap(i, j);
        }
    }

    /// `Rand.Perm`.
    pub fn perm(&mut self, n: usize) -> Vec<usize> {
        let mut p: Vec<usize> = (0..n).collect();
        self.shuffle(n, |i, j| p.swap(i, j));
        p
    }
}

/// `Set.AssignedRows`: ceil(originalRows * votingPower * lt.Denominator /
/// (totalVotingPower * lt.Numerator)), clamped to `[min_rows,
/// original_rows]`.
///
/// The products use Go's wrapping `int64` arithmetic, so an (unrealistic)
/// overflowing voting power yields Go's result. Where Go would divide by
/// zero (zero total power or numerator) the unclamped count is taken as 0.
#[must_use]
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn assigned_rows(
    voting_power: i64,
    total_voting_power: i64,
    original_rows: usize,
    min_rows: usize,
    liveness_threshold: Fraction,
) -> usize {
    let num = (original_rows as i64)
        .wrapping_mul(voting_power)
        .wrapping_mul(liveness_threshold.denominator as i64);
    let den = total_voting_power.wrapping_mul(liveness_threshold.numerator as i64);
    let rows = if den == 0 {
        0
    } else {
        num.wrapping_add(den).wrapping_sub(1).wrapping_div(den)
    };
    rows.max(min_rows as i64).min(original_rows as i64) as usize
}

/// An assignment check failure (`ShardMap.Verify`); the text is Go's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssignmentError {
    /// The validator has no entry in the shard map.
    #[error("validator not in shard map")]
    ValidatorNotInMap,
    /// The shard carries a different number of rows than assigned.
    #[error("expected {expected} rows, got {got}")]
    RowCount {
        /// Assigned rows.
        expected: usize,
        /// Uploaded rows.
        got: usize,
    },
    /// A row index that is not assigned to the validator.
    #[error("row {0} not assigned to validator")]
    NotAssigned(u32),
    /// A row index that appears twice.
    #[error("duplicate row {0}")]
    Duplicate(u32),
}

/// `ShardMap`: the row indices assigned to each validator, by position in
/// the validator set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShardMap {
    rows: Vec<Vec<u32>>,
}

impl ShardMap {
    /// The rows assigned to the validator at `validator_index`.
    #[must_use]
    pub fn rows(&self, validator_index: usize) -> Option<&[u32]> {
        self.rows.get(validator_index).map(Vec::as_slice)
    }

    /// Number of validators in the map (0 for Go's empty map).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `ShardMap.Verify`: `row_indices` must be exactly the validator's
    /// assigned rows (count, membership, no duplicates; any order).
    pub fn verify(
        &self,
        validator_index: usize,
        row_indices: &[u32],
    ) -> Result<(), AssignmentError> {
        let rows = self
            .rows(validator_index)
            .ok_or(AssignmentError::ValidatorNotInMap)?;
        verify_rows(rows, row_indices)
    }
}

/// The checks of `ShardMap.Verify` against one validator's assigned rows.
pub fn verify_rows(assigned: &[u32], row_indices: &[u32]) -> Result<(), AssignmentError> {
    if row_indices.len() != assigned.len() {
        return Err(AssignmentError::RowCount {
            expected: assigned.len(),
            got: row_indices.len(),
        });
    }
    let mut seen: HashMap<u32, bool> = assigned.iter().map(|&r| (r, false)).collect();
    for &row in row_indices {
        match seen.get_mut(&row) {
            None => return Err(AssignmentError::NotAssigned(row)),
            Some(true) => return Err(AssignmentError::Duplicate(row)),
            Some(s) => *s = true,
        }
    }
    Ok(())
}

/// `Set.Assign`: shuffles `[0, total_rows)` with a ChaCha8 RNG seeded by the
/// commitment and hands each validator, in set order, its
/// [`assigned_rows`] consecutive shuffled indices, wrapping modulo
/// `total_rows`. Empty when there are no validators or `total_rows` or
/// `min_rows` is 0.
#[must_use]
pub fn assign(
    commitment: &[u8; 32],
    voting_powers: &[i64],
    total_rows: usize,
    original_rows: usize,
    min_rows: usize,
    liveness_threshold: Fraction,
) -> ShardMap {
    if voting_powers.is_empty() || total_rows == 0 || min_rows == 0 {
        return ShardMap::default();
    }
    let total_voting_power = voting_powers.iter().fold(0i64, |a, &p| a.wrapping_add(p));

    let mut rng = GoChaCha8Rand::new(commitment);
    #[allow(clippy::cast_possible_truncation)]
    let mut indices: Vec<u32> = (0..total_rows).map(|i| i as u32).collect();
    rng.shuffle(total_rows, |i, j| indices.swap(i, j));

    let mut offset = 0usize;
    let rows = voting_powers
        .iter()
        .map(|&vp| {
            let n = assigned_rows(
                vp,
                total_voting_power,
                original_rows,
                min_rows,
                liveness_threshold,
            );
            let rows = (0..n).map(|j| indices[(offset + j) % total_rows]).collect();
            offset += n;
            rows
        })
        .collect();
    ShardMap { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::DEFAULT_PROTOCOL_PARAMS;
    use serde::Deserialize;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../../../conformance/fixtures/fibre-server/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        if name.ends_with(".zst") {
            zstd::decode_all(raw.as_slice()).expect("zstd")
        } else {
            raw
        }
    }

    fn hex32(s: &str) -> [u8; 32] {
        hex::decode(s).expect("hex").try_into().expect("32 bytes")
    }

    #[derive(Deserialize)]
    struct ChachaFile {
        cases: Vec<ChachaCase>,
    }
    #[derive(Deserialize)]
    struct ChachaCase {
        seed: String,
        uint64s: Vec<String>,
        shuffles: Vec<ShuffleCase>,
        perm50: Vec<usize>,
    }
    #[derive(Deserialize)]
    struct ShuffleCase {
        n: usize,
        #[serde(default)]
        result: Vec<usize>,
        sha256: String,
    }

    #[test]
    fn chacha8_matches_go() {
        let f: ChachaFile = serde_json::from_slice(&fixture("chacha8.json")).expect("json");
        assert_eq!(f.cases.len(), 3);
        for c in f.cases {
            let seed = hex32(&c.seed);
            let mut rng = GoChaCha8Rand::new(&seed);
            for (i, want) in c.uint64s.iter().enumerate() {
                assert_eq!(
                    rng.uint64().to_string(),
                    *want,
                    "seed {} output {i}",
                    c.seed
                );
            }
            for s in c.shuffles {
                let mut rng = GoChaCha8Rand::new(&seed);
                let mut idx: Vec<usize> = (0..s.n).collect();
                rng.shuffle(s.n, |i, j| idx.swap(i, j));
                if s.n <= 64 {
                    assert_eq!(idx, s.result, "shuffle n={}", s.n);
                }
                let bytes: Vec<u8> = idx
                    .iter()
                    .flat_map(|&v| u32::try_from(v).expect("u32").to_le_bytes())
                    .collect();
                assert_eq!(
                    hex::encode(hash::sha256(&bytes)),
                    s.sha256,
                    "shuffle n={}",
                    s.n
                );
            }
            assert_eq!(GoChaCha8Rand::new(&seed).perm(50), c.perm50);
        }
    }

    #[derive(Deserialize)]
    struct AssignmentFile {
        protocol_params: HashMap<String, usize>,
        cases: Vec<AssignmentCase>,
    }
    #[derive(Deserialize)]
    struct FractionJson {
        numerator: u64,
        denominator: u64,
    }
    #[derive(Deserialize)]
    struct AssignmentCase {
        name: String,
        commitment: String,
        total_rows: usize,
        original_rows: usize,
        min_rows: usize,
        liveness_threshold: FractionJson,
        total_voting_power: i64,
        validators: Vec<AssignedValidator>,
        #[serde(default)]
        verify: Vec<VerifyCase>,
    }
    #[derive(Deserialize)]
    struct AssignedValidator {
        voting_power: i64,
        assigned_rows: usize,
        rows: Vec<u32>,
    }
    #[derive(Deserialize)]
    struct VerifyCase {
        name: String,
        validator_index: i64,
        row_indices: Vec<u32>,
        #[serde(default)]
        error: String,
    }

    #[test]
    fn assignment_matches_go() {
        let f: AssignmentFile =
            serde_json::from_slice(&fixture("assignment.json.zst")).expect("json");
        assert_eq!(f.cases.len(), 6);
        for c in &f.cases {
            let lt = Fraction {
                numerator: c.liveness_threshold.numerator,
                denominator: c.liveness_threshold.denominator,
            };
            let powers: Vec<i64> = c.validators.iter().map(|v| v.voting_power).collect();
            assert_eq!(
                powers.iter().sum::<i64>(),
                c.total_voting_power,
                "{}",
                c.name
            );
            let sm = assign(
                &hex32(&c.commitment),
                &powers,
                c.total_rows,
                c.original_rows,
                c.min_rows,
                lt,
            );
            for (i, v) in c.validators.iter().enumerate() {
                assert_eq!(
                    assigned_rows(
                        v.voting_power,
                        c.total_voting_power,
                        c.original_rows,
                        c.min_rows,
                        lt
                    ),
                    v.assigned_rows,
                    "{} validator {i} count",
                    c.name
                );
                let got = sm.rows(i).unwrap_or(&[]);
                assert_eq!(got, v.rows.as_slice(), "{} validator {i} rows", c.name);
            }
            if c.min_rows == 0 {
                assert!(sm.is_empty(), "{}", c.name);
            }
            for vc in &c.verify {
                let idx = usize::try_from(vc.validator_index).unwrap_or(usize::MAX);
                let got = sm
                    .verify(idx, &vc.row_indices)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                assert_eq!(got, vc.error, "{} verify {}", c.name, vc.name);
            }
        }

        let p = DEFAULT_PROTOCOL_PARAMS;
        let pp = &f.protocol_params;
        assert_eq!(pp["rows"], p.rows);
        assert_eq!(pp["total_rows"], p.total_rows());
        assert_eq!(pp["parity_rows"], p.parity_rows());
        assert_eq!(pp["codec_work_rows"], p.codec_work_rows());
        assert_eq!(pp["max_rows_per_validator"], p.max_rows_per_validator());
        assert_eq!(pp["min_rows_per_validator"], p.min_rows_per_validator());
        assert_eq!(
            pp["validators_for_reconstruction"],
            p.validators_for_reconstruction()
        );
        assert_eq!(Ok(pp["max_row_size"]), p.max_row_size(0));
        assert_eq!(pp["max_shard_size"], p.max_shard_size());
        assert_eq!(pp["merkle_proof_depth"], p.merkle_proof_depth());
        assert_eq!(pp["max_message_size"], p.max_message_size());
        assert_eq!(
            pp["max_payment_promise_size"],
            crate::params::MAX_PAYMENT_PROMISE_SIZE
        );
        let cfg = crate::params::BlobConfig::default_v0();
        assert_eq!(pp["blob_max_data_size"], cfg.max_data_size);
        assert_eq!(pp["upload_size_1"], cfg.upload_size(1));
        assert_eq!(pp["upload_size_300000"], cfg.upload_size(300_000));
    }

    #[test]
    fn empty_inputs_give_empty_map() {
        let lt = Fraction {
            numerator: 1,
            denominator: 3,
        };
        assert!(assign(&[0; 32], &[], 16, 4, 2, lt).is_empty());
        assert!(assign(&[0; 32], &[1], 0, 4, 2, lt).is_empty());
        assert_eq!(
            ShardMap::default().verify(0, &[]),
            Err(AssignmentError::ValidatorNotInMap)
        );
    }
}
