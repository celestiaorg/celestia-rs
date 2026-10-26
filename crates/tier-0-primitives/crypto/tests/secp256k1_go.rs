//! `secp256k1::verify_go` against cosmos-sdk's secp256k1
//! `PubKey.VerifySignature` (decred secp256k1 v4, no cgo) and
//! `secp256k1::verify` against celestia-core's (CometBFT) one: the edge-case
//! vectors in `conformance/fixtures/secp256k1/verify.json` and
//! `verify_cometbft.json` carry Go's own results (written by
//! `conformance/antevectors/secp256k1`), and random inputs are compared
//! with a k256 implementation of the same rules (the verifier libsecp256k1
//! replaced).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crypto::secp256k1::{verify, verify_go};
use k256::ecdsa::SigningKey;
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    name: String,
    pub_key: String,
    msg: String,
    sig: String,
    valid: bool,
}

/// The rules with k256: r and s reduced modulo n, high-S and zero scalars
/// rejected, SEC1 compressed / uncompressed keys.
fn verify_k256(key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    use k256::elliptic_curve::ops::Reduce;
    use k256::elliptic_curve::scalar::IsHigh;
    use k256::{FieldBytes, Scalar};
    if sig.len() != 64 {
        return false;
    }
    let Ok(vk) = k256::ecdsa::VerifyingKey::from_sec1_bytes(key) else {
        return false;
    };
    let r = <Scalar as Reduce<FieldBytes>>::reduce(&FieldBytes::try_from(&sig[..32]).unwrap());
    let s = <Scalar as Reduce<FieldBytes>>::reduce(&FieldBytes::try_from(&sig[32..]).unwrap());
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

fn vectors(file: &str) -> Vec<Vector> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/secp256k1")
        .join(file);
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// CometBFT's `PubKey.VerifySignature`: Rust builds the key from
/// `PubKeyFromProto` (33 bytes only) and its callers turn a signature of
/// the wrong length into `false`; every key length is also checked against
/// `verify_go`, since the Go function takes raw key bytes.
#[test]
fn cometbft_vectors() {
    let vectors = vectors("verify_cometbft.json");
    assert!(vectors.len() > 500);
    let mut failures = Vec::new();
    let mut compressed = (0usize, 0usize);
    for v in &vectors {
        let key = hex::decode(&v.pub_key).unwrap();
        let msg = hex::decode(&v.msg).unwrap();
        let sig = hex::decode(&v.sig).unwrap();
        if verify_go(&key, &msg, &sig) != v.valid {
            failures.push(format!("{} (verify_go): Go says {}", v.name, v.valid));
        }
        if let Ok(key) = <[u8; 33]>::try_from(key.as_slice()) {
            let got = <[u8; 64]>::try_from(sig.as_slice()).is_ok_and(|s| verify(&key, &msg, &s));
            if got != v.valid {
                failures.push(format!("{} (verify): Go says {}", v.name, v.valid));
            }
            compressed.0 += 1;
            compressed.1 += usize::from(v.valid);
        }
    }
    assert!(
        compressed.0 > 300 && compressed.1 > 30,
        "33-byte coverage {compressed:?}"
    );
    assert!(
        failures.is_empty(),
        "{} mismatches with Go:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn go_vectors() {
    let vectors = vectors("verify.json");
    assert!(vectors.len() > 500);
    assert!(vectors.iter().any(|v| v.valid) && vectors.iter().any(|v| !v.valid));
    let mut failures = Vec::new();
    for v in &vectors {
        let key = hex::decode(&v.pub_key).unwrap();
        let msg = hex::decode(&v.msg).unwrap();
        let sig = hex::decode(&v.sig).unwrap();
        if verify_go(&key, &msg, &sig) != v.valid {
            failures.push(format!("{}: Go says {}", v.name, v.valid));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches with Go:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// xorshift64*: deterministic inputs without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next().to_le_bytes()[0]).collect()
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
    }
}

/// The group order.
const N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];

/// `a + n` over 32 big-endian bytes, `None` when it does not fit.
fn plus_n(a: &[u8]) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in (0..32).rev() {
        let v = u16::from(a[i]) + u16::from(N[i]) + carry;
        out[i] = v.to_le_bytes()[0];
        carry = v >> 8;
    }
    (carry == 0).then_some(out)
}

/// `n - a` over 32 big-endian bytes (`0 < a < n`).
fn n_minus(a: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let v = i16::from(N[i]) - i16::from(a[i]) - borrow;
        out[i] = v.rem_euclid(256).to_le_bytes()[0];
        borrow = i16::from(v < 0);
    }
    out
}

#[test]
fn random_inputs_match_k256() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut valid, mut total) = (0usize, 0usize);
    for _ in 0..4000 {
        let sk = loop {
            if let Ok(sk) = SigningKey::from_slice(&rng.bytes(32)) {
                break sk;
            }
        };
        let vk = *sk.verifying_key();
        let len = rng.below(200);
        let mut msg = rng.bytes(len);
        let (sig, _) = sk.sign_prehash_recoverable(&hash::sha256(&msg));
        let sig = sig.normalize_s();
        let mut sig = sig.to_bytes().to_vec();
        let mut key = match rng.below(3) {
            0 => vk.to_sec1_point(true).as_bytes().to_vec(),
            1 => vk.to_sec1_point(false).as_bytes().to_vec(),
            _ => {
                // Hybrid: 06 / 07 by the parity of y (sometimes the wrong one).
                let mut b = vk.to_sec1_point(false).as_bytes().to_vec();
                b[0] = 6 | (b[64] & 1);
                if rng.below(4) == 0 {
                    b[0] ^= 1;
                }
                b
            }
        };
        match rng.below(10) {
            0..=2 => {}
            3 => {
                let high = n_minus(&sig[32..]);
                sig[32..].copy_from_slice(&high);
            }
            4 => {
                let i = rng.below(64);
                sig[i] ^= 1 << rng.below(8);
            }
            5 => {
                let i = rng.below(key.len());
                key[i] ^= 1 << rng.below(8);
            }
            6 => msg.push(rng.bytes(1)[0]),
            7 => {
                let at = 32 * rng.below(2);
                if let Some(v) = plus_n(&sig[at..at + 32]) {
                    sig[at..at + 32].copy_from_slice(&v);
                }
            }
            8 => sig = rng.bytes(64),
            _ => sig.truncate(rng.below(64)),
        }
        let got = verify_go(&key, &msg, &sig);
        // k256 has no hybrid keys: compare with the same point's 04 key.
        let mut k256_key = key.clone();
        if k256_key.len() == 65 && matches!(k256_key[0], 6 | 7) {
            if k256_key[0] & 1 != k256_key[64] & 1 {
                assert!(!got, "hybrid key with the wrong parity accepted");
                continue;
            }
            k256_key[0] = 4;
        }
        assert_eq!(
            got,
            verify_k256(&k256_key, &msg, &sig),
            "key {} msg {} sig {}",
            hex::encode(&key),
            hex::encode(&msg),
            hex::encode(&sig)
        );
        valid += usize::from(got);
        total += 1;
    }
    assert!(valid > 500 && valid < total, "{valid} of {total} valid");
}
