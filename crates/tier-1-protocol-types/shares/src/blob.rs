//! Blob and index-wrapper helpers.

use crate::share::{DEFAULT_SHARE_VERSION, MAX_SHARE_VERSION, SHARE_VERSION_ONE, SIGNER_SIZE};
use crate::{Error, Namespace, Result, Share, split_blobs};
use encoding::protobuf::Message;

/// Blob transaction protobuf type marker.
pub const PROTO_BLOB_TX_TYPE_ID: &str = "BLOB";
/// Index-wrapper protobuf type marker.
pub const PROTO_INDEX_WRAPPER_TYPE_ID: &str = "INDX";

/// Celestia blob data targeted at a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Blob namespace.
    pub namespace: Namespace,
    /// Raw blob data.
    pub data: Vec<u8>,
    /// Share version used to encode this blob.
    pub share_version: u8,
    /// Optional signer encoded in version-one blob shares.
    pub signer: Option<[u8; SIGNER_SIZE]>,
}

impl Blob {
    /// Construct a blob.
    pub fn new(namespace: Namespace, data: Vec<u8>, share_version: u8) -> Result<Self> {
        Self::new_with_signer(namespace, data, share_version, None)
    }

    /// Construct a blob with an optional signer.
    pub fn new_with_signer(
        namespace: Namespace,
        data: Vec<u8>,
        share_version: u8,
        signer: Option<[u8; SIGNER_SIZE]>,
    ) -> Result<Self> {
        let blob = Self {
            namespace,
            data,
            share_version,
            signer,
        };
        blob.validate_basic()?;
        Ok(blob)
    }

    /// Construct a blob using the default share version.
    pub fn new_v0(namespace: Namespace, data: Vec<u8>) -> Result<Self> {
        Self::new(namespace, data, DEFAULT_SHARE_VERSION)
    }

    /// Construct a signer-aware version-one blob.
    pub fn new_v1(namespace: Namespace, data: Vec<u8>, signer: [u8; SIGNER_SIZE]) -> Result<Self> {
        Self::new_with_signer(namespace, data, SHARE_VERSION_ONE, Some(signer))
    }

    /// Validate this blob.
    pub fn validate_basic(&self) -> Result<()> {
        self.namespace.validate_basic()?;
        if self.data.is_empty() {
            return Err(Error::InvalidBlob);
        }
        if self.share_version > MAX_SHARE_VERSION {
            return Err(Error::InvalidShareVersion {
                version: self.share_version,
                max: MAX_SHARE_VERSION,
            });
        }
        if self.share_version == SHARE_VERSION_ONE && self.signer.is_none() {
            return Err(Error::InvalidSignerLength {
                actual: 0,
                expected: SIGNER_SIZE,
            });
        }
        if self.share_version != SHARE_VERSION_ONE && self.signer.is_some() {
            return Err(Error::InvalidBlob);
        }
        Ok(())
    }

    /// Return the blob namespace.
    #[must_use]
    pub fn namespace(&self) -> Namespace {
        self.namespace
    }

    /// Return the blob data.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Return the blob data length.
    #[must_use]
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// Return the blob share version.
    #[must_use]
    pub fn share_version(&self) -> u8 {
        self.share_version
    }

    /// Return the signer encoded in version-one blob shares.
    #[must_use]
    pub const fn signer(&self) -> Option<[u8; SIGNER_SIZE]> {
        self.signer
    }

    /// Return true when this blob has an encoded signer.
    #[must_use]
    pub const fn has_signer(&self) -> bool {
        self.signer.is_some()
    }

    /// Return true when the blob data is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Split this blob into sparse shares.
    pub fn to_shares(&self) -> Result<Vec<Share>> {
        split_blobs(core::slice::from_ref(self))
    }
}

/// Index-wrapper transaction containing blob share indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexWrapper {
    /// Wrapped transaction bytes.
    pub tx: Vec<u8>,
    /// Blob share indexes referenced by the transaction.
    pub share_indexes: Vec<u32>,
}

impl IndexWrapper {
    /// Construct an index wrapper.
    #[must_use]
    pub fn new(tx: Vec<u8>, share_indexes: Vec<u32>) -> Self {
        Self { tx, share_indexes }
    }

    /// Encode this index wrapper as go-square compatible protobuf bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let proto = IndexWrapperProto {
            tx: self.tx.clone(),
            share_indexes: self.share_indexes.clone(),
            type_id: PROTO_INDEX_WRAPPER_TYPE_ID.to_owned(),
        };
        let mut out = Vec::with_capacity(proto.encoded_len());
        proto
            .encode(&mut out)
            .map_err(|error| Error::Encode(error.to_string()))?;
        Ok(out)
    }

    /// Decode index-wrapper protobuf bytes, returning `None` for non-wrappers.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>> {
        let Ok(proto) = IndexWrapperProto::decode(bytes) else {
            return Ok(None);
        };
        if proto.type_id != PROTO_INDEX_WRAPPER_TYPE_ID {
            return Ok(None);
        }
        Ok(Some(Self {
            tx: proto.tx,
            share_indexes: proto.share_indexes,
        }))
    }
}

#[derive(Clone, PartialEq, Message)]
struct IndexWrapperProto {
    #[prost(bytes = "vec", tag = "1")]
    tx: Vec<u8>,
    #[prost(uint32, repeated, tag = "2")]
    share_indexes: Vec<u32>,
    #[prost(string, tag = "3")]
    type_id: String,
}
