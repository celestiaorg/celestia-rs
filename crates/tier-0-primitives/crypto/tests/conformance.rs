//! Conformance tests for the crypto crate against committed fixture files.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crypto::{bls12381, ed25519, secp256k1};
use serde::Deserialize;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..10 {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let body =
                std::fs::read_to_string(&manifest).expect("read candidate workspace Cargo.toml");
            if body.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    panic!("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR")
}

fn load_fixture<T>(relative_path: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let path = workspace_root()
        .join("conformance/fixtures")
        .join(relative_path);
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[derive(Deserialize)]
struct Ed25519Fixture {
    cases: Vec<Ed25519Case>,
    malformed_private_key_hex: String,
}

#[derive(Deserialize)]
struct Ed25519Case {
    label: String,
    seed_hex: String,
    secret_hex: String,
    private_key_hex: String,
    public_key_hex: String,
    message_hex: String,
    signature_hex: String,
    address_hex: String,
}

#[derive(Deserialize)]
struct Secp256k1Fixture {
    cases: Vec<Secp256k1Case>,
    invalid_secret_hex: Vec<String>,
}

#[derive(Deserialize)]
struct Secp256k1Case {
    label: String,
    secret_input_hex: String,
    secret_key_hex: String,
    public_key_hex: String,
    message_hex: String,
    signature_hex: String,
    high_s_signature_hex: String,
    address_hex: String,
}

#[derive(Deserialize)]
struct Bls12381Fixture {
    cipher_suite: String,
    cases: Vec<Bls12381Case>,
    aggregate: Bls12381Aggregate,
    invalid_public_key_hex: String,
    invalid_signature_hex: String,
}

#[derive(Deserialize)]
struct Bls12381Case {
    label: String,
    ikm_hex: String,
    secret_key_hex: String,
    public_key_hex: String,
    message_hex: String,
    signature_hex: String,
}

#[derive(Deserialize)]
struct Bls12381Aggregate {
    message_hex: String,
    public_keys_hex: Vec<String>,
    signatures_hex: Vec<String>,
    aggregate_public_key_hex: String,
    aggregate_signature_hex: String,
    fast_aggregate_verify: bool,
}

fn decode_vec(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn decode_array<const N: usize>(hex: &str) -> [u8; N] {
    let bytes = decode_vec(hex);
    assert_eq!(bytes.len(), N, "fixture byte length");
    bytes
        .try_into()
        .unwrap_or_else(|_| panic!("{N}-byte fixture"))
}

#[test]
fn conformance_ed25519() {
    let fixture: Ed25519Fixture = load_fixture("crypto/ed25519.json");

    for case in &fixture.cases {
        let seed = decode_array::<{ ed25519::SEED_LENGTH }>(&case.seed_hex);
        let secret = decode_vec(&case.secret_hex);
        let private_key = decode_array::<{ ed25519::PRIVATE_KEY_LENGTH }>(&case.private_key_hex);
        let public_key = decode_array::<{ ed25519::PUBLIC_KEY_LENGTH }>(&case.public_key_hex);
        let message = decode_vec(&case.message_hex);
        let signature = decode_array::<{ ed25519::SIGNATURE_LENGTH }>(&case.signature_hex);
        let address = decode_array::<{ ed25519::ADDRESS_LENGTH }>(&case.address_hex);

        assert_eq!(
            ed25519::private_key_from_seed(&seed),
            private_key,
            "seed-derived private key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            ed25519::private_key_from_secret(&secret),
            private_key,
            "secret-derived private key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            ed25519::public_key_from_private_key(&private_key).expect("well-formed private key"),
            public_key,
            "public key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            ed25519::sign(&private_key, &message).expect("sign message"),
            signature,
            "signature mismatch for '{}'",
            case.label
        );
        assert!(
            ed25519::verify(&public_key, &message, &signature),
            "signature verifies for '{}'",
            case.label
        );
        assert!(
            !ed25519::verify(&public_key, b"wrong message", &signature),
            "wrong message rejected for '{}'",
            case.label
        );
        assert_eq!(
            ed25519::address(&public_key),
            address,
            "address mismatch for '{}'",
            case.label
        );
    }

    let malformed_private_key =
        decode_array::<{ ed25519::PRIVATE_KEY_LENGTH }>(&fixture.malformed_private_key_hex);
    assert_eq!(
        ed25519::public_key_from_private_key(&malformed_private_key),
        Err(ed25519::Error::MalformedPrivateKey)
    );
    assert_eq!(
        ed25519::sign(&malformed_private_key, b"message"),
        Err(ed25519::Error::MalformedPrivateKey)
    );
}

#[test]
fn conformance_secp256k1() {
    let fixture: Secp256k1Fixture = load_fixture("crypto/secp256k1.json");

    for case in &fixture.cases {
        let secret_input = decode_vec(&case.secret_input_hex);
        let secret_key = decode_array::<{ secp256k1::SECRET_KEY_LENGTH }>(&case.secret_key_hex);
        let public_key = decode_array::<{ secp256k1::PUBLIC_KEY_LENGTH }>(&case.public_key_hex);
        let message = decode_vec(&case.message_hex);
        let signature = decode_array::<{ secp256k1::SIGNATURE_LENGTH }>(&case.signature_hex);
        let high_s_signature =
            decode_array::<{ secp256k1::SIGNATURE_LENGTH }>(&case.high_s_signature_hex);
        let address = decode_array::<{ secp256k1::ADDRESS_LENGTH }>(&case.address_hex);

        assert_eq!(
            secp256k1::secret_key_from_secret(&secret_input),
            secret_key,
            "secret-derived key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            secp256k1::secret_key_from_bytes(&secret_key).expect("valid secret key"),
            secret_key,
            "secret parse mismatch for '{}'",
            case.label
        );
        assert_eq!(
            secp256k1::public_key_from_secret_key(&secret_key).expect("derive public key"),
            public_key,
            "public key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            secp256k1::sign(&secret_key, &message).expect("sign message"),
            signature,
            "signature mismatch for '{}'",
            case.label
        );
        assert!(
            secp256k1::verify(&public_key, &message, &signature),
            "signature verifies for '{}'",
            case.label
        );
        assert!(
            !secp256k1::verify(&public_key, b"wrong message", &signature),
            "wrong message rejected for '{}'",
            case.label
        );
        assert!(
            !secp256k1::verify(&public_key, &message, &high_s_signature),
            "high-S signature rejected for '{}'",
            case.label
        );
        assert_eq!(
            secp256k1::address(&public_key),
            address,
            "address mismatch for '{}'",
            case.label
        );
    }

    for invalid_secret_hex in &fixture.invalid_secret_hex {
        let invalid_secret = decode_array::<{ secp256k1::SECRET_KEY_LENGTH }>(invalid_secret_hex);
        assert_eq!(
            secp256k1::secret_key_from_bytes(&invalid_secret),
            Err(secp256k1::Error::InvalidSecretKey)
        );
    }
}

#[test]
fn conformance_bls12381() {
    let fixture: Bls12381Fixture = load_fixture("crypto/bls12381.json");
    assert_eq!(fixture.cipher_suite, bls12381::CIPHERSUITE);

    for case in &fixture.cases {
        let ikm = decode_vec(&case.ikm_hex);
        let secret_key = decode_array::<{ bls12381::SECRET_KEY_LENGTH }>(&case.secret_key_hex);
        let public_key = decode_array::<{ bls12381::PUBLIC_KEY_LENGTH }>(&case.public_key_hex);
        let message = decode_vec(&case.message_hex);
        let signature = decode_array::<{ bls12381::SIGNATURE_LENGTH }>(&case.signature_hex);

        assert_eq!(
            bls12381::secret_key_from_ikm(&ikm).expect("derive secret key from ikm"),
            secret_key,
            "IKM-derived secret key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bls12381::secret_key_from_bytes(&secret_key).expect("valid secret key"),
            secret_key,
            "secret parse mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bls12381::public_key_from_secret_key(&secret_key).expect("derive public key"),
            public_key,
            "public key mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bls12381::sign(&secret_key, &message).expect("sign message"),
            signature,
            "signature mismatch for '{}'",
            case.label
        );
        assert!(
            bls12381::verify(&public_key, &message, &signature),
            "signature verifies for '{}'",
            case.label
        );
        assert!(
            !bls12381::verify(&public_key, b"wrong message", &signature),
            "wrong message rejected for '{}'",
            case.label
        );
    }

    let public_keys = fixture
        .aggregate
        .public_keys_hex
        .iter()
        .map(|key| decode_array::<{ bls12381::PUBLIC_KEY_LENGTH }>(key))
        .collect::<Vec<_>>();
    let signatures = fixture
        .aggregate
        .signatures_hex
        .iter()
        .map(|signature| decode_array::<{ bls12381::SIGNATURE_LENGTH }>(signature))
        .collect::<Vec<_>>();
    let aggregate_public_key = decode_array::<{ bls12381::PUBLIC_KEY_LENGTH }>(
        &fixture.aggregate.aggregate_public_key_hex,
    );
    let aggregate_signature =
        decode_array::<{ bls12381::SIGNATURE_LENGTH }>(&fixture.aggregate.aggregate_signature_hex);
    let aggregate_message = decode_vec(&fixture.aggregate.message_hex);

    assert_eq!(
        bls12381::aggregate_public_keys(&public_keys).expect("aggregate public keys"),
        aggregate_public_key
    );
    assert_eq!(
        bls12381::aggregate_signatures(&signatures).expect("aggregate signatures"),
        aggregate_signature
    );
    assert_eq!(
        bls12381::fast_aggregate_verify(&public_keys, &aggregate_message, &aggregate_signature),
        fixture.aggregate.fast_aggregate_verify
    );
    assert!(
        !bls12381::fast_aggregate_verify(&public_keys, b"wrong message", &aggregate_signature),
        "wrong aggregate message rejected"
    );
    assert_eq!(
        bls12381::aggregate_public_keys(&[]),
        Err(bls12381::Error::EmptyAggregate)
    );
    assert_eq!(
        bls12381::aggregate_signatures(&[]),
        Err(bls12381::Error::EmptyAggregate)
    );

    let invalid_public_key =
        decode_array::<{ bls12381::PUBLIC_KEY_LENGTH }>(&fixture.invalid_public_key_hex);
    let invalid_signature =
        decode_array::<{ bls12381::SIGNATURE_LENGTH }>(&fixture.invalid_signature_hex);
    assert!(!bls12381::verify(
        &invalid_public_key,
        &aggregate_message,
        &aggregate_signature
    ));
    assert!(!bls12381::verify(
        &aggregate_public_key,
        &aggregate_message,
        &invalid_signature
    ));
}
