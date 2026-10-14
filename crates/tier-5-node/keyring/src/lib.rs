//! Cosmos SDK keyring compatible with Go `celestia-appd keys` on disk.
//!
//! Supported backends (cosmos-sdk `crypto/keyring`, `99designs/keyring`
//! file backend):
//!
//! | backend | directory | password |
//! |---|---|---|
//! | `test` | `<home>/keyring-test` | the fixed string `test` |
//! | `file` | `<home>/keyring-file` | prompted; bcrypt hash in `keyhash` |
//!
//! Every key is two items, each a compact JWE (PBES2-HS256+A128KW /
//! A256GCM) of the JSON-encoded `keyring.Item`: `<name>.info` holds the
//! protobuf `cosmos.crypto.keyring.v1.Record`, `<hex address>.address`
//! the item key of the record. Keys are secp256k1, derived from BIP-39
//! mnemonics along BIP-32 paths (default `m/44'/118'/0'/0/0`).

#![forbid(unsafe_code)]

pub mod armor;
pub mod hd;
pub mod jwe;

use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use celestia_proto::Any;
use celestia_proto::cosmos::crypto::keyring::v1::{Record, record};
use celestia_proto::cosmos::crypto::secp256k1::{PrivKey, PubKey};
use prost::Message as _;
use serde::{Deserialize, Serialize};

/// Keyring errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The key does not exist.
    NotFound(String),
    /// A key with this name or address exists.
    AlreadyExists(String),
    /// Wrong keyring or export passphrase.
    WrongPassword,
    /// Decryption failure.
    Decrypt(String),
    /// Malformed armor.
    Armor(String),
    /// Invalid mnemonic.
    InvalidMnemonic(String),
    /// Invalid HD path.
    InvalidHdPath(String),
    /// Cryptographic failure.
    Crypto(String),
    /// Unsupported backend or record type.
    Unsupported(String),
    /// I/O failure.
    Io(String),
    /// Malformed stored data.
    Corrupt(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(n) => write!(f, "{n}: key not found"),
            Self::AlreadyExists(n) => write!(f, "{n}: key exists"),
            Self::WrongPassword => f.write_str("invalid account password"),
            Self::Decrypt(m) => write!(f, "decrypt: {m}"),
            Self::Armor(m) => write!(f, "armor: {m}"),
            Self::InvalidMnemonic(m) => write!(f, "invalid mnemonic: {m}"),
            Self::InvalidHdPath(p) => write!(f, "invalid hd path {p}"),
            Self::Crypto(m) => write!(f, "crypto: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Corrupt(m) => write!(f, "corrupt keyring: {m}"),
        }
    }
}

impl std::error::Error for Error {}

fn io_err(e: impl fmt::Display) -> Error {
    Error::Io(e.to_string())
}

/// Keyring backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `test`: unencrypted in practice (fixed password `test`).
    Test,
    /// `file`: encrypted with a user passphrase.
    File,
}

impl std::str::FromStr for Backend {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "test" => Ok(Self::Test),
            "file" => Ok(Self::File),
            other => Err(Error::Unsupported(format!(
                "keyring backend {other:?} (supported: file, test)"
            ))),
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Test => "test",
            Self::File => "file",
        })
    }
}

/// What a password prompt is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Unlock an existing `file` keyring.
    Unlock,
    /// Choose the passphrase of a new `file` keyring.
    New,
    /// Re-enter the new passphrase.
    Confirm,
}

/// Supplies passwords (stdin lines in the CLI, like Go's `input.GetPassword`).
pub type PasswordSource = Box<dyn FnMut(Prompt) -> Result<String, Error>>;

/// Kind of a stored key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// Private key held locally.
    Local,
    /// Ledger device (listed only).
    Ledger,
    /// Public key only.
    Offline,
    /// Multisig (listed only).
    Multi,
}

impl fmt::Display for KeyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::Ledger => "ledger",
            Self::Offline => "offline",
            Self::Multi => "multi",
        })
    }
}

/// A key of the keyring.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyRecord {
    /// Name (`uid`).
    pub name: String,
    /// Compressed secp256k1 public key.
    pub pub_key: [u8; 33],
    /// Secret key (local keys).
    pub secret: Option<[u8; 32]>,
    /// Kind.
    pub kind: KeyKind,
}

impl fmt::Debug for KeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRecord")
            .field("name", &self.name)
            .field("pub_key", &hex::encode(self.pub_key))
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl KeyRecord {
    /// Account address bytes.
    #[must_use]
    pub fn address(&self) -> [u8; 20] {
        crypto::secp256k1::address(&self.pub_key)
    }

    /// Bech32 address with `prefix` (`celestia`, `celestiavaloper`, …).
    #[must_use]
    pub fn bech32(&self, prefix: &str) -> String {
        encoding::bech32::encode(prefix, &self.address()).unwrap_or_default()
    }

    /// The public key as Go prints it in `keys show` (`codec.ProtoMarshalJSON`).
    #[must_use]
    pub fn pubkey_json(&self) -> String {
        format!(
            "{{\"@type\":\"/cosmos.crypto.secp256k1.PubKey\",\"key\":\"{}\"}}",
            B64.encode(self.pub_key)
        )
    }

    /// The public key packed in an `Any`.
    #[must_use]
    pub fn pubkey_any(&self) -> Any {
        Any::pack(&PubKey {
            key: self.pub_key.to_vec(),
        })
    }

    /// Signs `msg` (SHA-256, low-S `r || s`).
    pub fn sign(&self, msg: &[u8]) -> Result<[u8; 64], Error> {
        let secret = self
            .secret
            .ok_or_else(|| Error::Unsupported(format!("key {} has no private key", self.name)))?;
        crypto::secp256k1::sign(&secret, msg).map_err(|e| Error::Crypto(e.to_string()))
    }

    fn from_secret(name: &str, secret: [u8; 32]) -> Result<Self, Error> {
        let pub_key = crypto::secp256k1::public_key_from_secret_key(&secret)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        Ok(Self {
            name: name.to_owned(),
            pub_key,
            secret: Some(secret),
            kind: KeyKind::Local,
        })
    }

    fn to_record(&self) -> Record {
        let item = match (self.kind, self.secret) {
            (KeyKind::Local, Some(s)) => record::Item::Local(record::Local {
                priv_key: Some(Any::pack(&PrivKey { key: s.to_vec() })),
            }),
            _ => record::Item::Offline(record::Offline {}),
        };
        Record {
            name: self.name.clone(),
            pub_key: Some(self.pubkey_any()),
            item: Some(item),
        }
    }

    fn from_record(r: &Record) -> Result<Self, Error> {
        let pk = r
            .pub_key
            .as_ref()
            .ok_or_else(|| Error::Corrupt(format!("{}: record without public key", r.name)))?;
        let pub_key: [u8; 33] = match pk.unpack::<PubKey>() {
            Some(Ok(p)) => p
                .key
                .try_into()
                .map_err(|_| Error::Corrupt("public key length".into()))?,
            _ => {
                return Err(Error::Unsupported(format!(
                    "{}: public key type {}",
                    r.name, pk.type_url
                )));
            }
        };
        let (kind, secret) = match &r.item {
            Some(record::Item::Local(l)) => {
                let any = l
                    .priv_key
                    .as_ref()
                    .ok_or_else(|| Error::Corrupt("local record without key".into()))?;
                let Some(Ok(k)) = any.unpack::<PrivKey>() else {
                    return Err(Error::Unsupported(format!(
                        "private key type {}",
                        any.type_url
                    )));
                };
                let s: [u8; 32] = k
                    .key
                    .try_into()
                    .map_err(|_| Error::Corrupt("private key length".into()))?;
                (KeyKind::Local, Some(s))
            }
            Some(record::Item::Ledger(_)) => (KeyKind::Ledger, None),
            Some(record::Item::Multi(_)) => (KeyKind::Multi, None),
            Some(record::Item::Offline(_)) | None => (KeyKind::Offline, None),
        };
        Ok(Self {
            name: r.name.clone(),
            pub_key,
            secret,
            kind,
        })
    }
}

/// `keyring.Item` as the file backend stores it (JSON, Go field names).
#[derive(Debug, Serialize, Deserialize)]
struct Item {
    #[serde(rename = "Key")]
    key: String,
    #[serde(rename = "Data", with = "b64")]
    data: Vec<u8>,
    #[serde(rename = "Label", default)]
    label: String,
    #[serde(rename = "Description", default)]
    description: String,
    #[serde(rename = "KeychainNotTrustApplication", default)]
    keychain_not_trust_application: bool,
    #[serde(rename = "KeychainNotSynchronizable", default)]
    keychain_not_synchronizable: bool,
}

mod b64 {
    use base64::Engine as _;
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::B64.encode(v))
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = Option::<String>::deserialize(d)?.unwrap_or_default();
        super::B64.decode(s).map_err(serde::de::Error::custom)
    }
}

/// `percent.Encode(key, "/")`.
fn escape(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c == '/' || c == '%' {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn unescape(name: &str) -> String {
    let b = name.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&b[i + 1..i + 3]), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Go `time.Now().String()` (stored in the JWE header, never parsed).
fn go_time_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = i64::try_from(now.as_secs()).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:09} +0000 UTC",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60,
        now.subsec_nanos()
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A keyring directory.
pub struct Keyring {
    dir: PathBuf,
    backend: Backend,
    password: Option<String>,
    source: Option<PasswordSource>,
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field("dir", &self.dir)
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

const INFO_SUFFIX: &str = ".info";
const ADDRESS_SUFFIX: &str = ".address";
const MAX_PASSPHRASE_ATTEMPTS: usize = 3;

impl Keyring {
    /// The keyring of `home` (`<home>/keyring-<backend>`).
    #[must_use]
    pub fn new(home: &Path, backend: Backend) -> Self {
        let dir = home.join(match backend {
            Backend::Test => "keyring-test",
            Backend::File => "keyring-file",
        });
        Self {
            dir,
            backend,
            password: (backend == Backend::Test).then(|| "test".to_owned()),
            source: None,
        }
    }

    /// Sets where `file` keyring passwords come from.
    #[must_use]
    pub fn with_password_source(mut self, source: PasswordSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Uses `password` for a `file` keyring (no prompt, no keyhash check).
    #[must_use]
    pub fn with_password(mut self, password: &str) -> Self {
        self.password = Some(password.to_owned());
        self
    }

    /// The keyring directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The backend.
    #[must_use]
    pub fn backend(&self) -> Backend {
        self.backend
    }

    fn ensure_dir(&self) -> Result<(), Error> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir).map_err(io_err)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(io_err)?;
            }
        }
        Ok(())
    }

    /// `newRealPrompt`: the keyring passphrase, checked against (or
    /// stored as) the bcrypt `keyhash`.
    fn password(&mut self) -> Result<String, Error> {
        if let Some(p) = &self.password {
            return Ok(p.clone());
        }
        self.ensure_dir()?;
        let source = self.source.as_mut().ok_or_else(|| {
            Error::Unsupported("no passphrase source for the file keyring".into())
        })?;
        let keyhash_path = self.dir.join("keyhash");
        let stored = std::fs::read(&keyhash_path).ok();
        for _ in 0..MAX_PASSPHRASE_ATTEMPTS {
            if let Some(hash) = &stored {
                let pass = source(Prompt::Unlock)?;
                let hash = String::from_utf8_lossy(hash).trim().to_owned();
                if bcrypt::verify(&pass, &hash).unwrap_or(false) {
                    self.password = Some(pass.clone());
                    return Ok(pass);
                }
                continue;
            }
            let pass = source(Prompt::New)?;
            let again = source(Prompt::Confirm)?;
            if pass != again {
                continue;
            }
            // Go asks for cost 2, which golang.org/x/crypto/bcrypt raises to
            // its default cost 10.
            let hash = bcrypt::hash_with_result(&pass, 10)
                .map_err(|e| Error::Crypto(e.to_string()))?
                .format_for_version(bcrypt::Version::TwoA);
            write_private(&keyhash_path, hash.as_bytes())?;
            self.password = Some(pass.clone());
            return Ok(pass);
        }
        Err(Error::WrongPassword)
    }

    fn item_path(&self, key: &str) -> PathBuf {
        self.dir.join(escape(key))
    }

    fn get_item(&mut self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = self.item_path(key);
        let Ok(token) = std::fs::read_to_string(&path) else {
            return Ok(None);
        };
        let password = self.password()?;
        let payload = jwe::decrypt(&token, &password).map_err(|e| match e {
            Error::Decrypt(_) => Error::WrongPassword,
            other => other,
        })?;
        let item: Item =
            serde_json::from_slice(&payload).map_err(|e| Error::Corrupt(format!("{key}: {e}")))?;
        Ok(Some(item.data))
    }

    fn set_item(&mut self, key: &str, data: Vec<u8>) -> Result<(), Error> {
        self.ensure_dir()?;
        let password = self.password()?;
        let item = Item {
            key: key.to_owned(),
            data,
            label: String::new(),
            description: String::new(),
            keychain_not_trust_application: false,
            keychain_not_synchronizable: false,
        };
        let json = serde_json::to_vec(&item).map_err(|e| Error::Corrupt(e.to_string()))?;
        let token = jwe::encrypt(&json, &password, &go_time_now())?;
        write_private(&self.item_path(key), token.as_bytes())
    }

    fn item_keys(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| unescape(&e.file_name().to_string_lossy()))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Every key (`keys list`), in item-file order.
    pub fn list(&mut self) -> Result<Vec<KeyRecord>, Error> {
        let mut out = Vec::new();
        for k in self.item_keys() {
            if let Some(name) = k.strip_suffix(INFO_SUFFIX) {
                out.push(self.key(name)?);
            }
        }
        Ok(out)
    }

    /// The key named `name`.
    pub fn key(&mut self, name: &str) -> Result<KeyRecord, Error> {
        let key = format!("{name}{INFO_SUFFIX}");
        let data = self
            .get_item(&key)?
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        let rec = Record::decode(data.as_slice()).map_err(|e| {
            Error::Unsupported(format!(
                "{name}: not a protobuf keyring record (legacy amino keyrings must be migrated by celestia-appd first): {e}"
            ))
        })?;
        KeyRecord::from_record(&rec)
    }

    /// The key with account address `address`.
    pub fn key_by_address(&mut self, address: &[u8]) -> Result<KeyRecord, Error> {
        let item = format!("{}{ADDRESS_SUFFIX}", hex::encode(address));
        let info = self
            .get_item(&item)?
            .ok_or_else(|| Error::NotFound(hex::encode(address)))?;
        let info = String::from_utf8_lossy(&info).into_owned();
        let name = info.strip_suffix(INFO_SUFFIX).unwrap_or(&info).to_owned();
        self.key(&name)
    }

    /// `writeRecord`: stores `<name>.info` and `<address>.address`.
    fn write(&mut self, rec: &KeyRecord) -> Result<(), Error> {
        let info_key = format!("{}{INFO_SUFFIX}", rec.name);
        let addr_key = format!("{}{ADDRESS_SUFFIX}", hex::encode(rec.address()));
        if self.item_path(&info_key).exists() {
            return Err(Error::AlreadyExists(info_key));
        }
        // Like Go's `existsInDb`, an address item without this name's record
        // is replaced: the address then resolves to the new name (the old
        // record stays listed under its own name).
        self.set_item(&info_key, rec.to_record().encode_to_vec())?;
        self.set_item(&addr_key, info_key.into_bytes())
    }

    /// `NewAccount(name, mnemonic, bip39Passphrase, hdPath, Secp256k1)`.
    pub fn add_mnemonic(
        &mut self,
        name: &str,
        mnemonic: &str,
        bip39_passphrase: &str,
        hd_path: &str,
    ) -> Result<KeyRecord, Error> {
        let secret = hd::derive(mnemonic, bip39_passphrase, hd_path)?;
        let rec = KeyRecord::from_secret(name, secret)?;
        self.write(&rec)?;
        Ok(rec)
    }

    /// Stores a raw secp256k1 secret key (`keys import-hex`).
    pub fn add_secret(&mut self, name: &str, secret: [u8; 32]) -> Result<KeyRecord, Error> {
        crypto::secp256k1::secret_key_from_bytes(&secret)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let rec = KeyRecord::from_secret(name, secret)?;
        self.write(&rec)?;
        Ok(rec)
    }

    /// Stores a public key only (`keys add --pubkey`).
    pub fn add_offline(&mut self, name: &str, pub_key: [u8; 33]) -> Result<KeyRecord, Error> {
        let rec = KeyRecord {
            name: name.to_owned(),
            pub_key,
            secret: None,
            kind: KeyKind::Offline,
        };
        self.write(&rec)?;
        Ok(rec)
    }

    /// `ImportPrivKey(name, armor, passphrase)`.
    pub fn import_armor(
        &mut self,
        name: &str,
        armor: &str,
        passphrase: &str,
    ) -> Result<KeyRecord, Error> {
        if self.item_path(&format!("{name}{INFO_SUFFIX}")).exists() {
            return Err(Error::AlreadyExists(name.to_owned()));
        }
        let secret = armor::decrypt_secp256k1(armor, passphrase)?;
        self.add_secret(name, secret)
    }

    /// `ExportPrivKeyArmor(name, passphrase)`.
    pub fn export_armor(&mut self, name: &str, passphrase: &str) -> Result<String, Error> {
        let rec = self.key(name)?;
        let secret = rec.secret.ok_or_else(|| {
            Error::Unsupported(format!("{name}: only local keys can be exported"))
        })?;
        armor::encrypt_secp256k1(&secret, passphrase)
    }

    /// Deletes a key (`keys delete`).
    pub fn delete(&mut self, name: &str) -> Result<(), Error> {
        let rec = self.key(name)?;
        let addr = self.item_path(&format!("{}{ADDRESS_SUFFIX}", hex::encode(rec.address())));
        let _ = std::fs::remove_file(addr);
        std::fs::remove_file(self.item_path(&format!("{name}{INFO_SUFFIX}"))).map_err(io_err)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(io_err)?;
    f.write_all(bytes).map_err(io_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        assert_eq!(escape("a/b%c"), "a%2Fb%25c");
        assert_eq!(unescape("a%2Fb%25c"), "a/b%c");
        assert_eq!(unescape("alice.info"), "alice.info");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_740), (2026, 10, 14));
    }
}
