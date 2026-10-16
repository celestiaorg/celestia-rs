//! Cosmos SDK / CometBFT secp256k1 signature helpers (libsecp256k1).

use std::sync::LazyLock;

use ::secp256k1::{All, Message, PublicKey, Secp256k1, SecretKey, ecdsa};
use hash::{ripemd160, sha256};

/// One libsecp256k1 context for signing and verification.
static CONTEXT: LazyLock<Secp256k1<All>> = LazyLock::new(Secp256k1::new);

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

/// The group order `n`, big-endian.
const ORDER: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];

/// `n / 2` (rounded down), big-endian: decred's `IsOverHalfOrder` bound.
const HALF_ORDER: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
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

/// Parse a raw 32-byte secp256k1 secret key (`0 < key < n`).
pub fn secret_key_from_bytes(bytes: &SecretKeyBytes) -> Result<SecretKeyBytes, Error> {
    SecretKey::from_byte_array(*bytes)
        .map(|_| *bytes)
        .map_err(|_| Error::InvalidSecretKey)
}

/// Derive the compressed secp256k1 public key for `secret_key`.
pub fn public_key_from_secret_key(secret_key: &SecretKeyBytes) -> Result<PublicKeyBytes, Error> {
    let secret_key =
        SecretKey::from_byte_array(*secret_key).map_err(|_| Error::InvalidSecretKey)?;
    Ok(PublicKey::from_secret_key(&CONTEXT, &secret_key).serialize())
}

/// Sign SHA-256(`msg`): RFC 6979 nonce, lower-S, `r || s` (cosmos-sdk's
/// and CometBFT's `PrivKey.Sign`, decred `ecdsa.SignCompact`).
pub fn sign(secret_key: &SecretKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error> {
    let secret_key =
        SecretKey::from_byte_array(*secret_key).map_err(|_| Error::InvalidSecretKey)?;
    let message = Message::from_digest(sha256(msg));
    Ok(CONTEXT.sign_ecdsa(message, &secret_key).serialize_compact())
}

/// CometBFT's secp256k1 `PubKey.VerifySignature` (celestia-core v0.42.3
/// `crypto/secp256k1`, decred secp256k1 v4) for a 33-byte key: votes,
/// commits, and ibc-go's 07-tendermint light client.
///
/// CometBFT reduces `r` / `s` modulo `n` (`ModNScalar.SetByteSlice`),
/// rejects a high `s` by comparing the signature with its DER round trip
/// (`Serialize` negates a high `s`, `ParseDERSignature` rejects zero `r` /
/// `s`) and verifies with `ecdsa.Signature.Verify` on SHA-256(`msg`). That
/// is exactly cosmos-sdk's rule set, and its keys are those of
/// `secp256k1.ParsePubKey` (33 bytes: `02` / `03`, `x < p`, on the curve),
/// so this is [`verify_go`]; `conformance/fixtures/secp256k1/verify_cometbft.json`
/// carries CometBFT's own results for every edge case.
#[must_use]
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool {
    verify_go(public_key, msg, signature)
}

/// cosmos-sdk `secp256k1.PubKey.VerifySignature` (`secp256k1_nocgo.go`,
/// decred secp256k1 v4) of `msg` for the public key bytes `key` and the
/// signature bytes `sig`. See [`verify_go_prehashed`].
#[must_use]
pub fn verify_go(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    sig.len() == SIGNATURE_LENGTH && verify_go_prehashed(key, &sha256(msg), sig)
}

/// [`verify_go`] of a message whose SHA-256 digest is `digest`.
///
/// Go's rules, each one checked against Go by the vectors in
/// `conformance/fixtures/secp256k1/verify.json`:
/// - `sig` is exactly 64 bytes `r ‖ s`; `r` and `s` are reduced modulo
///   the group order (`ModNScalar.SetByteSlice` ignores the overflow), so
///   `r + n` is accepted where it fits in 32 bytes and `n` reads as zero;
/// - a reduced `s` above `n / 2` is rejected (low-S), zero `r` / `s` too;
/// - `key` is anything `secp256k1.ParsePubKey` accepts: 33-byte compressed
///   (`02` / `03`), 65-byte uncompressed (`04`) or hybrid (`06` / `07`,
///   parity must match), coordinates below `p` and on the curve;
/// - the digest is reduced modulo `n`, and `R.x` is compared with `r` and
///   with `r + n` (when below `p`).
///
/// libsecp256k1 implements exactly these key encodings and the same
/// verification equation; it rejects overflowing `r` / `s` instead of
/// reducing them, so they are reduced here first.
#[must_use]
pub fn verify_go_prehashed(key: &[u8], digest: &[u8; 32], sig: &[u8]) -> bool {
    if sig.len() != SIGNATURE_LENGTH {
        return false;
    }
    let mut compact = [0u8; SIGNATURE_LENGTH];
    compact[..32].copy_from_slice(&reduce_mod_order(&sig[..32]));
    compact[32..].copy_from_slice(&reduce_mod_order(&sig[32..]));
    let (r, s) = compact.split_at(32);
    if s > HALF_ORDER.as_slice() || r.iter().all(|&b| b == 0) || s.iter().all(|&b| b == 0) {
        return false;
    }
    let Ok(public_key) = PublicKey::from_slice(key) else {
        return false;
    };
    let Ok(signature) = ecdsa::Signature::from_compact(&compact) else {
        return false;
    };
    let message = Message::from_digest(*digest);
    CONTEXT
        .verify_ecdsa(message, &signature, &public_key)
        .is_ok()
}

/// A 32-byte big-endian integer modulo `n` (decred
/// `ModNScalar.SetByteSlice`). `2^256 - 1 < 2n`, so one subtraction does.
fn reduce_mod_order(bytes: &[u8]) -> [u8; 32] {
    let mut value = [0u8; 32];
    value.copy_from_slice(bytes);
    if value >= ORDER {
        sub_assign_be(&mut value, &ORDER);
    }
    value
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
    fn reduce_mod_order_matches_go() {
        let mut ff = [0xff; 32];
        // 2^256 - 1 - n
        let mut expected = [0u8; 32];
        expected[15] = 1;
        expected[16..].copy_from_slice(&[
            0x45, 0x51, 0x23, 0x19, 0x50, 0xb7, 0x5f, 0xc4, 0x40, 0x2d, 0xa1, 0x73, 0x2f, 0xc9,
            0xbe, 0xbe,
        ]);
        assert_eq!(reduce_mod_order(&ff), expected);
        assert_eq!(reduce_mod_order(&ORDER), [0u8; 32]);
        assert_eq!(reduce_mod_order(&HALF_ORDER), HALF_ORDER);
        ff = ORDER;
        ff[31] -= 1;
        assert_eq!(reduce_mod_order(&ff), ff);
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
