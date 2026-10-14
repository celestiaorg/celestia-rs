//! The `99designs/keyring` file backend's item encryption: a compact JWE
//! (`dvsekhvalnov/jose2go`) with `PBES2-HS256+A128KW` key management and
//! `A256GCM` content encryption, keyed by the keyring password.

use aes::cipher::{
    BlockDecrypt as _, BlockEncrypt as _, KeyInit as _, generic_array::GenericArray,
};
use aes_gcm::aead::{Aead as _, Payload};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use rand::RngCore as _;

use crate::Error;

const ALG: &str = "PBES2-HS256+A128KW";
const ENC: &str = "A256GCM";
/// jose2go's default PBES2 iteration count.
pub const DEFAULT_P2C: u32 = 8192;
/// jose2go's bounds on accepted iteration counts.
const MIN_P2C: u64 = 0;
const MAX_P2C: u64 = 1_000_000;

const KW_IV: [u8; 8] = [0xA6; 8];

/// RFC 3394 AES key wrap with a 128-bit KEK (variable names follow the RFC).
#[allow(clippy::many_single_char_names)]
fn aes_kw_wrap(kek: &[u8; 16], cek: &[u8]) -> Vec<u8> {
    let cipher = aes::Aes128::new(GenericArray::from_slice(kek));
    let n = cek.len() / 8;
    let mut a = KW_IV;
    let mut r: Vec<[u8; 8]> = cek
        .chunks(8)
        .map(|c| c.try_into().unwrap_or([0; 8]))
        .collect();
    for j in 0..6u64 {
        for (i, ri) in r.iter_mut().enumerate() {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            b[8..].copy_from_slice(ri);
            let block = GenericArray::from_mut_slice(&mut b);
            cipher.encrypt_block(block);
            let t = (n as u64) * j + (i as u64) + 1;
            a.copy_from_slice(&b[..8]);
            for (k, x) in t.to_be_bytes().iter().enumerate() {
                a[k] ^= x;
            }
            ri.copy_from_slice(&b[8..]);
        }
    }
    let mut out = a.to_vec();
    for ri in r {
        out.extend_from_slice(&ri);
    }
    out
}

/// RFC 3394 AES key unwrap with a 128-bit KEK.
#[allow(clippy::many_single_char_names)]
fn aes_kw_unwrap(kek: &[u8; 16], wrapped: &[u8]) -> Result<Vec<u8>, Error> {
    if wrapped.len() < 24 || wrapped.len() % 8 != 0 {
        return Err(Error::Decrypt("invalid wrapped key length".into()));
    }
    let cipher = aes::Aes128::new(GenericArray::from_slice(kek));
    let n = wrapped.len() / 8 - 1;
    let mut a: [u8; 8] = wrapped[..8].try_into().unwrap_or([0; 8]);
    let mut r: Vec<[u8; 8]> = wrapped[8..]
        .chunks(8)
        .map(|c| c.try_into().unwrap_or([0; 8]))
        .collect();
    for j in (0..6u64).rev() {
        for i in (0..n).rev() {
            let t = (n as u64) * j + (i as u64) + 1;
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            for (k, x) in t.to_be_bytes().iter().enumerate() {
                b[k] ^= x;
            }
            b[8..].copy_from_slice(&r[i]);
            let block = GenericArray::from_mut_slice(&mut b);
            cipher.decrypt_block(block);
            a.copy_from_slice(&b[..8]);
            r[i].copy_from_slice(&b[8..]);
        }
    }
    if a != KW_IV {
        return Err(Error::Decrypt(
            "aes key unwrap: integrity check failed".into(),
        ));
    }
    Ok(r.concat())
}

fn kek(password: &str, p2s: &[u8], p2c: u32) -> [u8; 16] {
    let mut salt = ALG.as_bytes().to_vec();
    salt.push(0);
    salt.extend_from_slice(p2s);
    let mut out = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, p2c, &mut out);
    out
}

/// `jose.Encrypt(payload, PBES2_HS256_A128KW, A256GCM, password,
/// Headers{"created": created})`.
pub fn encrypt(payload: &[u8], password: &str, created: &str) -> Result<String, Error> {
    let mut rng = rand::rngs::OsRng;
    let mut p2s = [0u8; 12];
    rng.fill_bytes(&mut p2s);
    let mut cek = [0u8; 32];
    rng.fill_bytes(&mut cek);
    let mut iv = [0u8; 12];
    rng.fill_bytes(&mut iv);
    // Go marshals the header map with sorted keys.
    let header = serde_json::json!({
        "alg": ALG,
        "created": created,
        "enc": ENC,
        "p2c": DEFAULT_P2C,
        "p2s": B64URL.encode(p2s),
    });
    let header_b64 = B64URL.encode(header.to_string());
    let ecek = aes_kw_wrap(&kek(password, &p2s, DEFAULT_P2C), &cek);
    let gcm = aes_gcm::Aes256Gcm::new(GenericArray::from_slice(&cek));
    let sealed = gcm
        .encrypt(
            GenericArray::from_slice(&iv),
            Payload {
                msg: payload,
                aad: header_b64.as_bytes(),
            },
        )
        .map_err(|_| Error::Decrypt("aes-gcm encryption failed".into()))?;
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    Ok(format!(
        "{header_b64}.{}.{}.{}.{}",
        B64URL.encode(ecek),
        B64URL.encode(iv),
        B64URL.encode(ct),
        B64URL.encode(tag)
    ))
}

/// `jose.Decode(token, password)` for tokens written by [`encrypt`] (or Go).
pub fn decrypt(token: &str, password: &str) -> Result<Vec<u8>, Error> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 5 {
        return Err(Error::Decrypt("not a compact JWE".into()));
    }
    let dec = |s: &str| {
        B64URL
            .decode(s.trim_end_matches('='))
            .map_err(|e| Error::Decrypt(format!("base64: {e}")))
    };
    let header: serde_json::Value = serde_json::from_slice(&dec(parts[0])?)
        .map_err(|e| Error::Decrypt(format!("header: {e}")))?;
    if header["alg"] != ALG || header["enc"] != ENC {
        return Err(Error::Decrypt(format!(
            "unsupported JWE algorithms {} / {}",
            header["alg"], header["enc"]
        )));
    }
    let p2c = header["p2c"]
        .as_f64()
        .ok_or_else(|| Error::Decrypt("missing p2c".into()))?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let p2c = p2c as u64;
    if !(MIN_P2C..=MAX_P2C).contains(&p2c) {
        return Err(Error::Decrypt(format!("p2c {p2c} out of range")));
    }
    let p2s = dec(header["p2s"]
        .as_str()
        .ok_or_else(|| Error::Decrypt("missing p2s".into()))?)?;
    let kek = kek(password, &p2s, u32::try_from(p2c).unwrap_or(DEFAULT_P2C));
    let cek = aes_kw_unwrap(&kek, &dec(parts[1])?)?;
    if cek.len() != 32 {
        return Err(Error::Decrypt("unexpected content key size".into()));
    }
    let iv = dec(parts[2])?;
    if iv.len() != 12 {
        return Err(Error::Decrypt("unexpected iv size".into()));
    }
    let mut sealed = dec(parts[3])?;
    sealed.extend_from_slice(&dec(parts[4])?);
    let gcm = aes_gcm::Aes256Gcm::new(GenericArray::from_slice(&cek));
    gcm.decrypt(
        GenericArray::from_slice(&iv),
        Payload {
            msg: &sealed,
            aad: parts[0].as_bytes(),
        },
    )
    .map_err(|_| Error::Decrypt("aes.Decrypt(): authentication tag check failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3394_vector() {
        // RFC 3394 §4.1: wrap 128 bits of key data with a 128-bit KEK.
        let kek: [u8; 16] = hex::decode("000102030405060708090A0B0C0D0E0F")
            .unwrap()
            .try_into()
            .unwrap();
        let data = hex::decode("00112233445566778899AABBCCDDEEFF").unwrap();
        let wrapped = aes_kw_wrap(&kek, &data);
        assert_eq!(
            hex::encode_upper(&wrapped),
            "1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5"
        );
        assert_eq!(aes_kw_unwrap(&kek, &wrapped).unwrap(), data);
    }

    #[test]
    fn roundtrip() {
        let t = encrypt(b"hello", "pw", "now").unwrap();
        assert_eq!(decrypt(&t, "pw").unwrap(), b"hello");
        assert!(decrypt(&t, "wrong").is_err());
    }
}
