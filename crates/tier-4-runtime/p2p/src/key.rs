//! Node identity: `node_key.json` and node IDs (port of `p2p/key.go`).

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

/// Byte length of a node ID (a CometBFT address).
pub const ID_BYTE_LENGTH: usize = 20;

/// Amino type name of an Ed25519 private key in `node_key.json`.
const PRIV_KEY_TYPE: &str = "tendermint/PrivKeyEd25519";

/// A node ID: the first 20 bytes of `sha256(ed25519 pubkey)`, shown as
/// lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub [u8; ID_BYTE_LENGTH]);

impl NodeId {
    /// The ID of an Ed25519 public key (`p2p.PubKeyToID`).
    #[must_use]
    pub fn from_pubkey(pubkey: &[u8; 32]) -> Self {
        Self(crypto::ed25519::address(pubkey))
    }
}

/// Error for a malformed node ID (`validateID`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId(pub String);

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidId {}

impl FromStr for NodeId {
    type Err = InvalidId;

    /// Mirrors Go's `validateID`: non-empty, hex, 20 bytes.
    fn from_str(s: &str) -> Result<Self, InvalidId> {
        if s.is_empty() {
            return Err(InvalidId("no ID".into()));
        }
        let bytes = hex::decode(s).map_err(|e| InvalidId(format!("encoding/hex: {e}")))?;
        let arr: [u8; ID_BYTE_LENGTH] = bytes.as_slice().try_into().map_err(|_| {
            InvalidId(format!(
                "invalid hex length - got {}, expected {ID_BYTE_LENGTH}",
                bytes.len()
            ))
        })?;
        Ok(Self(arr))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({self})")
    }
}

impl Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Errors loading or saving a node key.
#[derive(Debug)]
pub enum NodeKeyError {
    /// Filesystem error.
    Io(std::io::Error),
    /// The JSON was malformed or not an Ed25519 key.
    Format(String),
}

impl fmt::Display for NodeKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "node key io: {e}"),
            Self::Format(e) => write!(f, "node key format: {e}"),
        }
    }
}

impl std::error::Error for NodeKeyError {}

impl From<std::io::Error> for NodeKeyError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// The node's persistent Ed25519 key (`p2p.NodeKey`).
#[derive(Clone)]
pub struct NodeKey {
    private_key: crypto::ed25519::PrivateKeyBytes,
}

impl fmt::Debug for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeKey({})", self.id())
    }
}

#[derive(Serialize, Deserialize)]
struct TypedValue {
    #[serde(rename = "type")]
    ty: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
struct NodeKeyJson {
    priv_key: TypedValue,
}

impl NodeKey {
    /// A node key from a 32-byte Ed25519 seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            private_key: crypto::ed25519::private_key_from_seed(seed),
        }
    }

    /// A fresh random node key (`ed25519.GenPrivKey`).
    #[must_use]
    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::random();
        Self::from_seed(&seed)
    }

    /// The 64-byte `seed || pubkey` private key.
    #[must_use]
    pub fn private_key(&self) -> &crypto::ed25519::PrivateKeyBytes {
        &self.private_key
    }

    /// The Ed25519 public key.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&self.private_key[32..]);
        pk
    }

    /// The node ID.
    #[must_use]
    pub fn id(&self) -> NodeId {
        NodeId::from_pubkey(&self.public_key())
    }

    /// Sign `msg` (deterministic Ed25519, as Go).
    #[must_use]
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        // The private key is always well-formed (built from a seed or
        // validated on load), so signing cannot fail.
        crypto::ed25519::sign(&self.private_key, msg).unwrap_or([0u8; 64])
    }

    /// Parse the `node_key.json` format written by celestia-core.
    pub fn from_json(json: &[u8]) -> Result<Self, NodeKeyError> {
        let parsed: NodeKeyJson =
            serde_json::from_slice(json).map_err(|e| NodeKeyError::Format(e.to_string()))?;
        if parsed.priv_key.ty != PRIV_KEY_TYPE {
            return Err(NodeKeyError::Format(format!(
                "unsupported key type {}",
                parsed.priv_key.ty
            )));
        }
        let raw = B64
            .decode(parsed.priv_key.value.as_bytes())
            .map_err(|e| NodeKeyError::Format(e.to_string()))?;
        let private_key: crypto::ed25519::PrivateKeyBytes = raw
            .as_slice()
            .try_into()
            .map_err(|_| NodeKeyError::Format(format!("private key length {}", raw.len())))?;
        crypto::ed25519::public_key_from_private_key(&private_key)
            .map_err(|e| NodeKeyError::Format(e.to_string()))?;
        Ok(Self { private_key })
    }

    /// Serialize exactly as Go's `cmtjson.Marshal(nodeKey)`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let j = NodeKeyJson {
            priv_key: TypedValue {
                ty: PRIV_KEY_TYPE.into(),
                value: B64.encode(self.private_key),
            },
        };
        serde_json::to_string(&j).unwrap_or_default()
    }

    /// Load `node_key.json`.
    pub fn load(path: &Path) -> Result<Self, NodeKeyError> {
        Self::from_json(&std::fs::read(path)?)
    }

    /// Save as `node_key.json` (mode 0600 like Go).
    pub fn save(&self, path: &Path) -> Result<(), NodeKeyError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// `LoadOrGenNodeKey`.
    pub fn load_or_generate(path: &Path) -> Result<Self, NodeKeyError> {
        if path.exists() {
            return Self::load(path);
        }
        let key = Self::generate();
        key.save(path)?;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_parsing() {
        let id: NodeId = "DEADBEEFdeadbeefdeadbeefdeadbeefdeadbeef".parse().unwrap();
        assert_eq!(id.to_string(), "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert!("".parse::<NodeId>().is_err());
        assert!("dead".parse::<NodeId>().is_err());
        assert!("zz".parse::<NodeId>().is_err());
    }

    #[test]
    fn json_roundtrip() {
        let k = NodeKey::from_seed(&[7; 32]);
        let back = NodeKey::from_json(k.to_json().as_bytes()).unwrap();
        assert_eq!(back.id(), k.id());
    }
}
