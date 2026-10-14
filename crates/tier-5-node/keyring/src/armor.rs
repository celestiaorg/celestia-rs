//! ASCII-armored, passphrase-encrypted private keys (`keys export` /
//! `keys import`; celestia cosmos-sdk `crypto/armor.go`): argon2id
//! (t=1, m=64 MiB, p=4) + ChaCha20-Poly1305 with a zero nonce over the
//! amino encoding of the key, in OpenPGP armor with a CRC-24 checksum.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead as _, KeyInit as _};
use rand::RngCore as _;

use crate::Error;

const BLOCK_TYPE: &str = "TENDERMINT PRIVATE KEY";
/// Amino prefix of `tendermint/PrivKeySecp256k1`.
const SECP256K1_PRIV_PREFIX: [u8; 4] = [0xe1, 0xb0, 0xf7, 0x9b];

fn crc24(data: &[u8]) -> u32 {
    const INIT: u32 = 0x00B7_04CE;
    const POLY: u32 = 0x0186_4CFB;
    let mut crc = INIT;
    for &b in data {
        crc ^= u32::from(b) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x0100_0000 != 0 {
                crc ^= POLY;
            }
        }
    }
    crc & 0x00FF_FFFF
}

/// OpenPGP armor (`armor.EncodeArmor`), headers sorted by key.
#[must_use]
pub fn encode(block_type: &str, headers: &[(&str, String)], data: &[u8]) -> String {
    let mut out = format!("-----BEGIN {block_type}-----\n");
    let mut hs: Vec<&(&str, String)> = headers.iter().collect();
    hs.sort_by_key(|(k, _)| *k);
    for (k, v) in hs {
        let _ = writeln!(out, "{k}: {v}");
    }
    out.push('\n');
    let b64 = B64.encode(data);
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    let crc = crc24(data).to_be_bytes();
    out.push('=');
    out.push_str(&B64.encode(&crc[1..]));
    out.push('\n');
    let _ = write!(out, "-----END {block_type}-----");
    out
}

/// Parsed armor: block type, headers, payload.
pub type Armored = (String, Vec<(String, String)>, Vec<u8>);

/// `armor.DecodeArmor`.
pub fn decode(s: &str) -> Result<Armored, Error> {
    let bad = |m: &str| Error::Armor(m.to_owned());
    let mut lines = s.lines().map(str::trim_end).skip_while(|l| l.is_empty());
    let first = lines.next().ok_or_else(|| bad("empty armor"))?;
    let block_type = first
        .strip_prefix("-----BEGIN ")
        .and_then(|r| r.strip_suffix("-----"))
        .ok_or_else(|| bad("missing armor header"))?
        .to_owned();
    let mut headers = Vec::new();
    for l in lines.by_ref() {
        if l.is_empty() {
            break;
        }
        let (k, v) = l.split_once(':').ok_or_else(|| bad("bad armor header"))?;
        headers.push((k.trim().to_owned(), v.trim().to_owned()));
    }
    let mut b64 = String::new();
    let mut crc = None;
    for l in lines {
        if l.starts_with("-----END ") {
            break;
        }
        if let Some(c) = l.strip_prefix('=') {
            crc = Some(c.to_owned());
            continue;
        }
        b64.push_str(l.trim());
    }
    let data = B64
        .decode(b64)
        .map_err(|e| Error::Armor(format!("base64: {e}")))?;
    if let Some(c) = crc {
        let c = B64
            .decode(c)
            .map_err(|e| Error::Armor(format!("checksum: {e}")))?;
        let want = crc24(&data).to_be_bytes();
        if c.as_slice() != &want[1..] {
            return Err(bad("armor checksum mismatch"));
        }
    }
    Ok((block_type, headers, data))
}

fn argon2_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], Error> {
    let params =
        argon2::Params::new(64 * 1024, 1, 4, Some(32)).map_err(|e| Error::Crypto(e.to_string()))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    a.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(key)
}

/// `EncryptArmorPrivKey(privKey, passphrase, "secp256k1")`.
pub fn encrypt_secp256k1(secret: &[u8; 32], passphrase: &str) -> Result<String, Error> {
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let key = argon2_key(passphrase, &salt)?;
    let mut plain = SECP256K1_PRIV_PREFIX.to_vec();
    plain.push(32);
    plain.extend_from_slice(secret);
    let aead = chacha20poly1305::ChaCha20Poly1305::new((&key).into());
    let enc = aead
        .encrypt(&[0u8; 12].into(), plain.as_slice())
        .map_err(|_| Error::Crypto("encryption failed".into()))?;
    Ok(encode(
        BLOCK_TYPE,
        &[
            ("kdf", "argon2".into()),
            ("salt", hex::encode_upper(salt)),
            ("type", "secp256k1".into()),
        ],
        &enc,
    ))
}

/// `UnarmorDecryptPrivKey`: the secp256k1 secret key of an armored export.
pub fn decrypt_secp256k1(armor: &str, passphrase: &str) -> Result<[u8; 32], Error> {
    let (block_type, headers, data) = decode(armor)?;
    if block_type != BLOCK_TYPE {
        return Err(Error::Armor(format!(
            "unrecognized armor type: {block_type}"
        )));
    }
    let header = |k: &str| {
        headers
            .iter()
            .find(|(hk, _)| hk == k)
            .map(|(_, v)| v.clone())
    };
    if let Some(t) = header("type") {
        if t != "secp256k1" {
            return Err(Error::Armor(format!("unsupported key type {t}")));
        }
    }
    let salt = hex::decode(header("salt").ok_or_else(|| Error::Armor("missing salt".into()))?)
        .map_err(|e| Error::Armor(format!("salt: {e}")))?;
    let plain = match header("kdf").as_deref() {
        Some("argon2") => {
            let key = argon2_key(passphrase, &salt)?;
            let aead = chacha20poly1305::ChaCha20Poly1305::new((&key).into());
            aead.decrypt(&[0u8; 12].into(), data.as_slice())
                .map_err(|_| Error::WrongPassword)?
        }
        Some(other) => {
            return Err(Error::Armor(format!(
                "unsupported key derivation function (kdf) header: {other}"
            )));
        }
        None => return Err(Error::Armor("missing kdf header".into())),
    };
    match plain.as_slice() {
        [p0, p1, p2, p3, 32, rest @ ..]
            if [*p0, *p1, *p2, *p3] == SECP256K1_PRIV_PREFIX && rest.len() == 32 =>
        {
            let mut key = [0u8; 32];
            key.copy_from_slice(rest);
            Ok(key)
        }
        _ => Err(Error::Armor(
            "decrypted key is not an amino secp256k1 key".into(),
        )),
    }
}
