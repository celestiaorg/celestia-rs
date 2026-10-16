//! `celestia-rs keys …`: the Go `celestia-appd keys` commands on the same
//! keyring directories (`--keyring-backend test|file`), with Go's output
//! formats (`text` YAML records, `json`).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use keyring::{Backend, KeyRecord, Keyring, Prompt};
use serde_json::{Value, json};

use crate::util;

/// Bech32 prefixes (`celestia`, `celestiavaloper`, `celestiavalcons`).
pub const ACC_PREFIX: &str = "celestia";
/// Validator operator prefix.
pub const VAL_PREFIX: &str = "celestiavaloper";
/// Validator consensus prefix.
pub const CONS_PREFIX: &str = "celestiavalcons";

/// The keyring backend: the flag, else `client.toml`'s `keyring-backend`,
/// else `test`.
pub fn backend(home: &Path, flag: Option<&str>) -> anyhow::Result<Backend> {
    let name = if let Some(f) = flag {
        f.to_owned()
    } else {
        let files = crate::config::ConfigFiles::load(home)?;
        let b = files.string(crate::config::File::Client, "keyring-backend");
        if b.is_empty() { "test".into() } else { b }
    };
    name.parse::<Backend>().map_err(|e| anyhow!("{e}"))
}

/// Opens the keyring of `home` (`--keyring-dir` overrides the base
/// directory); `file` passphrases are read from stdin like Go's prompts,
/// without echo on a terminal.
#[must_use]
pub fn open(home: &Path, keyring_dir: Option<&Path>, backend: Backend) -> Keyring {
    let base = keyring_dir.unwrap_or(home);
    let kr = Keyring::new(base, backend);
    if let Ok(p) = std::env::var("CELESTIA_RS_KEYRING_PASSPHRASE") {
        return kr.with_password(&p);
    }
    kr.with_password_source(Box::new(|p: Prompt| {
        let msg = match p {
            Prompt::Unlock | Prompt::New => "Enter keyring passphrase:",
            Prompt::Confirm => "Re-enter keyring passphrase:",
        };
        util::prompt_password(msg).map_err(|e| keyring::Error::Io(e.to_string()))
    }))
}

/// Go's `KeyOutput` (`keys show`, `keys list`, `keys add`).
#[must_use]
pub fn key_output(k: &KeyRecord, prefix: &str, mnemonic: Option<&str>) -> Value {
    let mut v = json!({
        "name": k.name,
        "type": k.kind.to_string(),
        "address": k.bech32(prefix),
        "pubkey": k.pubkey_json(),
    });
    if let Some(m) = mnemonic {
        v["mnemonic"] = json!(m);
    }
    v
}

fn yaml_quote(s: &str) -> String {
    if s.contains(['{', '}', '"', ':', '\'', '#']) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s.to_owned()
    }
}

/// Go's text output: a YAML list of key records (fields sorted).
#[must_use]
pub fn yaml_records(records: &[Value]) -> String {
    let mut out = String::new();
    for r in records {
        let Value::Object(m) = r else { continue };
        let mut first = true;
        for (k, v) in m {
            let s = v.as_str().unwrap_or_default();
            out.push_str(if first { "- " } else { "  " });
            let _ = writeln!(out, "{k}: {}", yaml_quote(s));
            first = false;
        }
    }
    out
}

/// Prints records in `output` format (`text` or `json`).
pub fn print_records(records: &[Value], output: &str, single: bool) {
    if output == "json" {
        let v = if single {
            records.first().cloned().unwrap_or(Value::Null)
        } else {
            Value::Array(records.to_vec())
        };
        util::out(v.to_string());
    } else {
        util::out(yaml_records(records));
    }
}

/// `keys add` options.
#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    /// Key name.
    pub name: String,
    /// Read a mnemonic from stdin.
    pub recover: bool,
    /// HD account.
    pub account: u32,
    /// HD index.
    pub index: u32,
    /// Full HD path (overrides account / index).
    pub hd_path: Option<String>,
    /// Public key JSON / base64 (offline key).
    pub pubkey: Option<String>,
    /// Don't print the mnemonic.
    pub no_backup: bool,
    /// BIP-39 passphrase prompt (`--interactive`): unsupported, empty.
    pub output: String,
}

/// `keys add`.
pub fn add(kr: &mut Keyring, o: &AddOptions) -> anyhow::Result<()> {
    if let Some(pk) = &o.pubkey {
        use base64::Engine as _;
        let v: Value = serde_json::from_str(pk).unwrap_or_else(|_| json!({"key": pk}));
        let b64 = v["key"]
            .as_str()
            .ok_or_else(|| anyhow!("invalid --pubkey {pk}"))?;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("pubkey base64")?;
        let pub_key: [u8; 33] = raw
            .try_into()
            .map_err(|_| anyhow!("pubkey must be a 33-byte compressed secp256k1 key"))?;
        let k = kr
            .add_offline(&o.name, pub_key)
            .map_err(|e| anyhow!("{e}"))?;
        print_records(&[key_output(&k, ACC_PREFIX, None)], &o.output, true);
        return Ok(());
    }
    let path = o
        .hd_path
        .clone()
        .unwrap_or_else(|| keyring::hd::hd_path(keyring::hd::COIN_TYPE, o.account, o.index));
    let (mnemonic, show) = if o.recover {
        let m = util::prompt("> Enter your bip39 mnemonic")?;
        keyring::hd::validate_mnemonic(&m).map_err(|_| anyhow!("invalid mnemonic"))?;
        (m, false)
    } else {
        (keyring::hd::new_mnemonic(), !o.no_backup)
    };
    let k = kr
        .add_mnemonic(&o.name, &mnemonic, "", &path)
        .map_err(|e| anyhow!("{e}"))?;
    if o.output == "json" {
        let out = key_output(&k, ACC_PREFIX, show.then_some(mnemonic.as_str()));
        util::out(out.to_string());
    } else {
        util::out(yaml_records(&[key_output(&k, ACC_PREFIX, None)]));
        if show {
            util::err(format!(
                "\n**Important** write this mnemonic phrase in a safe place.\nIt is the only way to recover your account if you ever forget your password.\n\n{mnemonic}\n"
            ));
        }
    }
    Ok(())
}

/// Resolves a key by name or bech32 / hex address.
pub fn find(kr: &mut Keyring, name_or_addr: &str) -> anyhow::Result<KeyRecord> {
    match kr.key(name_or_addr) {
        Ok(k) => Ok(k),
        Err(keyring::Error::NotFound(_)) => {
            let addr = encoding::bech32::decode(name_or_addr)
                .ok()
                .map(|(_, b)| b)
                .or_else(|| hex::decode(name_or_addr).ok())
                .ok_or_else(|| anyhow!("{name_or_addr}: key not found"))?;
            kr.key_by_address(&addr).map_err(|e| anyhow!("{e}"))
        }
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// `keys show` options.
#[derive(Debug, Clone, Default)]
pub struct ShowOptions {
    /// Print the address only.
    pub address: bool,
    /// Print the public key only.
    pub pubkey: bool,
    /// `acc`, `val` or `cons`.
    pub bech: String,
    /// Output format.
    pub output: String,
}

/// `keys show`.
pub fn show(kr: &mut Keyring, name: &str, o: &ShowOptions) -> anyhow::Result<()> {
    let k = find(kr, name)?;
    let prefix = match o.bech.as_str() {
        "" | "acc" => ACC_PREFIX,
        "val" => VAL_PREFIX,
        "cons" => CONS_PREFIX,
        other => bail!("invalid Bech32 prefix encoding provided: {other}"),
    };
    if o.address {
        util::out(k.bech32(prefix));
    } else if o.pubkey {
        util::out(k.pubkey_json());
    } else {
        print_records(&[key_output(&k, prefix, None)], &o.output, true);
    }
    Ok(())
}

/// `keys list`.
pub fn list(kr: &mut Keyring, output: &str, names_only: bool) -> anyhow::Result<()> {
    let keys = kr.list().map_err(|e| anyhow!("{e}"))?;
    if names_only {
        for k in keys {
            util::out(k.name);
        }
        return Ok(());
    }
    let recs: Vec<Value> = keys
        .iter()
        .map(|k| key_output(k, ACC_PREFIX, None))
        .collect();
    print_records(&recs, output, false);
    Ok(())
}

/// `keys export` (armored, passphrase from stdin) or
/// `--unarmored-hex --unsafe`.
pub fn export(kr: &mut Keyring, name: &str, unarmored_hex: bool) -> anyhow::Result<()> {
    if unarmored_hex {
        let k = find(kr, name)?;
        let s = k
            .secret
            .ok_or_else(|| anyhow!("{name}: only local keys can be exported"))?;
        util::out(hex::encode(s));
        return Ok(());
    }
    let pass = util::prompt_password("Enter passphrase to encrypt the exported key:")?;
    let armor = kr.export_armor(name, &pass).map_err(|e| anyhow!("{e}"))?;
    util::out(armor);
    Ok(())
}

/// `keys import <name> <keyfile>` (passphrase from stdin).
pub fn import(kr: &mut Keyring, name: &str, file: &Path) -> anyhow::Result<()> {
    let armor =
        std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let pass = util::prompt_password("Enter passphrase to decrypt your key:")?;
    kr.import_armor(name, &armor, &pass)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// `keys import-hex <name> <hex>`.
pub fn import_hex(kr: &mut Keyring, name: &str, hex_key: &str) -> anyhow::Result<()> {
    let raw = hex::decode(hex_key.trim()).context("private key must be hex")?;
    let secret: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow!("private key must be 32 bytes"))?;
    kr.add_secret(name, secret).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// `keys delete` (`-y` skips the confirmation).
pub fn delete(kr: &mut Keyring, name: &str, yes: bool) -> anyhow::Result<()> {
    find(kr, name)?;
    if !yes {
        let a = util::prompt("Key reference will be deleted. Continue? [y/N]: ")?;
        if !matches!(a.trim(), "y" | "Y" | "yes") {
            bail!("aborted");
        }
    }
    kr.delete(name).map_err(|e| anyhow!("{e}"))?;
    util::err("Key deleted forever (uh oh!)");
    Ok(())
}
