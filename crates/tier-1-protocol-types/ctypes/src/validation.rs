//! Commit verification (`types/validation.go`): `VerifyCommit`,
//! `VerifyCommitLight[AllSignatures]`,
//! `VerifyCommitLightTrusting[AllSignatures]`, with Go's batch and single
//! verification paths (they differ in which error wins).
//!
//! Ed25519 batch verification in Go (curve25519-voi, ZIP-215) accepts
//! exactly the signatures that single ZIP-215 verification accepts, so the
//! batch path verifies each batched signature individually and reports the
//! first invalid one in batch order, as Go does after a failed batch.

use crate::block_id::BlockId;
use crate::commit::{BLOCK_ID_FLAG_ABSENT, BLOCK_ID_FLAG_COMMIT, Commit, CommitSig};
use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::validator::Validator;
use crate::validator_set::{ValidatorSet, safe_mul};

const BATCH_VERIFY_THRESHOLD: usize = 2;

/// `cmtmath.Fraction` (trust level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    /// Numerator.
    pub numerator: u64,
    /// Denominator.
    pub denominator: u64,
}

impl Fraction {
    /// The default light client trust level, 1/3.
    pub const ONE_THIRD: Self = Self {
        numerator: 1,
        denominator: 3,
    };
}

fn should_batch_verify(vals: &ValidatorSet, commit: &Commit) -> bool {
    commit.signatures.len() >= BATCH_VERIFY_THRESHOLD
        && vals
            .peek_proposer()
            .is_some_and(|p| p.pub_key.supports_batch_verifier())
        && vals.all_keys_have_same_type()
}

type SigFilter = fn(&CommitSig) -> bool;

fn ignore_absent(c: &CommitSig) -> bool {
    c.block_id_flag == BLOCK_ID_FLAG_ABSENT
}
fn ignore_not_commit(c: &CommitSig) -> bool {
    c.block_id_flag != BLOCK_ID_FLAG_COMMIT
}
fn count_commit(c: &CommitSig) -> bool {
    c.block_id_flag == BLOCK_ID_FLAG_COMMIT
}
fn count_all(_: &CommitSig) -> bool {
    true
}

/// `VerifyCommit`: +2/3 of `vals` signed `block_id` at `height`; checks
/// every signature.
pub fn verify_commit(
    chain_id: &str,
    vals: &ValidatorSet,
    block_id: &BlockId,
    height: i64,
    commit: &Commit,
) -> Result<()> {
    verify_basic_vals_and_commit(vals, commit, height, block_id)?;
    let needed = vals.total_voting_power().wrapping_mul(2) / 3;
    if should_batch_verify(vals, commit) {
        return verify_commit_batch(
            chain_id,
            vals,
            commit,
            needed,
            ignore_absent,
            count_commit,
            true,
            true,
        );
    }
    verify_commit_single(
        chain_id,
        vals,
        commit,
        needed,
        ignore_absent,
        count_commit,
        true,
        true,
    )
}

/// `VerifyCommitLight`: +2/3 signed; stops once enough power is seen.
pub fn verify_commit_light(
    chain_id: &str,
    vals: &ValidatorSet,
    block_id: &BlockId,
    height: i64,
    commit: &Commit,
) -> Result<()> {
    verify_commit_light_internal(chain_id, vals, block_id, height, commit, false)
}

/// `VerifyCommitLightAllSignatures`.
pub fn verify_commit_light_all_signatures(
    chain_id: &str,
    vals: &ValidatorSet,
    block_id: &BlockId,
    height: i64,
    commit: &Commit,
) -> Result<()> {
    verify_commit_light_internal(chain_id, vals, block_id, height, commit, true)
}

fn verify_commit_light_internal(
    chain_id: &str,
    vals: &ValidatorSet,
    block_id: &BlockId,
    height: i64,
    commit: &Commit,
    count_all_signatures: bool,
) -> Result<()> {
    verify_basic_vals_and_commit(vals, commit, height, block_id)?;
    let needed = vals.total_voting_power().wrapping_mul(2) / 3;
    if should_batch_verify(vals, commit) {
        return verify_commit_batch(
            chain_id,
            vals,
            commit,
            needed,
            ignore_not_commit,
            count_all,
            count_all_signatures,
            true,
        );
    }
    verify_commit_single(
        chain_id,
        vals,
        commit,
        needed,
        ignore_not_commit,
        count_all,
        count_all_signatures,
        true,
    )
}

/// `VerifyCommitLightTrusting`: `trust_level` of `vals` (looked up by
/// address) signed the commit.
pub fn verify_commit_light_trusting(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &Commit,
    trust_level: Fraction,
) -> Result<()> {
    verify_commit_light_trusting_internal(chain_id, vals, commit, trust_level, false)
}

/// `VerifyCommitLightTrustingAllSignatures`.
pub fn verify_commit_light_trusting_all_signatures(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &Commit,
    trust_level: Fraction,
) -> Result<()> {
    verify_commit_light_trusting_internal(chain_id, vals, commit, trust_level, true)
}

#[allow(clippy::cast_possible_wrap)]
fn verify_commit_light_trusting_internal(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &Commit,
    trust_level: Fraction,
    count_all_signatures: bool,
) -> Result<()> {
    if trust_level.denominator == 0 {
        return Err(Error::msg("trustLevel has zero Denominator"));
    }
    let (product, overflow) = safe_mul(vals.total_voting_power(), trust_level.numerator as i64);
    if overflow {
        return Err(Error::msg(
            "int64 overflow while calculating voting power needed. please provide smaller trustLevel numerator",
        ));
    }
    let needed = product.wrapping_div(trust_level.denominator as i64);
    if should_batch_verify(vals, commit) {
        return verify_commit_batch(
            chain_id,
            vals,
            commit,
            needed,
            ignore_not_commit,
            count_all,
            count_all_signatures,
            false,
        );
    }
    verify_commit_single(
        chain_id,
        vals,
        commit,
        needed,
        ignore_not_commit,
        count_all,
        count_all_signatures,
        false,
    )
}

/// Validator lookup shared by both paths; `Ok(None)` means "skip".
fn lookup(
    vals: &ValidatorSet,
    commit_sig: &CommitSig,
    idx: usize,
    look_up_by_index: bool,
    seen: &mut Vec<(i32, usize)>,
) -> Result<Option<Validator>> {
    if look_up_by_index {
        let val = vals.validators[idx].clone();
        if val.address != commit_sig.validator_address {
            return Err(Error::msg(format!(
                "validator address mismatch at index {idx}: expected {}, got {}",
                hex_upper(&val.address),
                hex_upper(&commit_sig.validator_address)
            )));
        }
        return Ok(Some(val));
    }
    let Some((val_idx, val)) = vals.get_by_address(&commit_sig.validator_address) else {
        return Ok(None);
    };
    if let Some(&(_, first)) = seen.iter().find(|(v, _)| *v == val_idx) {
        return Err(Error::msg(format!(
            "double vote from {val} ({first} and {idx})"
        )));
    }
    seen.push((val_idx, idx));
    Ok(Some(val))
}

fn idx_i32(idx: usize) -> i32 {
    i32::try_from(idx).unwrap_or(i32::MAX)
}

#[allow(clippy::too_many_arguments)]
fn verify_commit_batch(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &Commit,
    needed: i64,
    ignore_sig: SigFilter,
    count_sig: SigFilter,
    count_all_signatures: bool,
    look_up_by_index: bool,
) -> Result<()> {
    let mut seen = Vec::new();
    let mut batch: Vec<(usize, Validator, Vec<u8>)> = Vec::new();
    let mut tallied = 0i64;
    for (idx, cs) in commit.signatures.iter().enumerate() {
        if ignore_sig(cs) {
            continue;
        }
        let Some(val) = lookup(vals, cs, idx, look_up_by_index, &mut seen)? else {
            continue;
        };
        let sign_bytes = commit.vote_sign_bytes(chain_id, idx_i32(idx))?;
        if cs.signature.len() != 64 {
            return Err(Error::msg("invalid signature"));
        }
        if count_sig(cs) {
            tallied = tallied.wrapping_add(val.voting_power);
        }
        batch.push((idx, val, sign_bytes));
        if !count_all_signatures && tallied > needed {
            break;
        }
    }
    if tallied <= needed {
        return Err(Error::NotEnoughVotingPowerSigned {
            got: tallied,
            needed,
        });
    }
    if let Some(idx) = first_invalid_signature(commit, &batch) {
        let cs = &commit.signatures[idx];
        return Err(Error::msg(format!(
            "wrong signature (#{idx}): {}",
            hex_upper(cs.to_string().as_bytes())
        )));
    }
    if batch.is_empty() {
        return Err(Error::msg(
            "BUG: batch verification failed with no invalid signatures",
        ));
    }
    Ok(())
}

/// The commit index of the first entry of `batch` (in batch order) whose
/// signature does not verify, checked in parallel for large batches
/// ([`crypto::parallel::first_failure`]: the sequential answer).
fn first_invalid_signature(
    commit: &Commit,
    batch: &[(usize, Validator, Vec<u8>)],
) -> Option<usize> {
    crypto::parallel::first_failure(batch, |(idx, val, sign_bytes)| {
        val.pub_key
            .verify_signature(sign_bytes, &commit.signatures[*idx].signature)
    })
    .map(|i| batch[i].0)
}

#[allow(clippy::too_many_arguments)]
fn verify_commit_single(
    chain_id: &str,
    vals: &ValidatorSet,
    commit: &Commit,
    needed: i64,
    ignore_sig: SigFilter,
    count_sig: SigFilter,
    count_all_signatures: bool,
    look_up_by_index: bool,
) -> Result<()> {
    let mut seen = Vec::new();
    let mut tallied = 0i64;
    // Go reuses `val` across iterations; error messages print the previous
    // one ("nil-Validator" before any was found).
    let mut last_val: Option<Validator> = None;
    for (idx, cs) in commit.signatures.iter().enumerate() {
        if ignore_sig(cs) {
            continue;
        }
        if cs.validate_basic().is_err() {
            return Err(Error::msg(format!(
                "invalid signatures from {} at index {idx}",
                last_val
                    .as_ref()
                    .map_or_else(|| "nil-Validator".to_owned(), ToString::to_string)
            )));
        }
        let found = lookup(vals, cs, idx, look_up_by_index, &mut seen);
        let val = match found {
            Ok(Some(v)) => v,
            Ok(None) => {
                last_val = None;
                continue;
            }
            Err(e) => return Err(e),
        };
        last_val = Some(val.clone());
        let sign_bytes = commit.vote_sign_bytes(chain_id, idx_i32(idx))?;
        if !val.pub_key.verify_signature(&sign_bytes, &cs.signature) {
            return Err(Error::msg(format!(
                "wrong signature (#{idx}): {}",
                hex_upper(&cs.signature)
            )));
        }
        if count_sig(cs) {
            tallied = tallied.wrapping_add(val.voting_power);
        }
        if !count_all_signatures && tallied > needed {
            return Ok(());
        }
    }
    if tallied <= needed {
        return Err(Error::NotEnoughVotingPowerSigned {
            got: tallied,
            needed,
        });
    }
    Ok(())
}

fn verify_basic_vals_and_commit(
    vals: &ValidatorSet,
    commit: &Commit,
    height: i64,
    block_id: &BlockId,
) -> Result<()> {
    if vals.size() != commit.signatures.len() {
        return Err(Error::InvalidCommitSignatures {
            expected: vals.size(),
            actual: commit.signatures.len(),
        });
    }
    if height != commit.height {
        return Err(Error::InvalidCommitHeight {
            expected: height,
            actual: commit.height,
        });
    }
    if !block_id.equals(&commit.block_id) {
        return Err(Error::msg(format!(
            "invalid commit -- wrong block ID: want {block_id}, got {}",
            commit.block_id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod parallel_tests {
    use super::*;
    use crate::commit::CommitSig;
    use crate::pubkey::PubKey;
    use crate::time::Time;

    /// A commit for `block_id` signed by `n` validators (seeded keys),
    /// with the signatures at `bad` (validator-set order) corrupted.
    fn signed_commit(n: u8, bad: &[usize]) -> (ValidatorSet, BlockId, Commit) {
        let keys: Vec<_> = (0..n)
            .map(|i| crypto::ed25519::private_key_from_seed(&[i + 1; 32]))
            .collect();
        let vals: Vec<Validator> = keys
            .iter()
            .map(|k| {
                let pk = crypto::ed25519::public_key_from_private_key(k).unwrap_or_default();
                Validator::new(PubKey::Ed25519(pk), 10)
            })
            .collect();
        let set = ValidatorSet::new(&vals).unwrap_or_default();
        let block_id = BlockId {
            hash: vec![7; 32],
            part_set_header: crate::part_set::PartSetHeader {
                total: 1,
                hash: vec![9; 32],
            },
        };
        let mut commit = Commit {
            height: 5,
            round: 0,
            block_id: block_id.clone(),
            signatures: set
                .validators
                .iter()
                .enumerate()
                .map(|(i, v)| CommitSig {
                    block_id_flag: BLOCK_ID_FLAG_COMMIT,
                    validator_address: v.address.clone(),
                    timestamp: Time::from_unix(1_700_000_000 + i64::try_from(i).unwrap_or(0), 0),
                    signature: vec![0; 64],
                })
                .collect(),
        };
        for i in 0..set.validators.len() {
            let addr = &set.validators[i].address;
            let key = keys
                .iter()
                .find(|k| {
                    crypto::ed25519::public_key_from_private_key(k)
                        .is_ok_and(|pk| PubKey::Ed25519(pk).address() == *addr)
                })
                .copied()
                .unwrap_or([0; 64]);
            let sb = commit
                .vote_sign_bytes("chain", idx_i32(i))
                .unwrap_or_default();
            let mut sig = crypto::ed25519::sign(&key, &sb).unwrap_or([0; 64]).to_vec();
            if bad.contains(&i) {
                sig[0] ^= 1;
            }
            commit.signatures[i].signature = sig;
        }
        (set, block_id, commit)
    }

    #[test]
    fn many_signatures_verify() {
        let (set, id, commit) = signed_commit(40, &[]);
        assert!(verify_commit("chain", &set, &id, 5, &commit).is_ok());
    }

    /// With several invalid signatures, the first one in validator-set
    /// order is reported, whatever order they are checked in.
    #[test]
    fn first_invalid_signature_is_reported() {
        let (set, id, commit) = signed_commit(40, &[30, 7]);
        let err = verify_commit("chain", &set, &id, 5, &commit)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.starts_with("wrong signature (#7)"), "{err}");
        let (set, id, commit) = signed_commit(40, &[39]);
        let err = verify_commit("chain", &set, &id, 5, &commit)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.starts_with("wrong signature (#39)"), "{err}");
    }
}
