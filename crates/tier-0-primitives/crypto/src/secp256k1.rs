//! Cosmos SDK secp256k1 account signature helpers.

use hash::{ripemd160, sha256};
use k256::SecretKey;
use k256::ecdsa::signature::{Signer, Verifier};
use k256::ecdsa::{Signature, SigningKey, VerifyingKey};

/// Size in bytes of a secp256k1 secret key.
pub const SECRET_KEY_LENGTH: usize = 32;

/// Size in bytes of a compressed secp256k1 public key.
pub const PUBLIC_KEY_LENGTH: usize = 33;

/// Size in bytes of a Cosmos secp256k1 signature, encoded as r || s.
pub const SIGNATURE_LENGTH: usize = 64;

/// Size in bytes of a Cosmos account address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte secp256k1 secret key.
pub type SecretKeyBytes = [u8; SECRET_KEY_LENGTH];

/// A 33-byte compressed secp256k1 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A 64-byte secp256k1 signature, encoded as r || s.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// A 20-byte Cosmos account address.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

const ORDER_MINUS_ONE: SecretKeyBytes = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x40,
];

/// Errors returned by secp256k1 helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret key is zero or outside the secp256k1 scalar field.
    InvalidSecretKey,
    /// The compressed public key is malformed or not on curve.
    InvalidPublicKey,
    /// The signature scalar pair is malformed or not in lower-S form.
    InvalidSignature,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSecretKey => f.write_str("invalid secp256k1 secret key"),
            Self::InvalidPublicKey => f.write_str("invalid secp256k1 public key"),
            Self::InvalidSignature => f.write_str("invalid secp256k1 signature"),
        }
    }
}

impl std::error::Error for Error {}

/// Derive the Cosmos SDK secp256k1 secret key from an arbitrary secret.
#[must_use]
pub fn secret_key_from_secret(secret: &[u8]) -> SecretKeyBytes {
    let mut key = sha256(secret);
    if key.as_slice() >= ORDER_MINUS_ONE.as_slice() {
        sub_assign_be(&mut key, &ORDER_MINUS_ONE);
    }
    add_one_be(&mut key);
    key
}

/// Parse a raw 32-byte secp256k1 secret key.
pub fn secret_key_from_bytes(bytes: &SecretKeyBytes) -> Result<SecretKeyBytes, Error> {
    SecretKey::from_slice(bytes)
        .map(|_| *bytes)
        .map_err(|_| Error::InvalidSecretKey)
}

/// Derive the compressed secp256k1 public key for `secret_key`.
pub fn public_key_from_secret_key(secret_key: &SecretKeyBytes) -> Result<PublicKeyBytes, Error> {
    let secret_key = SecretKey::from_slice(secret_key).map_err(|_| Error::InvalidSecretKey)?;
    let signing_key = SigningKey::from(secret_key);
    let encoded = signing_key.verifying_key().to_encoded_point(true);
    encoded
        .as_bytes()
        .try_into()
        .map_err(|_| Error::InvalidPublicKey)
}

/// Sign `msg` using Cosmos SDK secp256k1 semantics.
pub fn sign(secret_key: &SecretKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error> {
    let secret_key = SecretKey::from_slice(secret_key).map_err(|_| Error::InvalidSecretKey)?;
    let signing_key = SigningKey::from(secret_key);
    let signature: Signature = signing_key.sign(msg);
    signature
        .to_bytes()
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidSignature)
}

/// Verify a Cosmos SDK lower-S r || s secp256k1 signature.
#[must_use]
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(public_key) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(signature.as_slice()) else {
        return false;
    };
    if signature.normalize_s().is_some() {
        return false;
    }
    verifying_key.verify(msg, &signature).is_ok()
}

/// Return Cosmos's secp256k1 address: RIPEMD-160(SHA-256(compressed_public_key)).
#[must_use]
pub fn address(public_key: &PublicKeyBytes) -> AddressBytes {
    ripemd160(&sha256(public_key))
}

fn sub_assign_be(lhs: &mut SecretKeyBytes, rhs: &SecretKeyBytes) {
    let mut borrow = 0u16;
    for (left, right) in lhs.iter_mut().rev().zip(rhs.iter().rev()) {
        let subtrahend = u16::from(*right) + borrow;
        let current = u16::from(*left);
        if current >= subtrahend {
            *left = u8::try_from(current - subtrahend).unwrap_or(0);
            borrow = 0;
        } else {
            *left = u8::try_from(current + 256 - subtrahend).unwrap_or(0);
            borrow = 1;
        }
    }
}

fn add_one_be(value: &mut SecretKeyBytes) {
    for byte in value.iter_mut().rev() {
        let (sum, overflow) = byte.overflowing_add(1);
        *byte = sum;
        if !overflow {
            break;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages_are_stable() {
        assert_eq!(
            Error::InvalidSecretKey.to_string(),
            "invalid secp256k1 secret key"
        );
        assert_eq!(
            Error::InvalidPublicKey.to_string(),
            "invalid secp256k1 public key"
        );
        assert_eq!(
            Error::InvalidSignature.to_string(),
            "invalid secp256k1 signature"
        );
    }

    #[test]
    fn rejects_invalid_inputs() {
        let invalid_secret_key = [0u8; SECRET_KEY_LENGTH];
        let invalid_public_key = [0u8; PUBLIC_KEY_LENGTH];

        assert_eq!(
            public_key_from_secret_key(&invalid_secret_key),
            Err(Error::InvalidSecretKey)
        );
        assert_eq!(
            sign(&invalid_secret_key, b"message"),
            Err(Error::InvalidSecretKey)
        );

        let secret_key = secret_key_from_secret(b"valid secp256k1 secret");
        let public_key = public_key_from_secret_key(&secret_key).expect("valid secp256k1 secret");
        let signature = sign(&secret_key, b"message").expect("valid secp256k1 secret");
        let invalid_signature = [0xff; SIGNATURE_LENGTH];

        assert!(!verify(&invalid_public_key, b"message", &signature));
        assert!(!verify(&public_key, b"message", &invalid_signature));
    }

    #[test]
    fn scalar_helpers_handle_borrow_and_overflow() {
        let mut lhs = [0u8; SECRET_KEY_LENGTH];
        lhs[30] = 1;
        let mut rhs = [0u8; SECRET_KEY_LENGTH];
        rhs[31] = 1;

        sub_assign_be(&mut lhs, &rhs);
        assert_eq!(lhs[30], 0);
        assert_eq!(lhs[31], 0xff);

        let mut value = [0xff; SECRET_KEY_LENGTH];
        add_one_be(&mut value);
        assert_eq!(value, [0u8; SECRET_KEY_LENGTH]);
    }
}
