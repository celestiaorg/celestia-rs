//! Short-lived TLS certificates. Publish the next certificate before switching to it.
use libp2p::multihash::Multihash;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::{
    ServerConfig,
    crypto::ring,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};
use time::OffsetDateTime;

const PERIOD: i64 = 13 * 24 * 3600;
const SKEW: i64 = 3600;

struct Material {
    key: Arc<CertifiedKey>,
    pin: Multihash<64>,
}

fn material(dir: &Path, start: i64) -> io::Result<Material> {
    let path = dir.join(format!("da_webtransport_{start}.json"));
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = KeyPair::generate().map_err(io::Error::other)?;
            // Chromium requires a v3 certificate with extensions for pinned WebTransport.
            // Match the key usage and basic constraints of the released Go libp2p host.
            let mut params = CertificateParams::default();
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.key_usages = vec![
                KeyUsagePurpose::DigitalSignature,
                KeyUsagePurpose::KeyCertSign,
            ];
            params.extended_key_usages = vec![
                ExtendedKeyUsagePurpose::ClientAuth,
                ExtendedKeyUsagePurpose::ServerAuth,
            ];
            params.not_before =
                OffsetDateTime::from_unix_timestamp(start - SKEW).map_err(io::Error::other)?;
            params.not_after = OffsetDateTime::from_unix_timestamp(start + PERIOD + SKEW)
                .map_err(io::Error::other)?;
            let cert = params.self_signed(&key).map_err(io::Error::other)?;
            let bytes = serde_json::to_vec(&(cert.der().as_ref(), key.serialize_der()))?;
            let mut file = tempfile::NamedTempFile::new_in(dir)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            match file.persist_noclobber(&path) {
                Ok(_) => bytes,
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => fs::read(&path)?,
                Err(e) => return Err(e.error),
            }
        }
        Err(e) => return Err(e),
    };
    let (der, key): (Vec<u8>, Vec<u8>) = serde_json::from_slice(&bytes)?;
    let pin = Multihash::wrap(0x12, &hash::sha256(&der)).map_err(io::Error::other)?;
    let key = CertifiedKey::from_der(
        vec![CertificateDer::from(der)],
        PrivatePkcs8KeyDer::from(key).into(),
        &ring::default_provider(),
    )
    .map_err(io::Error::other)?;
    Ok(Material {
        key: Arc::new(key),
        pin,
    })
}

pub(super) struct Epoch {
    pub(super) advertised: [Multihash<64>; 2],
    pub(super) bindings: [Multihash<64>; 3],
    pub(super) remaining: Duration,
    key: Arc<CertifiedKey>,
}

impl Epoch {
    pub(super) fn load(dir: &Path, offset: i64) -> io::Result<Self> {
        Self::at(dir, offset, OffsetDateTime::now_utc().unix_timestamp())
    }

    fn at(dir: &Path, offset: i64, now: i64) -> io::Result<Self> {
        let start = (now - offset).div_euclid(PERIOD) * PERIOD + offset;
        let previous = material(dir, start - PERIOD)?;
        let current = material(dir, start)?;
        let next = material(dir, start + PERIOD)?;
        let old = dir.join(format!("da_webtransport_{}.json", start - 2 * PERIOD));
        if let Err(e) = fs::remove_file(old)
            && e.kind() != io::ErrorKind::NotFound
        {
            return Err(e);
        }
        Ok(Self {
            advertised: [current.pin, next.pin],
            bindings: [previous.pin, current.pin, next.pin],
            remaining: Duration::from_secs((start + PERIOD - now).unsigned_abs()),
            key: current.key,
        })
    }
}
#[derive(Debug)]
pub(super) struct Resolver(RwLock<Arc<CertifiedKey>>);
impl Resolver {
    pub(super) fn tls(epoch: &Epoch) -> io::Result<(Arc<Self>, tobi_quic::ServerTlsConfig)> {
        let resolver = Arc::new(Self(RwLock::new(epoch.key.clone())));
        let mut tls = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_cert_resolver(resolver.clone());
        tls.alpn_protocols = vec![b"h3".to_vec()];
        Ok((resolver, tobi_quic::ServerTlsConfig::new(Arc::new(tls))))
    }
    pub(super) fn rotate(&self, epoch: &Epoch) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = epoch.key.clone();
    }
}
impl ResolvesServerCert for Resolver {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(
            self.0
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_certificates_overlap_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let offset = 1234;
        let boundary = 1400 * PERIOD + offset;
        let before = Epoch::at(dir.path(), offset, boundary - 1).unwrap();
        let after = Epoch::at(dir.path(), offset, boundary).unwrap();
        let restart = Epoch::at(dir.path(), offset, boundary + 1).unwrap();
        assert_eq!(before.remaining, Duration::from_secs(1));
        assert_eq!(before.advertised[1], after.advertised[0]);
        assert_eq!(before.advertised, after.bindings[..2]);
        assert_eq!(after.advertised, restart.advertised);
        assert_eq!(after.key.cert, restart.key.cert);
    }
}
