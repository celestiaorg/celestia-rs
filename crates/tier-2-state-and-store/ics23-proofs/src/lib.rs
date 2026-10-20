//! Cosmos SDK ICS-23 proof operation primitives.
//!
//! See `docs/specs/m2/ics23-proofs.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;

use prost::Message;

pub use error::{Error, Result};
pub use iavl::Hash;

use ics23::commitment_proof::Proof as RawCommitmentProof;
use ics23::{
    BatchEntry, BatchProof, CommitmentProof as RawProof, ExistenceProof, HostFunctionsManager,
    NonExistenceProof,
};

/// Cosmos SDK IAVL commitment proof operation type.
pub const PROOF_OP_IAVL_COMMITMENT: &str = "ics23:iavl";
/// Cosmos SDK simple Merkle commitment proof operation type.
pub const PROOF_OP_SIMPLE_MERKLE_COMMITMENT: &str = "ics23:simple";
/// Cosmos SDK sparse Merkle tree commitment proof operation type.
pub const PROOF_OP_SMT_COMMITMENT: &str = "ics23:smt";

/// `CometBFT` `tendermint.crypto.ProofOp` protobuf-compatible shape.
#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct ProofOp {
    /// Operation type. This corresponds to the protobuf field named `type`.
    #[prost(string, tag = "1")]
    pub field_type: String,
    /// Operation key.
    #[prost(bytes = "vec", tag = "2")]
    pub key: Vec<u8>,
    /// Operation payload.
    #[prost(bytes = "vec", tag = "3")]
    pub data: Vec<u8>,
}

/// `CometBFT` `tendermint.crypto.ProofOps` protobuf-compatible shape.
#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct ProofOps {
    /// Proof operations in execution order.
    #[prost(message, repeated, tag = "1")]
    pub ops: Vec<ProofOp>,
}

/// Opaque ICS-23 commitment proof wrapper.
#[derive(Clone)]
pub struct CommitmentProof {
    raw: RawProof,
    bytes: Vec<u8>,
    kind: CommitmentProofKind,
    key: Vec<u8>,
    value: Option<Vec<u8>>,
}

impl CommitmentProof {
    /// Decode ICS-23 `CommitmentProof` protobuf bytes.
    pub fn from_proto_bytes(bytes: Vec<u8>) -> Result<Self> {
        let raw = RawProof::decode(bytes.as_slice())
            .map_err(|_| Error::MalformedProto("commitment proof decode failed"))?;
        let (kind, key, value) = proof_shape(&raw)?;
        Ok(Self {
            raw,
            bytes,
            kind,
            key,
            value,
        })
    }

    /// Wrap proof bytes from the local IAVL crate.
    pub fn from_iavl_proof(proof: &iavl::Proof) -> Result<Self> {
        let [op] = proof.ops() else {
            return Err(Error::InvalidProof(
                "expected exactly one IAVL proof operation",
            ));
        };
        Self::from_proto_bytes(op.data().to_vec())
    }

    /// Return the canonical protobuf bytes for this proof.
    #[must_use]
    pub fn as_proto_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the high-level proof kind.
    #[must_use]
    pub fn kind(&self) -> CommitmentProofKind {
        self.kind
    }

    /// Return the key this proof speaks about, when the proof kind has one.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Return the proven value for membership proofs.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Calculate the root represented by this proof.
    pub fn calculate_root(&self, spec: ProofSpec) -> Result<Hash> {
        let _ = spec.raw();
        calculate_raw_root(&self.raw)
    }

    /// Verify membership against an expected root, key, and value.
    pub fn verify_membership(
        &self,
        spec: ProofSpec,
        root: Hash,
        key: &[u8],
        value: &[u8],
    ) -> Result<()> {
        let root = root.to_vec();
        if ics23::verify_membership::<HostFunctionsManager>(
            &self.raw,
            &spec.raw(),
            &root,
            key,
            value,
        ) {
            Ok(())
        } else {
            Err(Error::InvalidProof("membership verification failed"))
        }
    }

    /// Verify non-membership against an expected root and key.
    pub fn verify_non_membership(&self, spec: ProofSpec, root: Hash, key: &[u8]) -> Result<()> {
        let root = root.to_vec();
        if ics23::verify_non_membership::<HostFunctionsManager>(&self.raw, &spec.raw(), &root, key)
        {
            Ok(())
        } else {
            Err(Error::InvalidProof("non-membership verification failed"))
        }
    }
}

/// High-level ICS-23 commitment proof variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitmentProofKind {
    /// Existence proof.
    Existence,
    /// Non-existence proof.
    NonExistence,
    /// Batch proof.
    Batch,
    /// Compressed batch proof.
    Compressed,
}

/// Supported ICS-23 proof specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofSpec {
    /// Cosmos IAVL proof spec.
    Iavl,
    /// `CometBFT` simple Merkle proof spec.
    Tendermint,
    /// Sparse Merkle tree proof spec.
    Smt,
}

impl ProofSpec {
    fn raw(self) -> ics23::ProofSpec {
        match self {
            Self::Iavl => ics23::iavl_spec(),
            Self::Tendermint => ics23::tendermint_spec(),
            Self::Smt => ics23::smt_spec(),
        }
    }
}

/// Opaque Cosmos SDK commitment proof operation.
#[derive(Clone)]
pub struct CommitmentOp {
    spec: ProofSpec,
    key: Vec<u8>,
    proof: CommitmentProof,
}

impl CommitmentOp {
    /// Create an IAVL commitment operation.
    #[must_use]
    pub fn new_iavl(key: Vec<u8>, proof: CommitmentProof) -> Self {
        Self {
            spec: ProofSpec::Iavl,
            key,
            proof,
        }
    }

    /// Create a simple Merkle commitment operation.
    #[must_use]
    pub fn new_simple_merkle(key: Vec<u8>, proof: CommitmentProof) -> Self {
        Self {
            spec: ProofSpec::Tendermint,
            key,
            proof,
        }
    }

    /// Create an SMT commitment operation.
    #[must_use]
    pub fn new_smt(key: Vec<u8>, proof: CommitmentProof) -> Self {
        Self {
            spec: ProofSpec::Smt,
            key,
            proof,
        }
    }

    /// Decode a commitment operation from a `CometBFT` proof op.
    pub fn from_proof_op(op: ProofOp) -> Result<Self> {
        let spec = proof_op_spec(&op.field_type)?;
        if op.key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let proof = CommitmentProof::from_proto_bytes(op.data)?;
        Ok(Self {
            spec,
            key: op.key,
            proof,
        })
    }

    /// Wrap proof bytes from the local IAVL crate as an SDK commitment op.
    pub fn from_iavl_proof(proof: &iavl::Proof) -> Result<Self> {
        let [op] = proof.ops() else {
            return Err(Error::InvalidProof(
                "expected exactly one IAVL proof operation",
            ));
        };
        if op.key().is_empty() {
            return Err(Error::EmptyKey);
        }
        Ok(Self::new_iavl(
            op.key().to_vec(),
            CommitmentProof::from_iavl_proof(proof)?,
        ))
    }

    /// Return the SDK proof operation type.
    #[must_use]
    pub fn field_type(&self) -> &'static str {
        proof_op_type(self.spec)
    }

    /// Return the ICS-23 proof spec for this operation type.
    #[must_use]
    pub fn spec(&self) -> ProofSpec {
        self.spec
    }

    /// Return operation key bytes.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Return the wrapped commitment proof.
    #[must_use]
    pub fn proof(&self) -> &CommitmentProof {
        &self.proof
    }

    /// Encode this operation as a `CometBFT` proof op.
    #[must_use]
    pub fn to_proof_op(&self) -> ProofOp {
        ProofOp {
            field_type: self.field_type().to_owned(),
            key: self.key.clone(),
            data: self.proof.as_proto_bytes().to_vec(),
        }
    }

    /// Run the operation and return the calculated root.
    pub fn run(&self, args: &[&[u8]]) -> Result<Vec<Hash>> {
        if self.key.is_empty() {
            return Err(Error::EmptyKey);
        }
        let root = self.proof.calculate_root(self.spec())?;
        match args {
            [] => self
                .proof
                .verify_non_membership(self.spec(), root, &self.key)?,
            [value] => self
                .proof
                .verify_membership(self.spec(), root, &self.key, value)?,
            _ => return Err(Error::InvalidProof("invalid commitment op argument count")),
        }
        Ok(vec![root])
    }
}

fn proof_shape(raw: &RawProof) -> Result<(CommitmentProofKind, Vec<u8>, Option<Vec<u8>>)> {
    match raw.proof.as_ref() {
        Some(RawCommitmentProof::Exist(existence)) => Ok((
            CommitmentProofKind::Existence,
            existence.key.clone(),
            Some(existence.value.clone()),
        )),
        Some(RawCommitmentProof::Nonexist(nonexistence)) => Ok((
            CommitmentProofKind::NonExistence,
            nonexistence.key.clone(),
            None,
        )),
        Some(RawCommitmentProof::Batch(_)) => Ok((CommitmentProofKind::Batch, Vec::new(), None)),
        Some(RawCommitmentProof::Compressed(_)) => {
            Ok((CommitmentProofKind::Compressed, Vec::new(), None))
        }
        None => Err(Error::MalformedProto("missing commitment proof variant")),
    }
}

fn proof_op_spec(field_type: &str) -> Result<ProofSpec> {
    match field_type {
        PROOF_OP_IAVL_COMMITMENT => Ok(ProofSpec::Iavl),
        PROOF_OP_SIMPLE_MERKLE_COMMITMENT => Ok(ProofSpec::Tendermint),
        PROOF_OP_SMT_COMMITMENT => Ok(ProofSpec::Smt),
        other => Err(Error::InvalidProofOpType(other.to_owned())),
    }
}

fn proof_op_type(spec: ProofSpec) -> &'static str {
    match spec {
        ProofSpec::Iavl => PROOF_OP_IAVL_COMMITMENT,
        ProofSpec::Tendermint => PROOF_OP_SIMPLE_MERKLE_COMMITMENT,
        ProofSpec::Smt => PROOF_OP_SMT_COMMITMENT,
    }
}

fn calculate_raw_root(raw: &RawProof) -> Result<Hash> {
    match raw.proof.as_ref() {
        Some(RawCommitmentProof::Exist(existence)) => calculate_existence_root(existence),
        Some(RawCommitmentProof::Nonexist(nonexistence)) => {
            calculate_nonexistence_root(nonexistence)
        }
        Some(RawCommitmentProof::Batch(batch)) => calculate_batch_root(batch),
        Some(RawCommitmentProof::Compressed(_)) => {
            let decompressed = ics23::decompress(raw)
                .map_err(|_| Error::InvalidProof("compressed proof decompression failed"))?;
            calculate_raw_root(&decompressed)
        }
        None => Err(Error::MalformedProto("missing commitment proof variant")),
    }
}

fn calculate_batch_root(batch: &BatchProof) -> Result<Hash> {
    let mut root = None;
    for entry in &batch.entries {
        let next = calculate_batch_entry_root(entry)?;
        if root.is_some_and(|existing| existing != next) {
            return Err(Error::InvalidProof("batch proof roots mismatch"));
        }
        root = Some(next);
    }
    root.ok_or(Error::InvalidProof("empty batch proof"))
}

fn calculate_batch_entry_root(entry: &BatchEntry) -> Result<Hash> {
    match entry.proof.as_ref() {
        Some(ics23::batch_entry::Proof::Exist(existence)) => calculate_existence_root(existence),
        Some(ics23::batch_entry::Proof::Nonexist(nonexistence)) => {
            calculate_nonexistence_root(nonexistence)
        }
        None => Err(Error::InvalidProof("empty batch proof entry")),
    }
}

fn calculate_nonexistence_root(nonexistence: &NonExistenceProof) -> Result<Hash> {
    let mut root = None;
    for existence in [&nonexistence.left, &nonexistence.right]
        .into_iter()
        .flatten()
    {
        let next = calculate_existence_root(existence)?;
        if root.is_some_and(|existing| existing != next) {
            return Err(Error::InvalidProof("non-existence neighbor roots mismatch"));
        }
        root = Some(next);
    }
    root.ok_or(Error::InvalidProof("missing non-existence neighbors"))
}

fn calculate_existence_root(existence: &ExistenceProof) -> Result<Hash> {
    let root = ics23::calculate_existence_root::<HostFunctionsManager>(existence)
        .map_err(|_| Error::InvalidProof("existence root calculation failed"))?;
    root.try_into()
        .map_err(|_| Error::InvalidProof("calculated root must be 32 bytes"))
}
