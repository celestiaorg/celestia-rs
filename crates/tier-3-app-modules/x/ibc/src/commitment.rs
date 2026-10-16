//! ICS-23 commitment proofs (`core/23-commitment`): Merkle paths with the
//! counterparty prefix and chained `MerkleProof` verification.

use celestia_proto::ibc::core::commitment::v1::{MerklePath, MerklePrefix, MerkleProof};
use ics23::HostFunctionsManager;
use ics23::commitment_proof::Proof;
use prost::Message;
use sdk::{Result, SdkError};

use crate::errors::commitment as errs;

/// `GetSDKSpecs`: IAVL then Tendermint.
#[must_use]
pub fn sdk_specs() -> Vec<ics23::ProofSpec> {
    vec![ics23::iavl_spec(), ics23::tendermint_spec()]
}

/// `ApplyPrefix`: `[prefix, path...]`.
pub fn apply_prefix(prefix: Option<&MerklePrefix>, path: &[String]) -> Result<MerklePath> {
    let p = prefix.map(|p| p.key_prefix.as_slice()).unwrap_or_default();
    if p.is_empty() {
        return Err(errs::invalid_prefix("prefix can't be empty"));
    }
    // `string(prefix.Bytes())`: the bytes as they are (Go form).
    let mut key_path = vec![gostr::from_go_bytes(p).into_owned()];
    key_path.extend(path.iter().cloned());
    Ok(MerklePath { key_path })
}

/// Decodes a proof (`cdc.Unmarshal(proof, &merkleProof)`).
pub fn decode_proof(proof: &[u8]) -> Result<MerkleProof> {
    MerkleProof::decode(proof).map_err(|_| {
        errs::invalid_proof("failed to unmarshal proof into ICS 23 commitment merkle proof")
    })
}

fn calculate(p: &ics23::CommitmentProof) -> core::result::Result<Vec<u8>, String> {
    match &p.proof {
        Some(Proof::Exist(ex)) => {
            ics23::calculate_existence_root::<HostFunctionsManager>(ex).map_err(|e| e.to_string())
        }
        Some(Proof::Nonexist(non)) => {
            if let Some(l) = &non.left {
                ics23::calculate_existence_root::<HostFunctionsManager>(l)
                    .map_err(|e| e.to_string())
            } else if let Some(r) = &non.right {
                ics23::calculate_existence_root::<HostFunctionsManager>(r)
                    .map_err(|e| e.to_string())
            } else {
                Err("Nonexistence proof has empty Left and Right proof".into())
            }
        }
        _ => Err("unrecognized proof type".into()),
    }
}

fn validate_args(proof: &MerkleProof, specs: &[ics23::ProofSpec], root: &[u8]) -> Result<()> {
    if proof.proofs.is_empty() {
        return Err(errs::invalid_merkle_proof("proof cannot be empty"));
    }
    if root.is_empty() {
        return Err(errs::invalid_merkle_proof("root cannot be empty"));
    }
    if specs.len() != proof.proofs.len() {
        return Err(errs::invalid_merkle_proof(format!(
            "length of specs: {} not equal to length of proof: {}",
            specs.len(),
            proof.proofs.len()
        )));
    }
    Ok(())
}

/// `%s` of a `MerklePath` value (its `String` has a pointer receiver):
/// `{[a b]}`.
fn path_text(keys: &MerklePath) -> String {
    format!("{{[{}]}}", keys.key_path.join(" "))
}

/// `%T` of a `CommitmentProof.Proof` (the oneof wrapper type).
fn proof_type(p: Option<&Proof>) -> &'static str {
    match p {
        Some(Proof::Exist(_)) => "*ics23.CommitmentProof_Exist",
        Some(Proof::Nonexist(_)) => "*ics23.CommitmentProof_Nonexist",
        Some(Proof::Batch(_)) => "*ics23.CommitmentProof_Batch",
        Some(Proof::Compressed(_)) => "*ics23.CommitmentProof_Compressed",
        None => "<nil>",
    }
}

fn verify_chained_membership(
    root: &[u8],
    specs: &[ics23::ProofSpec],
    proofs: &[ics23::CommitmentProof],
    keys: &MerklePath,
    value: Vec<u8>,
    index: usize,
) -> Result<()> {
    let mut value = value;
    let mut subroot = value.clone();
    for i in index..proofs.len() {
        match &proofs[i].proof {
            Some(Proof::Exist(_)) => {
                subroot = calculate(&proofs[i]).map_err(|e| {
                    errs::invalid_proof(format!(
                        "could not calculate proof root at index {i}, merkle tree may be empty. {e}"
                    ))
                })?;
                let key = keys
                    .key_path
                    .get(keys.key_path.len().wrapping_sub(1 + i))
                    .ok_or_else(|| errs::invalid_proof("could not retrieve key bytes"))?;
                if !ics23::verify_membership::<HostFunctionsManager>(
                    &proofs[i],
                    &specs[i],
                    &subroot,
                    &gostr::to_go_bytes(key),
                    &value,
                ) {
                    return Err(errs::invalid_proof(format!(
                        "chained membership proof failed to verify membership of value: {} in subroot {} at index {i}. Please ensure the path and value are both correct.",
                        hex::encode_upper(&value),
                        hex::encode_upper(&subroot)
                    )));
                }
                value.clone_from(&subroot);
            }
            Some(Proof::Nonexist(_)) => {
                return Err(errs::invalid_proof(format!(
                    "chained membership proof contains nonexistence proof at index {i}. If this is unexpected, please ensure that proof was queried from a height that contained the value in store and was queried with the correct key. The key used: {}",
                    path_text(keys)
                )));
            }
            other => {
                return Err(errs::invalid_proof(format!(
                    "expected proof type: *ics23.CommitmentProof_Exist, got: {}",
                    proof_type(other.as_ref())
                )));
            }
        }
    }
    if root != subroot.as_slice() {
        return Err(errs::invalid_proof(format!(
            "proof did not commit to expected root: {}, got: {}. Please ensure proof was submitted with correct proofHeight and to the correct chain.",
            hex::encode_upper(root),
            hex::encode_upper(&subroot)
        )));
    }
    Ok(())
}

/// `MerkleProof.VerifyMembership`.
pub fn verify_membership(
    proof: &MerkleProof,
    specs: &[ics23::ProofSpec],
    root: &[u8],
    path: &MerklePath,
    value: &[u8],
) -> Result<()> {
    validate_args(proof, specs, root)?;
    if path.key_path.len() != specs.len() {
        return Err(errs::invalid_proof(format!(
            "path length {} not same as proof {}",
            path.key_path.len(),
            specs.len()
        )));
    }
    if value.is_empty() {
        return Err(errs::invalid_proof("empty value in membership proof"));
    }
    verify_chained_membership(root, specs, &proof.proofs, path, value.to_vec(), 0)
}

/// `MerkleProof.VerifyNonMembership`.
pub fn verify_non_membership(
    proof: &MerkleProof,
    specs: &[ics23::ProofSpec],
    root: &[u8],
    path: &MerklePath,
) -> Result<()> {
    validate_args(proof, specs, root)?;
    if path.key_path.len() != specs.len() {
        return Err(errs::invalid_proof(format!(
            "path length {} not same as proof {}",
            path.key_path.len(),
            specs.len()
        )));
    }
    match &proof.proofs[0].proof {
        Some(Proof::Nonexist(_)) => {
            let subroot = calculate(&proof.proofs[0]).map_err(|e| {
                errs::invalid_proof(format!(
                    "could not calculate root for proof index 0, merkle tree is likely empty. {e}"
                ))
            })?;
            let key = path
                .key_path
                .last()
                .ok_or_else(|| errs::invalid_proof("could not retrieve key bytes"))?;
            if !ics23::verify_non_membership::<HostFunctionsManager>(
                &proof.proofs[0],
                &specs[0],
                &subroot,
                &gostr::to_go_bytes(key),
            ) {
                return Err(errs::invalid_proof(format!(
                    "could not verify absence of key {key}. Please ensure that the path is correct."
                )));
            }
            verify_chained_membership(root, specs, &proof.proofs, path, subroot, 1)
        }
        Some(Proof::Exist(_)) => Err(errs::invalid_proof(
            "got ExistenceProof in VerifyNonMembership. If this is unexpected, please ensure that proof was queried with the correct key.",
        )),
        other => Err(errs::invalid_proof(format!(
            "expected proof type: *ics23.CommitmentProof_Exist, got: {}",
            proof_type(other.as_ref())
        ))),
    }
}

/// Converts an error to the unregistered kind (for callers that wrap a
/// non-ibc error).
#[must_use]
pub fn unregistered(e: &SdkError) -> SdkError {
    crate::errors::unregistered(e.log.clone())
}
