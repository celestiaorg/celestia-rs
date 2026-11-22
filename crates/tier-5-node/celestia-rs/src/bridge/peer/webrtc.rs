//! Preserve the WebRTC DTLS certificate hash advertised by a DA peer.
use anyhow::{Context, Result};
use libp2p::{Multiaddr, multiaddr::Protocol};
use libp2p_webrtc::tokio::Certificate;
use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

pub(super) fn for_listeners(
    peer_key: &Path,
    addresses: &[Multiaddr],
) -> Result<Option<Certificate>> {
    if !addresses
        .iter()
        .any(|addr| addr.iter().any(|p| p == Protocol::WebRTCDirect))
    {
        return Ok(None);
    }
    // WebRTC's DTLS stack uses rustls defaults. Both ring and AWS-LC can be
    // present through optional DA transports. Preserve an existing selection.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = peer_key.parent().context("missing DA identity directory")?;
    let path = dir.join("da_webrtc.pem");
    match fs::read_to_string(&path) {
        Ok(pem) => return Ok(Some(Certificate::from_pem(&pem)?)),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let certificate = Certificate::generate(&mut webrtc_rand::rng())?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(certificate.serialize_pem().as_bytes())?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(&path) {
        Ok(_) => Ok(Some(certificate)),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            Ok(Some(Certificate::from_pem(&fs::read_to_string(path)?)?))
        }
        Err(error) => Err(error.into()),
    }
}
