//! Go-compatible HS256 permissions and durable token revocation.
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hmac::{Hmac, Mac as _};
use rand::{RngCore as _, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use tokio::sync::watch;

const ZERO: &str = "0001-01-01T00:00:00Z";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Claims {
    allow: Vec<String>,
    nonce: String,
    expires_at: String,
}

pub(crate) struct Auth {
    key: [u8; 32],
    revoked: PathBuf,
    changed: watch::Sender<()>,
}

fn secret(home: &Path) -> Result<[u8; 32]> {
    let dir = home.join("config");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("da-jwt.key");
    match std::fs::read(&path) {
        Ok(bytes) => {
            return bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid DA token key length"));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut key = [0; 32];
    OsRng.try_fill_bytes(&mut key)?;
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    file.write_all(&key)?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(&path) {
        Ok(_) => {
            #[cfg(unix)]
            std::fs::File::open(dir)?.sync_all()?;
            Ok(key)
        }
        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => std::fs::read(path)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid DA token key length")),
        Err(e) => Err(e.error.into()),
    }
}

fn sign(key: &[u8; 32], allow: Vec<String>, ttl: i64) -> Result<String> {
    let mut nonce = [0; 32];
    OsRng.try_fill_bytes(&mut nonce)?;
    let expires_at = if ttl == 0 {
        ZERO.to_owned()
    } else {
        let now = light::verifier::now().unix_nanos() + i128::from(ttl);
        crate::util::rfc3339_nano(
            i64::try_from(now.div_euclid(1_000_000_000))?,
            u32::try_from(now.rem_euclid(1_000_000_000))?,
        )
    };
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Claims {
        allow,
        nonce: STANDARD.encode(nonce),
        expires_at,
    })?);
    let data = format!("{header}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
    mac.update(data.as_bytes());
    Ok(format!(
        "{data}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

/// Issue an operator token for the given permission level.
pub fn operator_token(home: &Path, level: &str, ttl_seconds: u64) -> Result<String> {
    let perms = ["public", "read", "write", "admin"];
    let end = perms
        .iter()
        .position(|p| *p == level)
        .context("unknown permission level")?;
    let ttl = i64::try_from(ttl_seconds)?
        .checked_mul(1_000_000_000)
        .context("token expiry out of range")?;
    sign(
        &secret(home)?,
        perms[..=end].iter().map(|s| (*s).to_owned()).collect(),
        ttl,
    )
}

impl Auth {
    pub(super) fn open(home: &Path) -> Result<Self> {
        let revoked = home.join("data/da-revoked");
        std::fs::create_dir_all(&revoked)?;
        Ok(Self {
            key: secret(home)?,
            revoked,
            changed: watch::channel(()).0,
        })
    }

    fn claims(&self, token: &str) -> Result<(Claims, PathBuf)> {
        let (input, sig) = token.rsplit_once('.').context("malformed token")?;
        let (header, payload) = input.split_once('.').context("malformed token")?;
        let signature = URL_SAFE_NO_PAD.decode(sig)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key)?;
        mac.update(input.as_bytes());
        mac.verify_slice(&signature)?;
        let metadata: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header)?)?;
        ensure!(metadata["alg"] == "HS256", "unsupported token algorithm");
        let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
        let marker = self.revoked.join(hex::encode(hash::sha256(&signature)));
        Ok((claims, marker))
    }

    pub(super) fn verify(&self, token: &str) -> Result<Vec<String>> {
        let (claims, marker) = self.claims(token)?;
        if claims.expires_at != ZERO {
            let (sec, nanos) = sdk::time::parse_rfc3339_nano(claims.expires_at.as_bytes())
                .context("invalid token expiry")?;
            ensure!(
                i128::from(sec) * 1_000_000_000 + i128::from(nanos)
                    >= light::verifier::now().unix_nanos(),
                "token expired"
            );
        }
        ensure!(!marker.try_exists()?, "token revoked");
        Ok(claims.allow)
    }

    pub(super) async fn revoked(&self, mut changed: watch::Receiver<()>, token: Option<&str>) {
        loop {
            if changed.changed().await.is_err() || token.is_some_and(|t| self.verify(t).is_err()) {
                return;
            }
        }
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub(super) fn admin(&self, method: &str, args: &[Value]) -> Result<Value> {
        let token = || {
            args.first()
                .and_then(Value::as_str)
                .context("invalid token argument")
        };
        match (method, args.len()) {
            ("node.AuthVerify", 1) => Ok(serde_json::to_value(self.verify(token()?)?)?),
            ("node.AuthRevoke", 1) => {
                let (_, marker) = self.claims(token()?)?;
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(marker)?;
                file.sync_all()?;
                self.changed.send_replace(());
                #[cfg(unix)]
                std::fs::File::open(&self.revoked)?.sync_all()?;
                Ok(Value::Null)
            }
            ("node.AuthNew", 1) | ("node.AuthNewWithExpiry", 2) => {
                let perms = serde_json::from_value(args[0].clone())?;
                let ttl = if args.len() == 2 {
                    args[1].as_i64().context("invalid TTL")?
                } else {
                    0
                };
                Ok(Value::String(sign(&self.key, perms, ttl)?))
            }
            _ => anyhow::bail!("invalid auth parameters"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expire_and_revoke_operator_tokens_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let auth = Auth::open(root.path()).unwrap();
        let token = operator_token(root.path(), "read", 0).unwrap();
        assert_eq!(auth.verify(&token).unwrap(), ["public", "read"]);
        let expired = sign(&auth.key, vec!["read".to_owned()], -1_000_000_000).unwrap();
        assert!(auth.verify(&expired).is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(Auth::open(other.path()).unwrap().verify(&token).is_err());
        auth.admin("node.AuthRevoke", &[Value::String(token.clone())])
            .unwrap();
        drop(auth);
        assert!(Auth::open(root.path()).unwrap().verify(&token).is_err());
    }
}
