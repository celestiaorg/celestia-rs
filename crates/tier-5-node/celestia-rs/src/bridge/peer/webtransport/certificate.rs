//! Authenticate the pinned TLS certificate before the Noise peer identity exchange.
use libp2p::multihash::Multihash;
use rustls::{
    DigitallySignedStruct, Error, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{collections::HashSet, sync::Arc};
use x509_parser::{
    prelude::{FromDer, X509Certificate},
    public_key::PublicKey,
};

#[derive(Debug)]
pub(super) struct Verifier {
    pins: HashSet<Multihash<64>>,
    provider: Arc<CryptoProvider>,
}
impl Verifier {
    pub(super) fn config(pins: HashSet<Multihash<64>>) -> Result<rustls::ClientConfig, Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Ok(
            rustls::ClientConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(&[&rustls::version::TLS13])?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(Self { pins, provider }))
                .with_no_client_auth(),
        )
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let invalid =
            || Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure);
        if !self
            .pins
            .iter()
            .any(|pin| pin.code() == 0x12 && pin.digest() == hash::sha256(leaf.as_ref()))
        {
            return Err(invalid());
        }
        let (rest, cert) = X509Certificate::from_der(leaf.as_ref()).map_err(|_| invalid())?;
        let validity = cert.validity();
        let start = validity.not_before.timestamp();
        let end = validity.not_after.timestamp();
        let now = i64::try_from(now.as_secs()).map_err(|_| invalid())?;
        if !rest.is_empty()
            || !(start..=end).contains(&now)
            || end.saturating_sub(start) > 14 * 24 * 3600
            || matches!(
                cert.public_key().parsed().map_err(|_| invalid())?,
                PublicKey::RSA(_)
            )
        {
            return Err(invalid());
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("WebTransport requires TLS 1.3".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
