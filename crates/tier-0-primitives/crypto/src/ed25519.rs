//! `CometBFT` Ed25519 consensus signature helpers.

use curve25519_dalek_ng::edwards::CompressedEdwardsY;
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
    /// An authority key used a non-canonical point encoding.
    #[error("ed25519 authority key is not canonical")]
    NonCanonicalAuthorityKey,
    /// An authority key did not encode a point on the curve.
    #[error("ed25519 authority key is not on the curve")]
    AuthorityKeyNotOnCurve,
    /// An authority key encoded a small-order point.
    #[error("ed25519 authority key has small order")]
    SmallOrderAuthorityKey,
    /// An authority key was not in the prime-order subgroup.
    #[error("ed25519 authority key is not in the prime-order subgroup")]
    NonPrimeOrderAuthorityKey,
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

/// Validate an Ed25519 key before granting it authority over a new identity.
///
/// Signature verification intentionally retains CometBFT's ZIP-215 behavior
/// for historical consensus data. New authority keys must additionally use a
/// canonical encoding of a non-small-order point in the prime-order subgroup.
pub fn validate_authority_key(public_key: &PublicKeyBytes) -> Result<(), Error> {
    let compressed = CompressedEdwardsY(*public_key);
    let Some(point) = compressed.decompress() else {
        return Err(Error::AuthorityKeyNotOnCurve);
    };
    if point.compress().to_bytes() != *public_key {
        return Err(Error::NonCanonicalAuthorityKey);
    }
    if point.is_small_order() {
        return Err(Error::SmallOrderAuthorityKey);
    }
    if !point.is_torsion_free() {
        return Err(Error::NonPrimeOrderAuthorityKey);
    }
    Ok(())
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
        assert_eq!(
            Error::NonCanonicalAuthorityKey.to_string(),
            "ed25519 authority key is not canonical"
        );
        assert_eq!(
            Error::AuthorityKeyNotOnCurve.to_string(),
            "ed25519 authority key is not on the curve"
        );
        assert_eq!(
            Error::SmallOrderAuthorityKey.to_string(),
            "ed25519 authority key has small order"
        );
        assert_eq!(
            Error::NonPrimeOrderAuthorityKey.to_string(),
            "ed25519 authority key is not in the prime-order subgroup"
        );
    }

    #[test]
    fn verify_rejects_malformed_public_key() {
        let public_key = [0xff; PUBLIC_KEY_LENGTH];
        let signature = [0u8; SIGNATURE_LENGTH];

        assert!(!verify(&public_key, b"message", &signature));
    }

    #[test]
    fn authority_keys_match_core_v0428() {
        for seed in 0..64 {
            let private_key = private_key_from_seed(&[seed; SEED_LENGTH]);
            let public_key = public_key_from_private_key(&private_key).unwrap();
            assert_eq!(validate_authority_key(&public_key), Ok(()));
        }

        for (encoded, expected) in [
            (
                "0100000000000000000000000000000000000000000000000000000000000000",
                Error::SmallOrderAuthorityKey,
            ),
            (
                "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
                Error::SmallOrderAuthorityKey,
            ),
            (
                "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
                Error::SmallOrderAuthorityKey,
            ),
            (
                "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
                Error::NonCanonicalAuthorityKey,
            ),
            (
                "0100000000000000000000000000000000000000000000000000000000000080",
                Error::NonCanonicalAuthorityKey,
            ),
            (
                "0b485cfc0eecc619440448436f8fc9df40566f2369e72400281454cb552afb52",
                Error::AuthorityKeyNotOnCurve,
            ),
            (
                "d03785ba6fb4990c5b4c743b53fc0ba2eb8b6ee2886e6c776eac2e1e7cb42cc4",
                Error::NonPrimeOrderAuthorityKey,
            ),
        ] {
            let key = hex::decode(encoded).unwrap().try_into().unwrap();
            assert_eq!(validate_authority_key(&key), Err(expected), "{encoded}");
        }
    }

    #[test]
    fn authority_admission_does_not_change_zip215_verification() {
        let mut key = [0; PUBLIC_KEY_LENGTH];
        key[0] = 1;
        let mut signature = [0; SIGNATURE_LENGTH];
        signature[0] = 1;
        assert_eq!(
            validate_authority_key(&key),
            Err(Error::SmallOrderAuthorityKey)
        );
        for message in [b"vote".as_slice(), b"another vote".as_slice()] {
            assert!(verify(&key, message, &signature));
        }
    }
}
