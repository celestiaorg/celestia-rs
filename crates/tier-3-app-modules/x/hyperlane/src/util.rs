//! Port of hyperlane-cosmos `util`: `HexAddress`, `HyperlaneMessage`, the
//! incremental merkle tree and Ethereum signing helpers.

use std::fmt;

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

/// Length of a [`HexAddress`].
pub const HEX_ADDRESS_LENGTH: usize = 32;
/// Length of the gogoproto customtype encoding (`0x` + 64 hex chars).
pub const ENCODED_HEX_ADDRESS_LENGTH: usize = 66;

/// Keccak-256.
#[must_use]
pub fn keccak256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Keccak256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// 32-byte identifier: `[0:20]` module name, `[20:24]` module type,
/// `[24:32]` internal id (`util.HexAddress`).
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HexAddress(pub [u8; HEX_ADDRESS_LENGTH]);

impl HexAddress {
    /// `GenerateHexAddress`.
    #[must_use]
    pub fn generate(module: &[u8; 20], internal_type: u32, internal_id: u64) -> Self {
        let mut b = [0u8; 32];
        b[..20].copy_from_slice(module);
        b[20..24].copy_from_slice(&internal_type.to_be_bytes());
        b[24..].copy_from_slice(&internal_id.to_be_bytes());
        Self(b)
    }

    /// `GetInternalId`.
    #[must_use]
    pub fn internal_id(&self) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.0[24..]);
        u64::from_be_bytes(b)
    }

    /// `GetType`.
    #[must_use]
    pub fn module_type(&self) -> u32 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&self.0[20..24]);
        u32::from_be_bytes(b)
    }

    /// `IsZeroAddress`.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Raw bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// `DecodeHexAddress`: optional `0x` prefix followed by 64 hex chars.
    pub fn decode(s: &str) -> Result<Self, String> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != HEX_ADDRESS_LENGTH * 2 {
            return Err("invalid hex address length".into());
        }
        let b = hex::decode(s).map_err(|e| e.to_string())?;
        let mut out = [0u8; 32];
        out.copy_from_slice(&b);
        Ok(Self(out))
    }

    /// gogoproto customtype `Unmarshal` (exactly 66 bytes). An empty string is
    /// treated as an absent non-nullable field, i.e. the zero address.
    pub fn from_proto(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Ok(Self::default());
        }
        if s.len() != ENCODED_HEX_ADDRESS_LENGTH {
            return Err("invalid hex address length".into());
        }
        Self::decode(s)
    }

    /// Nullable customtype: an empty string is `nil`.
    pub fn from_proto_opt(s: &str) -> Result<Option<Self>, String> {
        if s.is_empty() {
            Ok(None)
        } else {
            Self::from_proto(s).map(Some)
        }
    }

    /// Nullable customtype encoding: `None` is the empty (omitted) string.
    #[must_use]
    pub fn opt_to_proto(v: Option<&Self>) -> String {
        v.map(ToString::to_string).unwrap_or_default()
    }
}

impl fmt::Display for HexAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl fmt::Debug for HexAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Offset of the message body.
pub const BODY_OFFSET: usize = 77;

/// Hyperlane message (`util.HyperlaneMessage`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HyperlaneMessage {
    /// Version.
    pub version: u8,
    /// Nonce.
    pub nonce: u32,
    /// Origin domain.
    pub origin: u32,
    /// Sender.
    pub sender: HexAddress,
    /// Destination domain.
    pub destination: u32,
    /// Recipient.
    pub recipient: HexAddress,
    /// Body.
    pub body: Vec<u8>,
}

fn be_u32(b: &[u8]) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[..4]);
    u32::from_be_bytes(a)
}

fn addr32(b: &[u8]) -> HexAddress {
    let mut a = [0u8; 32];
    a.copy_from_slice(&b[..32]);
    HexAddress(a)
}

impl HyperlaneMessage {
    /// `ParseHyperlaneMessage`.
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        if raw.len() < BODY_OFFSET {
            return Err("invalid hyperlane message".into());
        }
        Ok(Self {
            version: raw[0],
            nonce: be_u32(&raw[1..5]),
            origin: be_u32(&raw[5..9]),
            sender: addr32(&raw[9..41]),
            destination: be_u32(&raw[41..45]),
            recipient: addr32(&raw[45..77]),
            body: raw[BODY_OFFSET..].to_vec(),
        })
    }

    /// Wire encoding.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BODY_OFFSET + self.body.len());
        out.push(self.version);
        out.extend_from_slice(&self.nonce.to_be_bytes());
        out.extend_from_slice(&self.origin.to_be_bytes());
        out.extend_from_slice(&self.sender.0);
        out.extend_from_slice(&self.destination.to_be_bytes());
        out.extend_from_slice(&self.recipient.0);
        out.extend_from_slice(&self.body);
        out
    }

    /// Message id: keccak256 of the encoding.
    #[must_use]
    pub fn id(&self) -> HexAddress {
        HexAddress(keccak256(&[&self.bytes()]))
    }
}

impl fmt::Display for HyperlaneMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.bytes()))
    }
}

/// `sdk.AccAddressFromBech32` (and the bech32 `address.Codec.StringToBytes`)
/// with the `celestia` prefix.
pub fn acc_address_from_bech32(s: &str) -> Result<Vec<u8>, String> {
    if s.trim().is_empty() {
        return Err("empty address string is not allowed".into());
    }
    let (hrp, bz) = encoding::bech32::decode(s).map_err(|e| e.to_string())?;
    if hrp != encoding::bech32::ACCOUNT_PREFIX {
        return Err(format!(
            "invalid Bech32 prefix; expected {}, got {hrp}",
            encoding::bech32::ACCOUNT_PREFIX
        ));
    }
    if bz.is_empty() {
        return Err("addresses cannot be empty: unknown address".into());
    }
    if bz.len() > 255 {
        return Err(format!(
            "address max length is 255, got {}: unknown address",
            bz.len()
        ));
    }
    Ok(bz)
}

/// `sdk.AccAddress.String()`: bech32 with the `celestia` prefix, empty for
/// an empty address.
#[must_use]
pub fn acc_address_string(bz: &[u8]) -> String {
    if bz.is_empty() {
        return String::new();
    }
    encoding::bech32::encode(encoding::bech32::ACCOUNT_PREFIX, bz).unwrap_or_default()
}

/// `util.DecodeEthHex`: optional `0x` prefix, then hex (even length).
pub fn decode_eth_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|e| e.to_string())
}

/// `util.EncodeEthHex`.
#[must_use]
pub fn encode_eth_hex(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

/// EIP-191 signing hash (`GetEthSigningHash`).
#[must_use]
pub fn eth_signing_hash(msg: &[u8]) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", msg.len());
    keccak256(&[prefix.as_bytes(), msg])
}

/// Recovers the Ethereum address that produced `sig` (65 bytes, recovery id
/// 27/28) over `hash` (`RecoverEthSignature` + `PubkeyToAddress`).
pub fn recover_eth_address(hash: &[u8; 32], sig: &[u8]) -> Result<[u8; 20], String> {
    if sig.len() != 65 {
        return Err("invalid signature".into());
    }
    // go-ethereum `SigToPub` (libsecp256k1 `RecoverPubkey`): the recovery id
    // must be < 4 after subtracting 27; high-S signatures are accepted.
    let v = sig[64].wrapping_sub(27);
    let mut signature = Signature::from_slice(&sig[..64]).map_err(|e| e.to_string())?;
    let mut rid = RecoveryId::from_byte(v).ok_or("invalid signature recovery id")?;
    if let Some(low) = signature.normalize_s() {
        // (r, n - s) with the opposite y parity recovers the same key; k256
        // only verifies low-S signatures.
        signature = low;
        rid = RecoveryId::new(!rid.is_y_odd(), rid.is_x_reduced());
    }
    let key = VerifyingKey::recover_from_prehash(hash, &signature, rid)
        .map_err(|_| "recovery failed".to_string())?;
    let point = key.to_encoded_point(false);
    let h = keccak256(&[&point.as_bytes()[1..]]);
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    Ok(out)
}

/// Depth of the incremental merkle tree.
pub const TREE_DEPTH: usize = 32;
/// Maximum number of leaves.
pub const MAX_LEAVES: u64 = (1 << TREE_DEPTH) - 1;

/// `ZeroHashes[i]`: root of an empty subtree of height `i`.
#[must_use]
pub fn zero_hashes() -> [[u8; 32]; TREE_DEPTH] {
    let mut z = [[0u8; 32]; TREE_DEPTH];
    for i in 1..TREE_DEPTH {
        z[i] = keccak256(&[&z[i - 1], &z[i - 1]]);
    }
    z
}

/// Incremental merkle tree (`util.MerkleTree`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleTree {
    /// Current branch.
    pub branch: [[u8; 32]; TREE_DEPTH],
    /// Number of inserted leaves.
    pub count: u32,
}

impl MerkleTree {
    /// `Insert`.
    pub fn insert(&mut self, node: [u8; 32]) -> Result<(), String> {
        if u64::from(self.count) >= MAX_LEAVES {
            return Err("merkle tree full".into());
        }
        self.count += 1;
        let mut size = self.count;
        let mut node = node;
        for i in 0..TREE_DEPTH {
            if size & 1 == 1 {
                self.branch[i] = node;
                return Ok(());
            }
            node = keccak256(&[&self.branch[i], &node]);
            size /= 2;
        }
        Err("unreachable code".into())
    }

    /// Current root.
    #[must_use]
    pub fn root(&self) -> [u8; 32] {
        let zeroes = zero_hashes();
        let mut current = [0u8; 32];
        for (i, zero) in zeroes.iter().enumerate() {
            if (self.count >> i) & 1 == 1 {
                current = keccak256(&[&self.branch[i], &current]);
            } else {
                current = keccak256(&[&current, zero]);
            }
        }
        current
    }
}

/// `BranchRoot`: root for `item` at `index` given its merkle branch.
#[must_use]
pub fn branch_root(item: [u8; 32], branch: &[[u8; 32]; TREE_DEPTH], index: u32) -> [u8; 32] {
    let mut current = item;
    for (i, next) in branch.iter().enumerate() {
        if (index >> i) & 1 == 1 {
            current = keccak256(&[next, &current]);
        } else {
            current = keccak256(&[&current, next]);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hashes_match_go_table() {
        let z = zero_hashes();
        assert_eq!(
            hex::encode(z[1]),
            "ad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5"
        );
        assert_eq!(
            hex::encode(z[31]),
            "8448818bb4ae4562849e949e17ac16e0be16688e156b5cf15e098c627c0056a9"
        );
    }

    #[test]
    fn hex_address_roundtrip() {
        let a = HexAddress::generate(b"hyperlane\0\0\0\0\0\0\0\0\0\0\0", 3, 7);
        assert_eq!(a.internal_id(), 7);
        assert_eq!(a.module_type(), 3);
        let s = a.to_string();
        assert_eq!(s.len(), 66);
        assert_eq!(HexAddress::from_proto(&s), Ok(a));
        assert!(HexAddress::from_proto(&s[2..]).is_err());
        assert_eq!(HexAddress::decode(&s[2..]), Ok(a));
    }

    #[test]
    fn message_roundtrip() {
        let m = HyperlaneMessage {
            version: 3,
            nonce: 1,
            origin: 2,
            sender: HexAddress([1; 32]),
            destination: 3,
            recipient: HexAddress([2; 32]),
            body: vec![9, 9],
        };
        assert_eq!(HyperlaneMessage::parse(&m.bytes()), Ok(m.clone()));
        assert!(HyperlaneMessage::parse(&[0; 76]).is_err());
    }

    #[test]
    fn recover_signature() {
        use k256::ecdsa::SigningKey;
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let hash = eth_signing_hash(b"hello");
        let (sig, rid) = sk.sign_prehash_recoverable(&hash).unwrap();
        let mut raw = sig.to_bytes().to_vec();
        raw.push(rid.to_byte() + 27);
        let pk = sk.verifying_key().to_encoded_point(false);
        let expected = &keccak256(&[&pk.as_bytes()[1..]])[12..];
        assert_eq!(recover_eth_address(&hash, &raw).unwrap(), expected);
    }

    #[test]
    fn tree_root_with_one_leaf() {
        let mut t = MerkleTree {
            branch: [[0; 32]; TREE_DEPTH],
            count: 0,
        };
        let empty_root = t.root();
        t.insert([5; 32]).unwrap();
        assert_ne!(t.root(), empty_root);
        // A proof of zero hashes for index 0 reproduces the root.
        assert_eq!(branch_root([5; 32], &zero_hashes(), 0), t.root());
    }
}
