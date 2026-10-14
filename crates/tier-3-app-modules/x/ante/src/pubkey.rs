//! Public keys accepted by the cosmos-sdk fork (`crypto/codec` registers
//! ed25519, secp256k1, secp256r1 and `LegacyAminoPubKey` multisig).

use celestia_proto::Any;
use celestia_proto::cosmos::crypto::multisig::LegacyAminoPubKey;
use prost::Message;
use sdk::SdkError;
use sdk::error::sdk as errs;

use crate::{R, panic};

/// Type URL of secp256k1 public keys.
pub const SECP256K1_TYPE_URL: &str = "/cosmos.crypto.secp256k1.PubKey";
/// Type URL of secp256r1 public keys.
pub const SECP256R1_TYPE_URL: &str = "/cosmos.crypto.secp256r1.PubKey";
/// Type URL of ed25519 public keys.
pub const ED25519_TYPE_URL: &str = "/cosmos.crypto.ed25519.PubKey";
/// Type URL of multisig public keys.
pub const MULTISIG_TYPE_URL: &str = "/cosmos.crypto.multisig.LegacyAminoPubKey";

/// The simulation placeholder key (`simSecp256k1Pubkey`).
pub const SIM_SECP256K1_PUBKEY: [u8; 33] = [
    0x03, 0x5A, 0xD6, 0x81, 0x0A, 0x47, 0xF0, 0x73, 0x55, 0x3F, 0xF3, 0x0D, 0x2F, 0xCC, 0x7E, 0x0D,
    0x3B, 0x1C, 0x0B, 0x74, 0xB6, 0x1A, 0x1A, 0xAA, 0x25, 0x82, 0x34, 0x40, 0x37, 0x15, 0x1E, 0x14,
    0x3A,
];

/// `bytes key = 1` public key messages (secp256k1, ed25519, secp256r1).
#[derive(Clone, PartialEq, Message)]
struct KeyBytes {
    #[prost(bytes = "vec", tag = "1")]
    key: Vec<u8>,
}

/// A decoded public key (`cryptotypes.PubKey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubKey {
    /// `secp256k1.PubKey`.
    Secp256k1(Vec<u8>),
    /// `secp256r1.PubKey` (compressed point).
    Secp256r1(Vec<u8>),
    /// `ed25519.PubKey`.
    Ed25519(Vec<u8>),
    /// `multisig.LegacyAminoPubKey`.
    Multisig {
        /// Threshold.
        threshold: u32,
        /// Sub-keys as packed (original `Any` bytes are kept, like gogoproto).
        anys: Vec<Any>,
        /// Decoded sub-keys.
        keys: Vec<PubKey>,
    },
}

fn decode_err(e: impl std::fmt::Display) -> SdkError {
    errs::tx_decode(e.to_string())
}

impl PubKey {
    /// The simulation placeholder.
    #[must_use]
    pub fn sim() -> Self {
        Self::Secp256k1(SIM_SECP256K1_PUBKEY.to_vec())
    }

    /// Unpacks an `Any` holding a registered public key.
    pub fn from_any(any: &Any) -> Result<Self, SdkError> {
        match any.type_url.as_str() {
            SECP256K1_TYPE_URL => Ok(Self::Secp256k1(
                KeyBytes::decode(any.value.as_slice())
                    .map_err(decode_err)?
                    .key,
            )),
            ED25519_TYPE_URL => Ok(Self::Ed25519(
                KeyBytes::decode(any.value.as_slice())
                    .map_err(decode_err)?
                    .key,
            )),
            SECP256R1_TYPE_URL => {
                let key = KeyBytes::decode(any.value.as_slice())
                    .map_err(decode_err)?
                    .key;
                if !crate::signing::p256_point_valid(&key) {
                    return Err(errs::tx_decode("invalid secp256r1 public key"));
                }
                Ok(Self::Secp256r1(key))
            }
            MULTISIG_TYPE_URL => {
                let m = LegacyAminoPubKey::decode(any.value.as_slice()).map_err(decode_err)?;
                let keys = m
                    .public_keys
                    .iter()
                    .map(Self::from_any)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Self::Multisig {
                    threshold: m.threshold,
                    anys: m.public_keys,
                    keys,
                })
            }
            other => Err(errs::tx_decode(format!(
                "no concrete type registered for type URL {other} against interface *types.PubKey"
            ))),
        }
    }

    /// `codectypes.NewAnyWithValue(pk)`: the canonical packing stored on
    /// accounts.
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Secp256k1(k) => Any {
                type_url: SECP256K1_TYPE_URL.into(),
                value: KeyBytes { key: k.clone() }.encode_to_vec(),
            },
            Self::Secp256r1(k) => Any {
                type_url: SECP256R1_TYPE_URL.into(),
                value: KeyBytes { key: k.clone() }.encode_to_vec(),
            },
            Self::Ed25519(k) => Any {
                type_url: ED25519_TYPE_URL.into(),
                value: KeyBytes { key: k.clone() }.encode_to_vec(),
            },
            Self::Multisig {
                threshold, anys, ..
            } => Any {
                type_url: MULTISIG_TYPE_URL.into(),
                value: LegacyAminoPubKey {
                    threshold: *threshold,
                    public_keys: anys.clone(),
                }
                .encode_to_vec(),
            },
        }
    }

    /// Go `%T` of the key.
    #[must_use]
    pub fn go_type(&self) -> &'static str {
        match self {
            Self::Secp256k1(_) => "*secp256k1.PubKey",
            Self::Secp256r1(_) => "*secp256r1.PubKey",
            Self::Ed25519(_) => "*ed25519.PubKey",
            Self::Multisig { .. } => "*multisig.LegacyAminoPubKey",
        }
    }

    /// Amino binary-bare encoding (`legacy.Cdc` / multisig `AminoCdc`).
    pub fn amino_bytes(&self) -> R<Vec<u8>> {
        let mut out = Vec::new();
        match self {
            Self::Secp256k1(k) => {
                out.extend_from_slice(&[0xeb, 0x5a, 0xe9, 0x87]);
                put_uvarint(&mut out, k.len() as u64);
                out.extend_from_slice(k);
            }
            Self::Ed25519(k) => {
                out.extend_from_slice(&[0x16, 0x24, 0xde, 0x64]);
                put_uvarint(&mut out, k.len() as u64);
                out.extend_from_slice(k);
            }
            Self::Secp256r1(_) => {
                return panic("amino: no concrete type registered for *secp256r1.PubKey");
            }
            Self::Multisig {
                threshold, keys, ..
            } => {
                out.extend_from_slice(&[0x22, 0xc1, 0xf7, 0xe2]);
                if *threshold != 0 {
                    out.push(0x08);
                    put_uvarint(&mut out, u64::from(*threshold));
                }
                for k in keys {
                    let bz = k.amino_bytes()?;
                    out.push(0x12);
                    put_uvarint(&mut out, bz.len() as u64);
                    out.extend_from_slice(&bz);
                }
            }
        }
        Ok(out)
    }

    /// `PubKey.Address()` (panics on malformed key lengths like Go).
    pub fn address(&self) -> R<Vec<u8>> {
        match self {
            Self::Secp256k1(k) => {
                if k.len() != 33 {
                    return panic("length of pubkey is incorrect");
                }
                Ok(hash::ripemd160(&hash::sha256(k)).to_vec())
            }
            Self::Ed25519(k) => {
                if k.len() != 32 {
                    return panic("pubkey is incorrect size");
                }
                Ok(hash::sha256(k)[..20].to_vec())
            }
            Self::Secp256r1(k) => {
                // address.Hash("cosmos.crypto.secp256r1.PubKey", key)
                let typ = hash::sha256(b"cosmos.crypto.secp256r1.PubKey");
                let mut buf = typ.to_vec();
                buf.extend_from_slice(k);
                Ok(hash::sha256(&buf).to_vec())
            }
            Self::Multisig { .. } => Ok(hash::sha256(&self.amino_bytes()?)[..20].to_vec()),
        }
    }

    /// `PubKey.VerifySignature` for single keys.
    pub fn verify_signature(&self, msg: &[u8], sig: &[u8]) -> R<bool> {
        match self {
            Self::Secp256k1(k) => Ok(crate::signing::verify_secp256k1(k, msg, sig)),
            Self::Secp256r1(k) => Ok(crate::signing::verify_secp256r1(k, msg, sig)),
            Self::Ed25519(k) => Ok(verify_ed25519(k, msg, sig)),
            Self::Multisig { .. } => panic("not implemented"),
        }
    }

    /// `CountSubKeys`.
    #[must_use]
    pub fn count_sub_keys(&self) -> u64 {
        match self {
            Self::Multisig { keys, .. } => keys.iter().map(Self::count_sub_keys).sum(),
            _ => 1,
        }
    }
}

fn verify_ed25519(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    // ed25519 accounts are rejected by the gas consumer before verification;
    // this mirrors cometbft's ZIP-215 verification for completeness.
    let (Ok(k), Ok(s)) = (<[u8; 32]>::try_from(key), <[u8; 64]>::try_from(sig)) else {
        return false;
    };
    let Ok(vk) = ed25519_consensus::VerificationKey::try_from(k) else {
        return false;
    };
    vk.verify(&ed25519_consensus::Signature::from(s), msg)
        .is_ok()
}

fn put_uvarint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(u8::try_from(v & 0x7f).unwrap_or(0) | 0x80);
        v >>= 7;
    }
    out.push(u8::try_from(v).unwrap_or(0));
}

/// Encodes a uvarint (protobuf/amino).
#[must_use]
pub fn uvarint(v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    put_uvarint(&mut out, v);
    out
}
