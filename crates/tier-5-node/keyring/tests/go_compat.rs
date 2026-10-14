//! Keyrings written by Go `celestia-appd keys` (v10.4.0, fixtures) read
//! back by the Rust keyring, and Rust-written keyrings read back by Rust
//! through the same code paths Go uses (JWE items, record protobufs).
//!
//! Fixtures (`tests/fixtures`): `keys add alice --recover` (test backend)
//! and `keys add bob --recover` (file backend, passphrase `passw0rd1`)
//! with the BIP-39 test mnemonic `abandon … art`, and `keys export alice`
//! (passphrase `exportpass`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use keyring::{Backend, Keyring, Prompt};

const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
const ADDRESS: &str = "celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj";
const PUBKEY: &str = "{\"@type\":\"/cosmos.crypto.secp256k1.PubKey\",\"key\":\"ArpmqEz3g5rxcqE+f8n15wCMuLyhWF+PO6+zA57aPB/d\"}";
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
    }
}

fn passwords(lines: &[&str]) -> keyring::PasswordSource {
    let mut lines: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
    lines.reverse();
    Box::new(move |_p: Prompt| lines.pop().ok_or(keyring::Error::WrongPassword))
}

#[test]
fn reads_go_test_keyring() {
    let home = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(FIXTURES).join("keyring-test"),
        &home.path().join("keyring-test"),
    );
    let mut kr = Keyring::new(home.path(), Backend::Test);
    let keys = kr.list().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].name, "alice");
    assert_eq!(keys[0].bech32("celestia"), ADDRESS);
    assert_eq!(keys[0].pubkey_json(), PUBKEY);
    // The secret matches the mnemonic's default-path key.
    let secret = keyring::hd::derive(MNEMONIC, "", keyring::hd::DEFAULT_HD_PATH).unwrap();
    assert_eq!(keys[0].secret, Some(secret));
    let by_addr = kr.key_by_address(&keys[0].address()).unwrap();
    assert_eq!(by_addr.name, "alice");
    assert_eq!(
        keys[0].bech32("celestiavaloper"),
        "celestiavaloper1r5v5srda7xfth3hn2s26txvrcrntldju075jm5"
    );
}

#[test]
fn reads_go_file_keyring() {
    let home = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(FIXTURES).join("keyring-file"),
        &home.path().join("keyring-file"),
    );
    // Wrong passphrases are rejected (keyhash), the right one unlocks.
    let mut kr = Keyring::new(home.path(), Backend::File)
        .with_password_source(passwords(&["nope", "passw0rd1"]));
    let bob = kr.key("bob").unwrap();
    assert_eq!(bob.bech32("celestia"), ADDRESS);
    let mut kr =
        Keyring::new(home.path(), Backend::File).with_password_source(passwords(&["a", "b", "c"]));
    assert!(kr.key("bob").is_err());
}

#[test]
fn imports_go_armor() {
    let armor = std::fs::read_to_string(Path::new(FIXTURES).join("alice.armor")).unwrap();
    let secret = keyring::armor::decrypt_secp256k1(&armor, "exportpass").unwrap();
    assert_eq!(
        secret,
        keyring::hd::derive(MNEMONIC, "", keyring::hd::DEFAULT_HD_PATH).unwrap()
    );
    assert!(keyring::armor::decrypt_secp256k1(&armor, "wrong").is_err());
}

#[test]
fn rust_keyring_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    let mut kr = Keyring::new(home.path(), Backend::Test);
    let a = kr
        .add_mnemonic("alice", MNEMONIC, "", keyring::hd::DEFAULT_HD_PATH)
        .unwrap();
    assert_eq!(a.bech32("celestia"), ADDRESS);
    assert!(
        kr.add_mnemonic("alice", MNEMONIC, "", keyring::hd::DEFAULT_HD_PATH)
            .is_err()
    );
    let m = keyring::hd::new_mnemonic();
    let b = kr
        .add_mnemonic("b/x", &m, "", keyring::hd::DEFAULT_HD_PATH)
        .unwrap();
    // Re-open: everything decodes.
    let mut kr = Keyring::new(home.path(), Backend::Test);
    let names: Vec<String> = kr.list().unwrap().into_iter().map(|k| k.name).collect();
    assert_eq!(names, ["alice", "b/x"]);
    assert_eq!(kr.key_by_address(&b.address()).unwrap().name, "b/x");
    // Export / import.
    let armor = kr.export_armor("alice", "pw").unwrap();
    assert!(armor.starts_with("-----BEGIN TENDERMINT PRIVATE KEY-----\nkdf: argon2\n"));
    kr.delete("alice").unwrap();
    assert!(kr.key("alice").is_err());
    let back = kr.import_armor("alice2", &armor, "pw").unwrap();
    assert_eq!(back.bech32("celestia"), ADDRESS);
    // File backend: a new keyring stores the keyhash, then unlocks with it.
    let mut kr = Keyring::new(home.path(), Backend::File)
        .with_password_source(passwords(&["secret12", "secret12"]));
    kr.add_secret("raw", [7u8; 32]).unwrap();
    assert!(home.path().join("keyring-file/keyhash").exists());
    let mut kr =
        Keyring::new(home.path(), Backend::File).with_password_source(passwords(&["secret12"]));
    assert_eq!(kr.key("raw").unwrap().secret, Some([7u8; 32]));
    // Signatures verify.
    let k = kr.key("raw").unwrap();
    let sig = k.sign(b"msg").unwrap();
    assert!(crypto::secp256k1::verify(&k.pub_key, b"msg", &sig));
}
