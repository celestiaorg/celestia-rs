//! Consensus public keys (`crypto.PubKey` as used by `types`, encoded via
//! `crypto/encoding`).

use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::proto::crypto::{PublicKey, public_key::Sum};

/// `ed25519.KeyType` / ABCI pubkey type name.
pub const ED25519_KEY_TYPE: &str = "ed25519";
/// `secp256k1.KeyType` / ABCI pubkey type name.
pub const SECP256K1_KEY_TYPE: &str = "secp256k1";
/// Ed25519 public key size.
pub const ED25519_PUB_KEY_SIZE: usize = 32;
/// Compressed secp256k1 public key size.
pub const SECP256K1_PUB_KEY_SIZE: usize = 33;

/// A validator public key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PubKey {
    /// Ed25519 (ZIP-215 verification, batch-verifiable).
    Ed25519([u8; ED25519_PUB_KEY_SIZE]),
    /// secp256k1 (compressed, lower-S `r || s` signatures over SHA-256).
    Secp256k1([u8; SECP256K1_PUB_KEY_SIZE]),
}

impl PubKey {
    /// Raw key bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Ed25519(b) => b,
            Self::Secp256k1(b) => b,
        }
    }

    /// `PubKey.Type()`.
    #[must_use]
    pub fn key_type(&self) -> &'static str {
        match self {
            Self::Ed25519(_) => ED25519_KEY_TYPE,
            Self::Secp256k1(_) => SECP256K1_KEY_TYPE,
        }
    }

    /// `PubKey.Address()` (20 bytes).
    #[must_use]
    pub fn address(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(b) => crypto::ed25519::address(b).to_vec(),
            Self::Secp256k1(b) => crypto::secp256k1::address(b).to_vec(),
        }
    }

    /// `PubKey.VerifySignature(msg, sig)`.
    #[must_use]
    pub fn verify_signature(&self, msg: &[u8], sig: &[u8]) -> bool {
        match self {
            Self::Ed25519(pk) => {
                let Ok(sig) = <[u8; 64]>::try_from(sig) else {
                    return false;
                };
                crypto::ed25519::verify(pk, msg, &sig)
            }
            Self::Secp256k1(pk) => {
                let Ok(sig) = <[u8; 64]>::try_from(sig) else {
                    return false;
                };
                crypto::secp256k1::verify(pk, msg, &sig)
            }
        }
    }

    /// `crypto/batch.SupportsBatchVerifier`.
    #[must_use]
    pub fn supports_batch_verifier(&self) -> bool {
        matches!(self, Self::Ed25519(_))
    }

    /// `cryptoenc.PubKeyToProto`.
    #[must_use]
    pub fn to_proto(&self) -> PublicKey {
        PublicKey {
            sum: Some(match self {
                Self::Ed25519(b) => Sum::Ed25519(b.to_vec()),
                Self::Secp256k1(b) => Sum::Secp256k1(b.to_vec()),
            }),
        }
    }

    /// `cryptoenc.PubKeyFromProto`.
    pub fn from_proto(pk: &PublicKey) -> Result<Self> {
        match &pk.sum {
            Some(Sum::Ed25519(b)) => <[u8; ED25519_PUB_KEY_SIZE]>::try_from(b.as_slice())
                .map(Self::Ed25519)
                .map_err(|_| {
                    Error::msg(format!(
                        "invalid size for PubKeyEd25519. Got {}, expected {}",
                        b.len(),
                        ED25519_PUB_KEY_SIZE
                    ))
                }),
            Some(Sum::Secp256k1(b)) => <[u8; SECP256K1_PUB_KEY_SIZE]>::try_from(b.as_slice())
                .map(Self::Secp256k1)
                .map_err(|_| {
                    Error::msg(format!(
                        "invalid size for PubKeySecp256k1. Got {}, expected {}",
                        b.len(),
                        SECP256K1_PUB_KEY_SIZE
                    ))
                }),
            None => Err(Error::msg("fromproto: key type <nil> is not supported")),
        }
    }
}

impl core::fmt::Display for PubKey {
    /// `PubKey.String()`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ed25519(b) => write!(f, "PubKeyEd25519{{{}}}", hex_upper(b)),
            Self::Secp256k1(b) => write!(f, "PubKeySecp256k1{{{}}}", hex_upper(b)),
        }
    }
}
