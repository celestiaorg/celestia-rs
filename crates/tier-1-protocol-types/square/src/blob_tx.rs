//! Blob transaction wrapper helpers.

use encoding::protobuf::Message;
use shares::{Blob, Namespace};

use crate::{Error, Result};

/// go-square protobuf type marker for blob transactions.
pub const PROTO_BLOB_TX_TYPE_ID: &str = "BLOB";

/// A blob transaction wrapper containing the original tx and attached blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobTx {
    /// Original encoded SDK transaction bytes.
    pub tx: Vec<u8>,
    /// Blobs attached to the transaction.
    pub blobs: Vec<Blob>,
}

impl BlobTx {
    /// Construct a blob transaction wrapper.
    pub fn new(tx: Vec<u8>, blobs: Vec<Blob>) -> Result<Self> {
        let blob_tx = Self { tx, blobs };
        blob_tx.validate_basic()?;
        Ok(blob_tx)
    }

    /// Validate the wrapper shape.
    pub fn validate_basic(&self) -> Result<()> {
        if self.blobs.is_empty() {
            return Err(Error::InvalidBlobTx);
        }
        for blob in &self.blobs {
            blob.validate_basic()?;
        }
        Ok(())
    }

    /// Encode this wrapper as go-square compatible protobuf bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        let proto = BlobTxProto {
            tx: self.tx.clone(),
            blobs: self.blobs.iter().map(BlobProto::from_blob).collect(),
            type_id: PROTO_BLOB_TX_TYPE_ID.to_owned(),
        };
        let mut out = Vec::with_capacity(proto.encoded_len());
        proto
            .encode(&mut out)
            .map_err(|error| Error::Encode(error.to_string()))?;
        Ok(out)
    }

    /// Decode protobuf bytes, returning `None` when the bytes are not a blob tx.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>> {
        let Ok(proto) = BlobTxProto::decode(bytes) else {
            return Ok(None);
        };
        if proto.type_id != PROTO_BLOB_TX_TYPE_ID || proto.blobs.is_empty() {
            return Ok(None);
        }

        let blobs = proto
            .blobs
            .into_iter()
            .map(BlobProto::into_blob)
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Self {
            tx: proto.tx,
            blobs,
        }))
    }
}

#[derive(Clone, PartialEq, Message)]
struct BlobProto {
    #[prost(bytes = "vec", tag = "1")]
    namespace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    data: Vec<u8>,
    #[prost(uint32, tag = "3")]
    share_version: u32,
    #[prost(uint32, tag = "4")]
    namespace_version: u32,
}

impl BlobProto {
    fn from_blob(blob: &Blob) -> Self {
        Self {
            namespace_id: blob.namespace().id().to_vec(),
            data: blob.data().to_vec(),
            share_version: u32::from(blob.share_version()),
            namespace_version: u32::from(blob.namespace().version()),
        }
    }

    fn into_blob(self) -> Result<Blob> {
        let namespace_id: [u8; shares::NAMESPACE_ID_SIZE] = self
            .namespace_id
            .try_into()
            .map_err(|_| Error::InvalidBlobTx)?;
        let namespace_version =
            u8::try_from(self.namespace_version).map_err(|_| Error::InvalidBlobTx)?;
        let share_version = u8::try_from(self.share_version).map_err(|_| Error::InvalidBlobTx)?;
        let namespace = Namespace::new(namespace_version, namespace_id)?;
        Blob::new(namespace, self.data, share_version).map_err(Error::from)
    }
}

#[derive(Clone, PartialEq, Message)]
struct BlobTxProto {
    #[prost(bytes = "vec", tag = "1")]
    tx: Vec<u8>,
    #[prost(message, repeated, tag = "2")]
    blobs: Vec<BlobProto>,
    #[prost(string, tag = "3")]
    type_id: String,
}
