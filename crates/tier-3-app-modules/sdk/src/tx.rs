//! A decoded transaction, shared by the decoder (`txdecode`), the ante
//! handler (`x-ante`) and the app's `runTx`.

use celestia_proto::Any;
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody};

/// A blob carried by a `BlobTx` wrapper (go-square v4 `Blob`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Namespace ID bytes (without version).
    pub namespace_id: Vec<u8>,
    /// Namespace version.
    pub namespace_version: u32,
    /// Share version.
    pub share_version: u32,
    /// Blob data.
    pub data: Vec<u8>,
    /// Signer (share versions 1 and 2; empty for version 0).
    pub signer: Vec<u8>,
}

/// A decoded SDK transaction (`sdk.Tx` from `DefaultTxDecoder`), together
/// with celestia wrapper data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tx {
    /// The SDK tx bytes (inner tx for `BlobTx`/`IndexWrapper`).
    pub tx_bytes: Vec<u8>,
    /// `TxRaw.body_bytes` exactly as received (signed over).
    pub body_bytes: Vec<u8>,
    /// `TxRaw.auth_info_bytes` exactly as received (signed over).
    pub auth_info_bytes: Vec<u8>,
    /// `TxRaw.signatures`.
    pub signatures: Vec<Vec<u8>>,
    /// Decoded body.
    pub body: TxBody,
    /// Decoded auth info.
    pub auth_info: AuthInfo,
    /// Whether the body had unknown non-critical (>= 1024) fields.
    pub body_has_unknown_non_criticals: bool,
    /// Blobs, when the tx was a `BlobTx`.
    pub blobs: Vec<Blob>,
    /// Share indexes, when the tx was an `IndexWrapper`.
    pub share_indexes: Vec<u32>,
    /// The messages as Go's message handlers see them (the `Any` cached
    /// values after `UnpackInterfaces`): same type URLs as `body.messages`,
    /// but each value re-encoded from the decoded Go struct, so prost decodes
    /// it to the same content Go holds (customtype amounts normalized, e.g.
    /// `"0x10"` -> `"16"`; strings keep their bytes, see `gostr`).
    /// `body.messages` keeps the bytes as received.
    pub normalized_msgs: Vec<Any>,
}

impl Tx {
    /// The tx messages.
    #[must_use]
    pub fn msgs(&self) -> &[Any] {
        &self.body.messages
    }

    /// The messages as `sdk.Tx.GetMsgs()` returns them in Go (the decoded
    /// gogoproto structs, [`Self::normalized_msgs`]). Code that decodes a
    /// message's fields must read these: prost reads the bytes as received
    /// differently from gogoproto where the tx decoder's unknown-field
    /// check does not look (packed scalars: 10-byte varints with high bits,
    /// an element running past its packed run; a repeated `stdtime` /
    /// `stdduration`, which gogoproto overwrites and prost merges; a
    /// repeated oneof message, which gogoproto replaces). Falls back to the
    /// bytes as received for a tx built without them.
    #[must_use]
    pub fn gogo_msgs(&self) -> &[Any] {
        if self.normalized_msgs.len() == self.body.messages.len()
            && self
                .normalized_msgs
                .iter()
                .zip(&self.body.messages)
                .all(|(n, r)| n.type_url == r.type_url)
        {
            &self.normalized_msgs
        } else {
            &self.body.messages
        }
    }

    /// Gas limit from the fee (0 if absent).
    #[must_use]
    pub fn gas_limit(&self) -> u64 {
        self.auth_info.fee.as_ref().map_or(0, |f| f.gas_limit)
    }
}
