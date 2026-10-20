//! `CometBFT` Ed25519 consensus signature helpers.

use ed25519_consensus::{Signature, SigningKey, VerificationKey};
use hash::sha256;

/// Size in bytes of a `CometBFT` Ed25519 private-key seed.
pub const SEED_LENGTH: usize = 32;

/// Size in bytes of a `CometBFT` Ed25519 private key.
pub const PRIVATE_KEY_LENGTH: usize = 64;

/// Size in bytes of a `CometBFT` Ed25519 public key.
pub const PUBLIC_KEY_LENGTH: usize = 32;

/// Size in bytes of an Ed25519 signature.
pub const SIGNATURE_LENGTH: usize = 64;

/// Size in bytes of a `CometBFT` public-key address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte Ed25519 private-key seed.
pub type SeedBytes = [u8; SEED_LENGTH];

/// A 64-byte `CometBFT` Ed25519 private key, encoded as `seed || public_key`.
pub type PrivateKeyBytes = [u8; PRIVATE_KEY_LENGTH];

/// A 32-byte Ed25519 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A 64-byte Ed25519 signature.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// A 20-byte `CometBFT` public-key address.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by Ed25519 helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The private key was not encoded as `seed || public_key`.
    #[error("malformed Ed25519 private key")]
    MalformedPrivateKey,
    /// The public key could not be parsed by the ZIP-215 verifier.
    #[error("malformed Ed25519 public key")]
    MalformedPublicKey,
    /// The signature could not be parsed.
    #[error("malformed Ed25519 signature")]
    MalformedSignature,
}

/// Derive the `CometBFT` Ed25519 private key from a 32-byte seed.
#[must_use]
pub fn private_key_from_seed(seed: &SeedBytes) -> PrivateKeyBytes {
    let signing_key = SigningKey::from(*seed);
    let public_key = signing_key.verification_key().to_bytes();
    let mut private_key = [0u8; PRIVATE_KEY_LENGTH];
    private_key[..SEED_LENGTH].copy_from_slice(seed);
    private_key[SEED_LENGTH..].copy_from_slice(&public_key);
    private_key
}

/// Derive the `CometBFT` Ed25519 private key from an arbitrary secret.
#[must_use]
pub fn private_key_from_secret(secret: &[u8]) -> PrivateKeyBytes {
    private_key_from_seed(&sha256(secret))
}

/// Return the public key embedded in a well-formed `CometBFT` private key.
pub fn public_key_from_private_key(private_key: &PrivateKeyBytes) -> Result<PublicKeyBytes, Error> {
    let mut seed = [0u8; SEED_LENGTH];
    seed.copy_from_slice(&private_key[..SEED_LENGTH]);
    let expected = private_key_from_seed(&seed);
    if expected != *private_key {
        return Err(Error::MalformedPrivateKey);
    }
    let mut public_key = [0u8; PUBLIC_KEY_LENGTH];
    public_key.copy_from_slice(&private_key[SEED_LENGTH..]);
    Ok(public_key)
}

/// Sign `msg` with a `CometBFT` Ed25519 private key.
pub fn sign(private_key: &PrivateKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error> {
    public_key_from_private_key(private_key)?;
    let mut seed = [0u8; SEED_LENGTH];
    seed.copy_from_slice(&private_key[..SEED_LENGTH]);
    Ok(SigningKey::from(seed).sign(msg).to_bytes())
}

/// Verify `signature` using `CometBFT` ZIP-215 Ed25519 semantics.
#[must_use]
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool {
    let Ok(verification_key) = VerificationKey::try_from(*public_key) else {
        return false;
    };
    let signature = Signature::from(*signature);
    verification_key.verify(&signature, msg).is_ok()
}

/// Return `CometBFT`'s Ed25519 address: first 20 bytes of SHA-256(public_key).
#[must_use]
pub fn address(public_key: &PublicKeyBytes) -> AddressBytes {
    let digest = sha256(public_key);
    let mut address = [0u8; ADDRESS_LENGTH];
    address.copy_from_slice(&digest[..ADDRESS_LENGTH]);
    address
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages_are_stable() {
        assert_eq!(
            Error::MalformedPrivateKey.to_string(),
            "malformed Ed25519 private key"
        );
        assert_eq!(
            Error::MalformedPublicKey.to_string(),
            "malformed Ed25519 public key"
        );
        assert_eq!(
            Error::MalformedSignature.to_string(),
            "malformed Ed25519 signature"
        );
    }

    #[test]
    fn verify_rejects_malformed_public_key() {
        let public_key = [0xff; PUBLIC_KEY_LENGTH];
        let signature = [0u8; SIGNATURE_LENGTH];

        assert!(!verify(&public_key, b"message", &signature));
    }
}
