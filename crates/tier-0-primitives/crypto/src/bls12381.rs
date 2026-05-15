//! BLS12-381 signature helpers.

use blst::BLST_ERROR;
use blst::min_pk::{AggregatePublicKey, AggregateSignature, PublicKey, SecretKey, Signature};

/// BLS signature ciphersuite used for Blobstream-compatible fixtures.
pub const CIPHERSUITE: &str = "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// Size in bytes of a BLS12-381 scalar.
pub const SECRET_KEY_LENGTH: usize = 32;

/// Size in bytes of a compressed G1 public key.
pub const PUBLIC_KEY_LENGTH: usize = 48;

/// Size in bytes of a compressed G2 signature.
pub const SIGNATURE_LENGTH: usize = 96;

/// A 32-byte BLS12-381 secret scalar.
pub type SecretKeyBytes = [u8; SECRET_KEY_LENGTH];

/// A compressed G1 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A compressed G2 signature.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// Errors returned by BLS12-381 helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret key is zero or outside the BLS12-381 scalar field.
    InvalidSecretKey,
    /// The public key is not a valid compressed G1 point in the prime-order subgroup.
    InvalidPublicKey,
    /// The signature is not a valid compressed G2 point in the prime-order subgroup.
    InvalidSignature,
    /// Aggregation was requested for an empty input list.
    EmptyAggregate,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSecretKey => f.write_str("invalid BLS12-381 secret key"),
            Self::InvalidPublicKey => f.write_str("invalid BLS12-381 public key"),
            Self::InvalidSignature => f.write_str("invalid BLS12-381 signature"),
            Self::EmptyAggregate => f.write_str("empty BLS12-381 aggregate"),
        }
    }
}

impl std::error::Error for Error {}

/// Derive a BLS12-381 secret key from input key material.
pub fn secret_key_from_ikm(ikm: &[u8]) -> Result<SecretKeyBytes, Error> {
    SecretKey::key_gen(ikm, &[])
        .map(|key| key.to_bytes())
        .map_err(|_| Error::InvalidSecretKey)
}

/// Parse a raw 32-byte BLS12-381 secret key.
pub fn secret_key_from_bytes(bytes: &SecretKeyBytes) -> Result<SecretKeyBytes, Error> {
    SecretKey::from_bytes(bytes)
        .map(|_| *bytes)
        .map_err(|_| Error::InvalidSecretKey)
}

/// Derive the compressed G1 public key for `secret_key`.
pub fn public_key_from_secret_key(secret_key: &SecretKeyBytes) -> Result<PublicKeyBytes, Error> {
    let secret_key = parse_secret_key(secret_key)?;
    Ok(secret_key.sk_to_pk().compress())
}

/// Sign `msg` into a compressed G2 signature.
pub fn sign(secret_key: &SecretKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error> {
    let secret_key = parse_secret_key(secret_key)?;
    Ok(secret_key.sign(msg, CIPHERSUITE.as_bytes(), &[]).compress())
}

/// Verify a compressed G2 signature against a compressed G1 public key.
#[must_use]
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool {
    let Ok(public_key) = parse_public_key(public_key) else {
        return false;
    };
    let Ok(signature) = parse_signature(signature) else {
        return false;
    };
    signature.verify(true, msg, CIPHERSUITE.as_bytes(), &[], &public_key, true)
        == BLST_ERROR::BLST_SUCCESS
}

/// Aggregate compressed G1 public keys.
pub fn aggregate_public_keys(public_keys: &[PublicKeyBytes]) -> Result<PublicKeyBytes, Error> {
    if public_keys.is_empty() {
        return Err(Error::EmptyAggregate);
    }
    let parsed = public_keys
        .iter()
        .map(parse_public_key)
        .collect::<Result<Vec<_>, _>>()?;
    let refs = parsed.iter().collect::<Vec<_>>();
    AggregatePublicKey::aggregate(&refs, false)
        .map(|key| key.to_public_key().compress())
        .map_err(|_| Error::InvalidPublicKey)
}

/// Aggregate compressed G2 signatures.
pub fn aggregate_signatures(signatures: &[SignatureBytes]) -> Result<SignatureBytes, Error> {
    if signatures.is_empty() {
        return Err(Error::EmptyAggregate);
    }
    let parsed = signatures
        .iter()
        .map(parse_signature)
        .collect::<Result<Vec<_>, _>>()?;
    let refs = parsed.iter().collect::<Vec<_>>();
    AggregateSignature::aggregate(&refs, false)
        .map(|signature| signature.to_signature().compress())
        .map_err(|_| Error::InvalidSignature)
}

/// Fast-aggregate-verify signatures over one common message.
#[must_use]
pub fn fast_aggregate_verify(
    public_keys: &[PublicKeyBytes],
    msg: &[u8],
    signature: &SignatureBytes,
) -> bool {
    if public_keys.is_empty() {
        return false;
    }
    let Ok(signature) = parse_signature(signature) else {
        return false;
    };
    let Ok(parsed) = public_keys
        .iter()
        .map(parse_public_key)
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let refs = parsed.iter().collect::<Vec<_>>();
    signature.fast_aggregate_verify(true, msg, CIPHERSUITE.as_bytes(), &refs)
        == BLST_ERROR::BLST_SUCCESS
}

fn parse_secret_key(secret_key: &SecretKeyBytes) -> Result<SecretKey, Error> {
    SecretKey::from_bytes(secret_key).map_err(|_| Error::InvalidSecretKey)
}

fn parse_public_key(public_key: &PublicKeyBytes) -> Result<PublicKey, Error> {
    PublicKey::key_validate(public_key).map_err(|_| Error::InvalidPublicKey)
}

fn parse_signature(signature: &SignatureBytes) -> Result<Signature, Error> {
    Signature::sig_validate(signature, true).map_err(|_| Error::InvalidSignature)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages_are_stable() {
        assert_eq!(
            Error::InvalidSecretKey.to_string(),
            "invalid BLS12-381 secret key"
        );
        assert_eq!(
            Error::InvalidPublicKey.to_string(),
            "invalid BLS12-381 public key"
        );
        assert_eq!(
            Error::InvalidSignature.to_string(),
            "invalid BLS12-381 signature"
        );
        assert_eq!(
            Error::EmptyAggregate.to_string(),
            "empty BLS12-381 aggregate"
        );
    }

    #[test]
    fn rejects_invalid_inputs() {
        let invalid_secret_key = [0u8; SECRET_KEY_LENGTH];
        let invalid_public_key = [0u8; PUBLIC_KEY_LENGTH];
        let invalid_signature = [0u8; SIGNATURE_LENGTH];

        assert_eq!(
            secret_key_from_ikm(b"too short"),
            Err(Error::InvalidSecretKey)
        );
        assert_eq!(
            secret_key_from_bytes(&invalid_secret_key),
            Err(Error::InvalidSecretKey)
        );
        assert_eq!(
            public_key_from_secret_key(&invalid_secret_key),
            Err(Error::InvalidSecretKey)
        );
        assert_eq!(
            sign(&invalid_secret_key, b"message"),
            Err(Error::InvalidSecretKey)
        );
        assert_eq!(
            aggregate_public_keys(&[invalid_public_key]),
            Err(Error::InvalidPublicKey)
        );
        assert_eq!(
            aggregate_signatures(&[invalid_signature]),
            Err(Error::InvalidSignature)
        );
        assert!(!fast_aggregate_verify(&[], b"message", &invalid_signature));
        assert!(!fast_aggregate_verify(
            &[invalid_public_key],
            b"message",
            &invalid_signature
        ));

        let secret_key = secret_key_from_ikm(&[42u8; 32]).expect("valid BLS IKM");
        let valid_signature = sign(&secret_key, b"message").expect("valid BLS secret key");
        assert!(!fast_aggregate_verify(
            &[invalid_public_key],
            b"message",
            &valid_signature
        ));
    }
}
