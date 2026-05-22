//! Cosmos SDK commit multi-store primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m2/multistore.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

use encoding::varint;
use ics23::commitment_proof::Proof as RawCommitmentProof;
use ics23::{ExistenceProof, HashOp, InnerOp, LeafOp, LengthOp};
use prost::Message;

pub use iavl::{Hash, Version};

/// IAVL-backed key/value store mounted in a multi-store.
pub type KvStore = kvstore::Store;

/// Commitment information for one committed store version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitId {
    /// Committed store version.
    pub version: Version,
    /// Committed root hash.
    pub hash: Hash,
}

impl CommitId {
    /// Return the Cosmos SDK zero commit ID.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            version: 0,
            hash: hash::sha256(&[]),
        }
    }

    /// Return true when this is the zero commit ID.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::zero()
    }
}

/// Store-specific commit information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    /// Mounted store name.
    pub name: String,
    /// Commit ID for this store.
    pub commit_id: CommitId,
}

/// Multi-store commit information for one app height.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitInfo {
    /// Multi-store commit version.
    pub version: Version,
    /// Store commit infos sorted by store name.
    pub store_infos: Vec<StoreInfo>,
    /// Optional commit timestamp from the Cosmos SDK protobuf shape.
    pub timestamp: Option<prost_types::Timestamp>,
}

impl CommitInfo {
    /// Construct validated commit info and sort stores by name.
    pub fn new(version: Version, store_infos: Vec<StoreInfo>) -> Result<Self> {
        if version <= 0 {
            return Err(Error::InvalidVersion { version });
        }
        let store_infos = sorted_committed_store_infos(store_infos)?;
        Ok(Self {
            version,
            store_infos,
            timestamp: None,
        })
    }

    /// Return the Cosmos SDK app hash for this commit info.
    pub fn hash(&self) -> Result<Hash> {
        commit_info_hash(&self.store_infos)
    }

    /// Return this commit info as a commit ID.
    pub fn commit_id(&self) -> Result<CommitId> {
        Ok(CommitId {
            version: self.version,
            hash: self.hash()?,
        })
    }

    /// Return the SDK simple Merkle proof op for `store_name`.
    pub fn proof_op(&self, store_name: &str) -> Result<ics23_proofs::ProofOp> {
        proof_op_from_store_infos(&self.store_infos, store_name)
    }

    /// Return store info by name.
    #[must_use]
    pub fn store_info(&self, store_name: &str) -> Option<&StoreInfo> {
        self.store_infos
            .iter()
            .find(|store_info| store_info.name == store_name)
    }

    /// Encode this value as Cosmos SDK `CommitInfo` protobuf bytes.
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>> {
        if self.version <= 0 {
            return Err(Error::InvalidVersion {
                version: self.version,
            });
        }
        let store_infos = sorted_committed_store_infos(self.store_infos.clone())?;
        Ok(RawCommitInfo {
            version: self.version,
            store_infos: store_infos.into_iter().map(RawStoreInfo::from).collect(),
            timestamp: self.timestamp,
        }
        .encode_to_vec())
    }

    /// Decode Cosmos SDK `CommitInfo` protobuf bytes.
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self> {
        let raw = RawCommitInfo::decode(bytes)
            .map_err(|_| Error::MalformedProto("commit info decode failed"))?;
        if raw.version <= 0 {
            return Err(Error::InvalidVersion {
                version: raw.version,
            });
        }
        let mut store_infos = Vec::with_capacity(raw.store_infos.len());
        for raw_store_info in raw.store_infos {
            store_infos.push(StoreInfo::try_from(raw_store_info)?);
        }
        let mut commit_info = Self::new(raw.version, store_infos)?;
        commit_info.timestamp = raw.timestamp;
        Ok(commit_info)
    }
}

/// Mounted store kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// IAVL-backed persistent key/value store.
    Iavl,
}

/// In-memory Cosmos SDK commit multi-store.
#[derive(Debug, Clone)]
pub struct Store {
    initial_version: Version,
    version: Version,
    stores: BTreeMap<String, KvStore>,
    last_commit_info: Option<CommitInfo>,
    history: BTreeMap<Version, CommitInfo>,
}

impl Store {
    /// Create an empty multi-store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            initial_version: 1,
            version: 0,
            stores: BTreeMap::new(),
            last_commit_info: None,
            history: BTreeMap::new(),
        }
    }

    /// Create an empty multi-store with a custom first commit version.
    pub fn with_initial_version(initial_version: Version) -> Result<Self> {
        if initial_version <= 0 {
            return Err(Error::InvalidVersion {
                version: initial_version,
            });
        }
        Ok(Self {
            initial_version,
            ..Self::new()
        })
    }

    /// Mount a new store under `name`.
    pub fn mount_store(&mut self, name: impl Into<String>, kind: StoreKind) -> Result<()> {
        let name = name.into();
        validate_store_name(&name)?;
        if self.stores.contains_key(&name) {
            return Err(Error::DuplicateStoreName(name));
        }
        match kind {
            StoreKind::Iavl => {
                self.stores.insert(name, KvStore::new());
            }
        }
        Ok(())
    }

    /// Return true when a store is mounted under `name`.
    #[must_use]
    pub fn has_store(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// Return mounted store names in lexicographic order.
    #[must_use]
    pub fn store_names(&self) -> Vec<&str> {
        self.stores.keys().map(String::as_str).collect()
    }

    /// Return an immutable mounted store by name.
    pub fn get_store(&self, name: &str) -> Result<&KvStore> {
        self.stores
            .get(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return a mutable mounted store by name.
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore> {
        self.stores
            .get_mut(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return the latest committed multi-store version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Return the latest multi-store commit ID.
    pub fn last_commit_id(&self) -> Result<CommitId> {
        self.last_commit_info
            .as_ref()
            .map_or_else(|| Ok(CommitId::zero()), CommitInfo::commit_id)
    }

    /// Return the latest multi-store commit info.
    #[must_use]
    pub fn last_commit_info(&self) -> Option<&CommitInfo> {
        self.last_commit_info.as_ref()
    }

    /// Return the app hash for current mounted store working roots.
    pub fn working_hash(&self) -> Result<Hash> {
        commit_info_hash(&self.working_store_infos())
    }

    /// Commit every mounted store and return the multi-store commit ID.
    pub fn commit(&mut self) -> Result<CommitId> {
        let next_version = if self.version == 0 && self.initial_version > 1 {
            self.initial_version
        } else {
            self.version.checked_add(1).ok_or(Error::InvalidVersion {
                version: self.version,
            })?
        };
        if next_version <= 0 {
            return Err(Error::InvalidVersion {
                version: next_version,
            });
        }

        let mut store_infos = Vec::with_capacity(self.stores.len());
        for (name, store) in &mut self.stores {
            let commit = store.save_version()?;
            store_infos.push(StoreInfo {
                name: name.clone(),
                commit_id: CommitId {
                    version: next_version,
                    hash: commit.root_hash,
                },
            });
        }

        let commit_info = CommitInfo::new(next_version, store_infos)?;
        let commit_id = commit_info.commit_id()?;
        self.version = next_version;
        self.history.insert(next_version, commit_info.clone());
        self.last_commit_info = Some(commit_info);
        Ok(commit_id)
    }

    /// Return historical commit info by version.
    pub fn get_commit_info(&self, version: Version) -> Result<&CommitInfo> {
        self.history
            .get(&version)
            .ok_or(Error::VersionNotFound { version })
    }

    /// Create a clone-backed cache multi-store.
    #[must_use]
    pub fn cache_multi_store(&self) -> CacheMultiStore {
        CacheMultiStore {
            stores: self.stores.clone(),
        }
    }

    /// Apply a cache multi-store to this parent store.
    pub fn apply_cache(&mut self, cache: CacheMultiStore) -> Result<()> {
        cache.write(self)
    }

    fn working_store_infos(&self) -> Vec<StoreInfo> {
        self.stores
            .iter()
            .map(|(name, store)| StoreInfo {
                name: name.clone(),
                commit_id: CommitId {
                    version: store.version(),
                    hash: store.root_hash(),
                },
            })
            .collect()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// Clone-backed write-back cache multi-store.
#[derive(Debug, Clone)]
pub struct CacheMultiStore {
    stores: BTreeMap<String, KvStore>,
}

impl CacheMultiStore {
    /// Return true when a store is mounted under `name`.
    #[must_use]
    pub fn has_store(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// Return cached store names in lexicographic order.
    #[must_use]
    pub fn store_names(&self) -> Vec<&str> {
        self.stores.keys().map(String::as_str).collect()
    }

    /// Return an immutable cached store by name.
    pub fn get_store(&self, name: &str) -> Result<&KvStore> {
        self.stores
            .get(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Return a mutable cached store by name.
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore> {
        self.stores
            .get_mut(name)
            .ok_or_else(|| Error::UnknownStore(name.to_owned()))
    }

    /// Flush cached store states into `parent`.
    pub fn write(self, parent: &mut Store) -> Result<()> {
        for (name, store) in self.stores {
            let parent_store = parent
                .stores
                .get_mut(&name)
                .ok_or_else(|| Error::UnknownStore(name.clone()))?;
            *parent_store = store;
        }
        Ok(())
    }

    /// Drop this cache without mutating the parent.
    pub fn discard(self) {}
}

/// Errors returned by multi-store helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A store name was empty.
    EmptyStoreName,
    /// A store name was mounted or decoded more than once.
    DuplicateStoreName(String),
    /// A store name is not mounted or not present in commit info.
    UnknownStore(String),
    /// A version was not positive where a committed version was required.
    InvalidVersion {
        /// Invalid version.
        version: Version,
    },
    /// Historical commit info was not found.
    VersionNotFound {
        /// Requested version.
        version: Version,
    },
    /// Commit info was structurally invalid.
    InvalidCommitInfo(&'static str),
    /// Protobuf bytes could not be decoded.
    MalformedProto(&'static str),
    /// Underlying key/value store error.
    Kv(kvstore::Error),
    /// Underlying IAVL error.
    Iavl(iavl::Error),
    /// Underlying simple Merkle error.
    Merkle(merkle::Error),
    /// Underlying ICS-23 proof error.
    Proof(ics23_proofs::Error),
}

impl From<kvstore::Error> for Error {
    fn from(value: kvstore::Error) -> Self {
        Self::Kv(value)
    }
}

impl From<iavl::Error> for Error {
    fn from(value: iavl::Error) -> Self {
        Self::Iavl(value)
    }
}

impl From<merkle::Error> for Error {
    fn from(value: merkle::Error) -> Self {
        Self::Merkle(value)
    }
}

impl From<ics23_proofs::Error> for Error {
    fn from(value: ics23_proofs::Error) -> Self {
        Self::Proof(value)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyStoreName => f.write_str("empty store name"),
            Self::DuplicateStoreName(name) => write!(f, "duplicate store name: {name}"),
            Self::UnknownStore(name) => write!(f, "unknown store: {name}"),
            Self::InvalidVersion { version } => write!(f, "invalid version: {version}"),
            Self::VersionNotFound { version } => write!(f, "version not found: {version}"),
            Self::InvalidCommitInfo(message) => write!(f, "invalid commit info: {message}"),
            Self::MalformedProto(message) => write!(f, "malformed protobuf: {message}"),
            Self::Kv(err) => write!(f, "kvstore: {err}"),
            Self::Iavl(err) => write!(f, "iavl: {err}"),
            Self::Merkle(err) => write!(f, "merkle: {err}"),
            Self::Proof(err) => write!(f, "proof: {err}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result alias for multi-store helpers.
pub type Result<T> = core::result::Result<T, Error>;

/// Return the Cosmos SDK app hash for store commit infos.
pub fn commit_info_hash(store_infos: &[StoreInfo]) -> Result<Hash> {
    let store_infos = sorted_store_infos(store_infos.to_vec())?;
    if store_infos.is_empty() {
        return Ok(hash::sha256(&[]));
    }
    let leaves = simple_map_leaves(&store_infos);
    Ok(merkle::root_from_byte_slices(&leaves))
}

/// Return the SDK simple Merkle proof op for `store_name`.
pub fn proof_op_from_store_infos(
    store_infos: &[StoreInfo],
    store_name: &str,
) -> Result<ics23_proofs::ProofOp> {
    validate_store_name(store_name)?;
    let store_infos = sorted_store_infos(store_infos.to_vec())?;
    let leaves = simple_map_leaves(&store_infos);
    let (_, proofs) = merkle::proofs_from_byte_slices(&leaves);
    let Some((index, store_info)) = store_infos
        .iter()
        .enumerate()
        .find(|(_, store_info)| store_info.name == store_name)
    else {
        return Err(Error::UnknownStore(store_name.to_owned()));
    };
    let proof = proofs
        .get(index)
        .ok_or(Error::InvalidCommitInfo("missing simple Merkle proof"))?;
    let commitment_proof = ics23::CommitmentProof {
        proof: Some(RawCommitmentProof::Exist(ExistenceProof {
            key: store_info.name.as_bytes().to_vec(),
            value: store_info.commit_id.hash.to_vec(),
            leaf: Some(LeafOp {
                hash: HashOp::Sha256.into(),
                prehash_key: HashOp::NoHash.into(),
                prehash_value: HashOp::Sha256.into(),
                length: LengthOp::VarProto.into(),
                prefix: vec![0],
            }),
            path: inner_ops_from_simple_proof(proof)?,
        })),
    };
    Ok(ics23_proofs::ProofOp {
        field_type: ics23_proofs::PROOF_OP_SIMPLE_MERKLE_COMMITMENT.to_owned(),
        key: store_info.name.as_bytes().to_vec(),
        data: commitment_proof.encode_to_vec(),
    })
}

fn sorted_store_infos(mut store_infos: Vec<StoreInfo>) -> Result<Vec<StoreInfo>> {
    let mut names = BTreeSet::new();
    for store_info in &store_infos {
        validate_store_name(&store_info.name)?;
        if !names.insert(store_info.name.clone()) {
            return Err(Error::DuplicateStoreName(store_info.name.clone()));
        }
    }
    store_infos.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(store_infos)
}

fn sorted_committed_store_infos(store_infos: Vec<StoreInfo>) -> Result<Vec<StoreInfo>> {
    let store_infos = sorted_store_infos(store_infos)?;
    for store_info in &store_infos {
        if store_info.commit_id.version <= 0 {
            return Err(Error::InvalidVersion {
                version: store_info.commit_id.version,
            });
        }
    }
    Ok(store_infos)
}

fn validate_store_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::EmptyStoreName);
    }
    Ok(())
}

fn simple_map_leaves(store_infos: &[StoreInfo]) -> Vec<Vec<u8>> {
    store_infos
        .iter()
        .map(|store_info| {
            kv_pair_bytes(
                store_info.name.as_bytes(),
                &hash::sha256(&store_info.commit_id.hash),
            )
        })
        .collect()
}

fn kv_pair_bytes(key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        varint::encoded_uvarint_len(key.len() as u64)
            + key.len()
            + varint::encoded_uvarint_len(value.len() as u64)
            + value.len(),
    );
    bytes.extend(varint::encode_uvarint(key.len() as u64));
    bytes.extend(key);
    bytes.extend(varint::encode_uvarint(value.len() as u64));
    bytes.extend(value);
    bytes
}

fn inner_ops_from_simple_proof(proof: &merkle::Proof) -> Result<Vec<InnerOp>> {
    proof.validate_basic()?;
    let path = simple_proof_path(proof.index, proof.total);
    if path.len() != proof.aunts.len() {
        return Err(Error::Merkle(merkle::Error::UnexpectedAuntCount));
    }
    Ok(proof
        .aunts
        .iter()
        .zip(path)
        .map(|(aunt, aunt_is_right)| {
            if aunt_is_right {
                InnerOp {
                    hash: HashOp::Sha256.into(),
                    prefix: vec![1],
                    suffix: aunt.to_vec(),
                }
            } else {
                let mut prefix = Vec::with_capacity(1 + aunt.len());
                prefix.push(1);
                prefix.extend(aunt);
                InnerOp {
                    hash: HashOp::Sha256.into(),
                    prefix,
                    suffix: Vec::new(),
                }
            }
        })
        .collect())
}

fn simple_proof_path(index: u64, total: u64) -> Vec<bool> {
    if total < 2 {
        return Vec::new();
    }
    let left_count = split_point(total);
    if index < left_count {
        let mut path = simple_proof_path(index, left_count);
        path.push(true);
        path
    } else {
        let mut path = simple_proof_path(index - left_count, total - left_count);
        path.push(false);
        path
    }
}

fn split_point(length: u64) -> u64 {
    debug_assert!(length > 1);
    let bit_len = u64::BITS - (length - 1).leading_zeros();
    1u64 << (bit_len - 1)
}

#[derive(Clone, PartialEq, Message)]
struct RawCommitInfo {
    #[prost(int64, tag = "1")]
    version: Version,
    #[prost(message, repeated, tag = "2")]
    store_infos: Vec<RawStoreInfo>,
    #[prost(message, optional, tag = "3")]
    timestamp: Option<prost_types::Timestamp>,
}

#[derive(Clone, PartialEq, Message)]
struct RawStoreInfo {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    commit_id: Option<RawCommitId>,
}

impl From<StoreInfo> for RawStoreInfo {
    fn from(value: StoreInfo) -> Self {
        Self {
            name: value.name,
            commit_id: Some(RawCommitId::from(value.commit_id)),
        }
    }
}

impl TryFrom<RawStoreInfo> for StoreInfo {
    type Error = Error;

    fn try_from(value: RawStoreInfo) -> Result<Self> {
        Ok(Self {
            name: value.name,
            commit_id: CommitId::try_from(
                value
                    .commit_id
                    .ok_or(Error::InvalidCommitInfo("missing store commit ID"))?,
            )?,
        })
    }
}

#[derive(Clone, PartialEq, Message)]
struct RawCommitId {
    #[prost(int64, tag = "1")]
    version: Version,
    #[prost(bytes = "vec", tag = "2")]
    hash: Vec<u8>,
}

impl From<CommitId> for RawCommitId {
    fn from(value: CommitId) -> Self {
        Self {
            version: value.version,
            hash: value.hash.to_vec(),
        }
    }
}

impl TryFrom<RawCommitId> for CommitId {
    type Error = Error;

    fn try_from(value: RawCommitId) -> Result<Self> {
        if value.version <= 0 {
            return Err(Error::InvalidVersion {
                version: value.version,
            });
        }
        let hash = value
            .hash
            .try_into()
            .map_err(|_| Error::InvalidCommitInfo("commit hash must be 32 bytes"))?;
        Ok(Self {
            version: value.version,
            hash,
        })
    }
}
