//! TLS credentials for libp2p WebSocket endpoints.
use anyhow::{Context, Result};
use libp2p::websocket::tls::{Certificate, Config, PrivateKey};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::path::PathBuf;

pub(super) fn server_tls() -> Result<Option<Config>> {
    let Some(path) = std::env::var_os("CELESTIA_TLS_PATH").filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let cert = path.join("cert.pem");
    let key = path.join("key.pem");
    if !cert.try_exists()? || !key.try_exists()? {
        return Ok(None);
    }
    let certs = CertificateDer::pem_file_iter(cert)?
        .map(|cert| cert.map(|bytes| Certificate::new(bytes.to_vec())))
        .collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    // The upstream constructor expects a recognized DER encoding.
    PrivateKeyDer::try_from(key.secret_der())
        .map_err(anyhow::Error::msg)
        .context("invalid WebSocket TLS private key")?;
    Ok(Some(Config::new(
        PrivateKey::new(key.secret_der().to_vec()),
        certs,
    )?))
}
