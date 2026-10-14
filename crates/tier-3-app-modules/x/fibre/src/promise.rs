//! Payment promises (celestia-app v10 `fibre/payment_promise.go`) and the
//! signature checks the chain applies to them.
//!
//! Sign bytes are CometBFT's `RawBytesMessageSignBytes(chain_id,
//! "fibre/pp:v0", stripped)` where `stripped` is
//! `signer(33) || namespace(29) || u32be(blob_size) || commitment(32) ||
//! u32be(blob_version) || u64be(height) || time.MarshalBinary(creation)`.

use celestia_proto::celestia::fibre::v1::PaymentPromise;
use pbjson_types::Timestamp;

use crate::ts_or_zero;

/// `signBytesPrefix`.
pub const SIGN_BYTES_PREFIX: &str = "fibre/pp:v0";
/// CometBFT `RawBytesSignBytesPrefix`.
pub const RAW_BYTES_SIGN_BYTES_PREFIX: &str = "COMET::RAW_BYTES::SIGN";
/// `maxChainIDSize`.
pub const MAX_CHAIN_ID_SIZE: usize = 20;
/// `secp256k1.PubKeySize`.
pub const SECP256K1_PUBKEY_SIZE: usize = 33;
/// `CommitmentSize`.
pub const COMMITMENT_SIZE: usize = 32;
/// `ed25519.SignatureSize`.
pub const ED25519_SIGNATURE_SIZE: usize = 64;

/// Seconds between Go's zero time (year 1) and the Unix epoch.
const UNIX_TO_INTERNAL: i64 = 62_135_596_800;

/// Go `time.Time.MarshalBinary` of a UTC time (version 1, offset -1).
#[must_use]
pub fn time_marshal_binary_utc(t: &Timestamp) -> [u8; 15] {
    let sec = t.seconds.wrapping_add(UNIX_TO_INTERNAL);
    let mut out = [0u8; 15];
    out[0] = 1;
    out[1..9].copy_from_slice(&sec.to_be_bytes());
    out[9..13].copy_from_slice(&t.nanos.to_be_bytes());
    out[13..15].copy_from_slice(&(-1i16).to_be_bytes());
    out
}

fn put_uvarint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push((v & 0x7f) as u8);
}

fn put_bytes_field(out: &mut Vec<u8>, tag: u8, b: &[u8]) {
    if b.is_empty() {
        return;
    }
    out.push(tag);
    put_uvarint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// CometBFT `types.RawBytesMessageSignBytes(chainID, uniqueID, rawBytes)`.
pub fn raw_bytes_message_sign_bytes(
    chain_id: &str,
    unique_id: &str,
    raw: &[u8],
) -> Result<Vec<u8>, String> {
    if chain_id.is_empty() {
        return Err("chainID cannot be empty".into());
    }
    if unique_id.is_empty() {
        return Err("uniqueID cannot be empty".into());
    }
    if raw.is_empty() {
        return Err("rawBytes cannot be empty".into());
    }
    // privval.SignRawBytesRequest{chain_id=1, raw_bytes=2, unique_id=3},
    // length-delimited.
    let mut body = Vec::new();
    put_bytes_field(&mut body, 0x0a, chain_id.as_bytes());
    put_bytes_field(&mut body, 0x12, raw);
    put_bytes_field(&mut body, 0x1a, unique_id.as_bytes());
    let mut out = RAW_BYTES_SIGN_BYTES_PREFIX.as_bytes().to_vec();
    put_uvarint(&mut out, body.len() as u64);
    out.extend_from_slice(&body);
    Ok(out)
}

/// `PaymentPromise.FromProto`: the namespace and commitment checks.
pub fn from_proto_check(p: &PaymentPromise) -> Result<(), String> {
    shares::Namespace::from_slice(&p.namespace).map_err(|e| format!("invalid namespace: {e}"))?;
    if p.commitment.len() != COMMITMENT_SIZE {
        return Err(format!(
            "commitment must be {COMMITMENT_SIZE} bytes, got {}",
            p.commitment.len()
        ));
    }
    Ok(())
}

/// The signer's compressed public key (`SignerPublicKey.Key`).
#[must_use]
pub fn signer_key(p: &PaymentPromise) -> &[u8] {
    p.signer_public_key
        .as_ref()
        .map_or(&[], |k| k.key.as_slice())
}

/// `strippedSignBytes`.
#[must_use]
pub fn stripped_sign_bytes(p: &PaymentPromise) -> Vec<u8> {
    let mut buf = Vec::with_capacity(SECP256K1_PUBKEY_SIZE + 29 + 4 + 32 + 4 + 8 + 15);
    buf.extend_from_slice(signer_key(p));
    buf.extend_from_slice(&p.namespace);
    buf.extend_from_slice(&p.blob_size.to_be_bytes());
    buf.extend_from_slice(&p.commitment);
    buf.extend_from_slice(&p.blob_version.to_be_bytes());
    #[allow(clippy::cast_sign_loss)]
    buf.extend_from_slice(&(p.height as u64).to_be_bytes());
    buf.extend_from_slice(&time_marshal_binary_utc(&ts_or_zero(p.creation_timestamp)));
    buf
}

/// `PaymentPromise.SignBytes`.
pub fn sign_bytes(p: &PaymentPromise) -> Result<Vec<u8>, String> {
    raw_bytes_message_sign_bytes(&p.chain_id, SIGN_BYTES_PREFIX, &stripped_sign_bytes(p))
}

/// `PaymentPromise.Hash`: `sha256(sign_bytes || signature)`.
pub fn hash(p: &PaymentPromise) -> Result<Vec<u8>, String> {
    if p.signature.is_empty() {
        return Err("signature must be set before computing hash".into());
    }
    let sb = sign_bytes(p).map_err(|e| format!("getting sign bytes: {e}"))?;
    let mut h = hash::Sha256Hasher::new();
    h.update(&sb);
    h.update(&p.signature);
    Ok(h.finalize().to_vec())
}

/// `promiseHash`: `FromProto` + `Hash`.
pub fn promise_hash(p: &PaymentPromise) -> Result<Vec<u8>, String> {
    from_proto_check(p)?;
    hash(p)
}

/// `PaymentPromise.Validate` (stateless, including the signer signature).
pub fn validate(p: &PaymentPromise) -> Result<(), String> {
    let key = signer_key(p);
    if key.len() != SECP256K1_PUBKEY_SIZE {
        return Err(format!(
            "signer key must be {SECP256K1_PUBKEY_SIZE} bytes, got {}",
            key.len()
        ));
    }
    if p.chain_id.is_empty() {
        return Err("chain id must not be empty".into());
    }
    if p.chain_id.len() > MAX_CHAIN_ID_SIZE {
        return Err(format!(
            "chain id length {} exceeds maximum {MAX_CHAIN_ID_SIZE}",
            p.chain_id.len()
        ));
    }
    if p.blob_size == 0 {
        return Err("upload size must be positive".into());
    }
    if sdk::time::is_zero(&ts_or_zero(p.creation_timestamp)) {
        return Err("creation timestamp must not be zero".into());
    }
    if p.signature.len() != 64 {
        return Err(format!(
            "signature must be 64 bytes, got {}",
            p.signature.len()
        ));
    }
    if p.height == 0 {
        return Err(format!("height must be positive, got {}", p.height));
    }
    let sb = sign_bytes(p).map_err(|e| format!("building sign bytes: {e}"))?;
    if !verify_secp256k1(key, &sb, &p.signature) {
        return Err("signature verification failed".into());
    }
    Ok(())
}

/// `FromProto` + `Validate`, with Go's error wrapping in the Msg server.
pub fn from_proto_and_validate(p: &PaymentPromise) -> sdk::Result<()> {
    from_proto_check(p).map_err(|e| {
        sdk::error::sdk::invalid_request(format!("failed to convert payment promise: {e}"))
    })?;
    validate(p).map_err(|e| {
        sdk::error::sdk::invalid_request(format!("payment promise validation failed: {e}"))
    })
}

/// secp256k1 `PubKey.VerifySignature` (cosmos-sdk `secp256k1_nocgo.go`):
/// `r` and `s` reduced modulo the group order, high-S rejected.
#[must_use]
pub fn verify_secp256k1(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    use k256::elliptic_curve::ops::Reduce;
    use k256::elliptic_curve::scalar::IsHigh;
    use k256::{FieldBytes, Scalar, U256};
    if sig.len() != 64 {
        return false;
    }
    let Ok(vk) = k256::ecdsa::VerifyingKey::from_sec1_bytes(key) else {
        return false;
    };
    let r = <Scalar as Reduce<U256>>::reduce_bytes(FieldBytes::from_slice(&sig[..32]));
    let s = <Scalar as Reduce<U256>>::reduce_bytes(FieldBytes::from_slice(&sig[32..]));
    if bool::from(s.is_high()) {
        return false;
    }
    let Ok(signature) = k256::ecdsa::Signature::from_scalars(r, s) else {
        return false;
    };
    let digest = hash::sha256(msg);
    k256::ecdsa::signature::hazmat::PrehashVerifier::verify_prehash(&vk, &digest, &signature)
        .is_ok()
}

/// Go standard library `crypto/ed25519.Verify` (RFC 8032 cofactorless
/// check with canonical `s`, as used by the fibre `SignatureSet`).
#[must_use]
pub fn verify_ed25519_go(public_key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    use curve25519_dalek_ng::edwards::{CompressedEdwardsY, EdwardsPoint};
    use curve25519_dalek_ng::scalar::Scalar;
    if public_key.len() != 32 || sig.len() != ED25519_SIGNATURE_SIZE || sig[63] & 0b1110_0000 != 0 {
        return false;
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(public_key);
    let Some(a_point) = CompressedEdwardsY(a).decompress() else {
        return false;
    };
    let mut s = [0u8; 32];
    s.copy_from_slice(&sig[32..]);
    let Some(s) = Scalar::from_canonical_bytes(s) else {
        return false;
    };
    let mut input = Vec::with_capacity(64 + msg.len());
    input.extend_from_slice(&sig[..32]);
    input.extend_from_slice(public_key);
    input.extend_from_slice(msg);
    let k = Scalar::from_bytes_mod_order_wide(&hash::sha512(&input));
    let r = EdwardsPoint::vartime_double_scalar_mul_basepoint(&k, &-a_point, &s);
    r.compress().as_bytes() == &sig[..32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_binary_matches_go() {
        // time.Date(2026, 10, 14, 5, 29, 15, 701265606, time.UTC).MarshalBinary()
        let t = Timestamp {
            seconds: 1_791_955_755,
            nanos: 701_265_606,
        };
        assert_eq!(
            hex::encode(time_marshal_binary_utc(&t)),
            "010000000ee2610a2b29cc76c6ffff"
        );
    }

    #[test]
    fn raw_bytes_sign_bytes_layout() {
        let sb = raw_bytes_message_sign_bytes("c", "u", b"r").unwrap_or_default();
        assert_eq!(
            sb,
            b"COMET::RAW_BYTES::SIGN\x09\x0a\x01c\x12\x01r\x1a\x01u".to_vec()
        );
        assert!(raw_bytes_message_sign_bytes("", "u", b"r").is_err());
    }

    /// Output of `conformance/fibre-client` (an independent Go
    /// implementation of the sign bytes, with Go's `time.MarshalBinary`,
    /// decred `SignCompact` and `crypto/ed25519`).
    #[test]
    fn promise_matches_go_fibre_client() {
        use base64::Engine as _;
        let b64 = |s: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .unwrap_or_default()
        };
        let p = PaymentPromise {
            chain_id: "test-chain".into(),
            height: 10,
            namespace: b64("AAAAAAAAAAAAAAAAAAAAAAAAAKoAAAAAAAAAAAA="),
            blob_size: 1000,
            blob_version: 0,
            commitment: vec![5; 32],
            creation_timestamp: Some(Timestamp {
                seconds: 1_800_000_000,
                nanos: 123_456_789,
            }),
            signer_public_key: Some(celestia_proto::cosmos::crypto::secp256k1::PubKey {
                key: b64("AxuExVZ7EmRAmV0+1aq6BWXXHhg0YEgZ/5wX9enV3QeP"),
            }),
            signature: b64(
                "w49suMaACdS8YYOnl2p+sj2Xnv+z0WIY9JY6DhgEWgZVGWw7bhhBIpr2ChpfviXjtmULDkSNLCGF+jAtxcSNNw==",
            ),
        };
        assert_eq!(
            hex::encode(hash(&p).unwrap_or_default()),
            "0e682408b3b883222026520eb3d9e8fe1d15bbc28ef8c3f9828d46b58f5e5750"
        );
        assert_eq!(validate(&p), Ok(()));
        let val = hex::decode("d868e076bf3f9bdd6a2e5543050cb4d44647a30b54ee16797f71ae7374a1bf13")
            .unwrap_or_default();
        let sig = hex::decode(
            "0ecc69a8469b7759aa37b88cca9c288088dacb1688b108a18623dd7eb64f596456d80dd94d59c8e0c735427213d2fe2a83098665e8685b7c3794163a5fdfe904",
        )
        .unwrap_or_default();
        let sb = sign_bytes(&p).unwrap_or_default();
        assert!(verify_ed25519_go(&val, &sb, &sig));
        let mut tampered = p;
        tampered.blob_size = 1001;
        assert!(validate(&tampered).is_err());
    }

    #[test]
    fn go_ed25519_accepts_valid_and_rejects_tampered() {
        let pk = crypto::ed25519::private_key_from_seed(&[7u8; 32]);
        let pubk = crypto::ed25519::public_key_from_private_key(&pk).unwrap_or([0; 32]);
        let sig = crypto::ed25519::sign(&pk, b"msg").unwrap_or([0; 64]);
        assert!(verify_ed25519_go(&pubk, b"msg", &sig));
        assert!(!verify_ed25519_go(&pubk, b"msh", &sig));
        let mut bad = sig;
        bad[63] |= 0x80;
        assert!(!verify_ed25519_go(&pubk, b"msg", &bad));
    }
}
