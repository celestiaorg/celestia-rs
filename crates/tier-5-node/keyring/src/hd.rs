//! BIP-39 mnemonics and BIP-32 secp256k1 derivation (Cosmos SDK
//! `crypto/hd`: `Secp256k1.Derive`, `ComputeMastersFromSeed`,
//! `DerivePrivateKeyForPath`).

use hmac::{Hmac, Mac as _};
use k256::elliptic_curve::PrimeField as _;
use rand::RngCore as _;
use sha2::Sha512;

use crate::Error;

/// Cosmos coin type.
pub const COIN_TYPE: u32 = 118;
/// `sdk.FullFundraiserPath`: the default HD path of `keys add`.
pub const DEFAULT_HD_PATH: &str = "m/44'/118'/0'/0/0";

/// A new 24-word English mnemonic (256 bits of entropy, like `keys add`).
#[must_use]
pub fn new_mnemonic() -> String {
    let mut entropy = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut entropy);
    bip39::Mnemonic::from_entropy(&entropy)
        .map(|m| m.to_string())
        .unwrap_or_default()
}

/// Validates an English mnemonic (`bip39.IsMnemonicValid`).
pub fn validate_mnemonic(mnemonic: &str) -> Result<(), Error> {
    bip39::Mnemonic::parse_in_normalized(bip39::Language::English, mnemonic)
        .map(|_| ())
        .map_err(|e| Error::InvalidMnemonic(e.to_string()))
}

/// The BIP-39 seed of `mnemonic` with `passphrase`
/// (`bip39.NewSeedWithErrorChecking`).
pub fn seed(mnemonic: &str, passphrase: &str) -> Result<[u8; 64], Error> {
    let m = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, mnemonic)
        .map_err(|e| Error::InvalidMnemonic(e.to_string()))?;
    Ok(m.to_seed_normalized(passphrase))
}

fn hmac512(key: &[u8], data: &[u8]) -> [u8; 64] {
    #[allow(clippy::expect_used)]
    let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn split(i: &[u8; 64]) -> ([u8; 32], [u8; 32]) {
    let mut l = [0u8; 32];
    let mut r = [0u8; 32];
    l.copy_from_slice(&i[..32]);
    r.copy_from_slice(&i[32..]);
    (l, r)
}

/// Parses `m/44'/118'/0'/0/0` into BIP-32 indexes (hardened bit set).
pub fn parse_path(path: &str) -> Result<Vec<u32>, Error> {
    let mut parts = path.trim().split('/');
    if parts.next() != Some("m") {
        return Err(Error::InvalidHdPath(path.to_owned()));
    }
    parts
        .map(|p| {
            let (num, hardened) = match p.strip_suffix('\'') {
                Some(n) => (n, true),
                None => (p, false),
            };
            let n: u32 = num
                .parse()
                .map_err(|_| Error::InvalidHdPath(path.to_owned()))?;
            if n >= 0x8000_0000 {
                return Err(Error::InvalidHdPath(path.to_owned()));
            }
            Ok(if hardened { n | 0x8000_0000 } else { n })
        })
        .collect()
}

fn scalar(bytes: &[u8; 32]) -> Option<k256::Scalar> {
    Option::from(k256::Scalar::from_repr((*bytes).into()))
}

/// Derives the secp256k1 secret key at `path` from a BIP-39 seed.
pub fn derive_from_seed(seed: &[u8], path: &str) -> Result<[u8; 32], Error> {
    let (mut key, mut chain) = split(&hmac512(b"Bitcoin seed", seed));
    for index in parse_path(path)? {
        let mut data = Vec::with_capacity(37);
        if index & 0x8000_0000 == 0 {
            let pubkey = crypto::secp256k1::public_key_from_secret_key(&key)
                .map_err(|e| Error::Crypto(e.to_string()))?;
            data.extend_from_slice(&pubkey);
        } else {
            data.push(0);
            data.extend_from_slice(&key);
        }
        data.extend_from_slice(&index.to_be_bytes());
        let (il, ir) = split(&hmac512(&chain, &data));
        let k = scalar(&key).ok_or_else(|| Error::Crypto("invalid parent key".into()))?;
        let t = scalar(&il).ok_or_else(|| Error::Crypto("invalid child key".into()))?;
        let child = k + t;
        if bool::from(k256::elliptic_curve::Field::is_zero(&child)) {
            return Err(Error::Crypto("derived zero key".into()));
        }
        key = child.to_repr().into();
        chain = ir;
    }
    Ok(key)
}

/// `hd.Secp256k1.Derive()(mnemonic, passphrase, path)`.
pub fn derive(mnemonic: &str, passphrase: &str, path: &str) -> Result<[u8; 32], Error> {
    derive_from_seed(&seed(mnemonic, passphrase)?, path)
}

/// `hd.CreateHDPath(coinType, account, index)` as a string.
#[must_use]
pub fn hd_path(coin_type: u32, account: u32, index: u32) -> String {
    format!("m/44'/{coin_type}'/{account}'/0/{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bip32_test_vector_1() {
        // BIP-32 test vector 1, chain m/0'/1/2'/2/1000000000.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let k = derive_from_seed(&seed, "m/0'/1/2'/2/1000000000").unwrap();
        assert_eq!(
            hex::encode(k),
            "471b76e389e528d6de6d816857e012c5455051cad6660850e58372a6c3e6e7c8"
        );
    }

    #[test]
    fn mnemonic_roundtrip() {
        let m = new_mnemonic();
        assert_eq!(m.split(' ').count(), 24);
        validate_mnemonic(&m).unwrap();
        assert!(validate_mnemonic("abandon abandon").is_err());
    }
}
