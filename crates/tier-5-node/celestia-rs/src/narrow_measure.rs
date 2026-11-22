// Measurement-only adaptation of the Apache-2.0 Lumina Fibre verifier at ccdbed9.
use der::asn1::OctetStringRef;
use der::{Decode, Sequence};
use ed25519_dalek::{Signature, VerifyingKey};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio_rustls::rustls::client::Resumption;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::CryptoProvider;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use x509_parser::prelude::parse_x509_certificate;
const IDENTITY_EXTENSION_OID: &str = "1.3.6.1.4.1.66463.1.1";
const SIGN_UNIQUE_ID: &str = "celestia-fibre-tls-v1";
const SIGN_PREFIX: &[u8] = b"celestia-fibre-tls:";
const BINDING_VERSION: i64 = 1;
const MAX_IDENTITY_EXTENSION_SIZE: usize = 8192;
const MAX_PAYLOAD_DER_SIZE: usize = 4096;
const MAX_CERT_VALIDITY_SECONDS: i128 = 31_536_600;
const CLOCK_SKEW_SECONDS: i128 = 300;
#[derive(Sequence)]
struct SignedIdentity<'a> {
    payload: &'a OctetStringRef,
    signature: &'a OctetStringRef,
}

#[derive(Sequence)]
struct BindingPayload<'a> {
    version: i64,
    not_before: i64,
    not_after: i64,
    tls_pub_key: &'a OctetStringRef,
}

#[derive(Debug, thiserror::Error)]
enum FibreCertificateError {
    #[error("peer cert is missing the fibre identity extension")]
    MissingIdentityExtension,
    #[error("peer cert has duplicate fibre identity extensions")]
    DuplicateIdentityExtensions,
    #[error("trailing bytes in identity extension: {0}")]
    IdentityTrailingData(#[source] der::Error),
    #[error("unmarshal identity extension: {0}")]
    IdentityDer(#[source] der::Error),
    #[error("trailing bytes in binding payload: {0}")]
    BindingTrailingData(#[source] der::Error),
    #[error("unmarshal binding payload: {0}")]
    BindingDer(#[source] der::Error),
    #[error("parse peer cert: {0}")]
    CertificateParse(#[source] x509_parser::nom::Err<x509_parser::error::X509Error>),
    #[error("peer cert has {0} trailing bytes")]
    CertificateTrailingData(usize),
    #[error("identity extension size {0} exceeds maximum {MAX_IDENTITY_EXTENSION_SIZE}")]
    IdentityExtensionTooLarge(usize),
    #[error("empty identity payload")]
    EmptyIdentityPayload,
    #[error("identity payload size {0} exceeds maximum {MAX_PAYLOAD_DER_SIZE}")]
    IdentityPayloadTooLarge(usize),
    #[error("empty identity signature")]
    EmptyIdentitySignature,
    #[error("unsupported fibre identity version {0}")]
    UnsupportedIdentityVersion(i64),
    #[error("peer cert signature is invalid: {0}")]
    InvalidIdentitySignature(#[source] ed25519_dalek::SignatureError),
    #[error("peer cert public key does not match signed identity")]
    TlsPublicKeyMismatch,
    #[error("fibre identity validity window is empty: {not_before}..{not_after}")]
    EmptyValidityWindow { not_before: i128, not_after: i128 },
    #[error("fibre identity validity window is {0} seconds, exceeding the maximum")]
    ValidityWindowTooLong(i128),
    #[error(
        "peer fibre identity is not currently valid: now {now}, window {not_before}..{not_after}"
    )]
    OutsideValidityWindow {
        now: i128,
        not_before: i128,
        not_after: i128,
    },
    #[error(
        "certificate validity {certificate_not_before}..{certificate_not_after} does not match signed identity {signed_not_before}..{signed_not_after}"
    )]
    CertificateValidityMismatch {
        signed_not_before: i64,
        signed_not_after: i64,
        certificate_not_before: i64,
        certificate_not_after: i64,
    },
    #[error("parse peer cert extended key usage: {0}")]
    ExtendedKeyUsage(#[source] x509_parser::error::X509Error),
    #[error("peer cert missing serverAuth extended key usage")]
    MissingServerAuth,
}

impl From<FibreCertificateError> for tokio_rustls::rustls::Error {
    fn from(error: FibreCertificateError) -> Self {
        tokio_rustls::rustls::Error::InvalidCertificate(
            tokio_rustls::rustls::CertificateError::Other(tokio_rustls::rustls::OtherError(
                Arc::new(error),
            )),
        )
    }
}

#[derive(Debug)]
struct FibreServerCertVerifier {
    validator_key: VerifyingKey,
    chain_id: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for FibreServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        verify_certificate(
            end_entity.as_ref(),
            &self.validator_key,
            &self.chain_id,
            now,
        )?;

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(
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

fn fibre_tls_config(
    validator_key: VerifyingKey,
    chain_id: String,
    provider: Arc<CryptoProvider>,
    time_provider: Option<Arc<dyn tokio_rustls::rustls::time_provider::TimeProvider>>,
) -> ClientConfig {
    let verifier = FibreServerCertVerifier {
        validator_key,
        chain_id,
        provider: provider.clone(),
    };
    let builder = match time_provider {
        Some(time_provider) => ClientConfig::builder_with_details(provider, time_provider),
        None => ClientConfig::builder_with_provider(provider),
    };
    let mut tls_config = builder
        .with_protocol_versions(&[&tokio_rustls::rustls::version::TLS13])
        .expect("ring and aws-lc-rs crypto providers support TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    tls_config.resumption = Resumption::disabled();
    tls_config
}
fn identity_extension<'a>(
    extensions: &'a [x509_parser::extensions::X509Extension<'a>],
) -> Result<&'a [u8], FibreCertificateError> {
    let mut matching = extensions
        .iter()
        .filter(|extension| extension.oid.to_id_string() == IDENTITY_EXTENSION_OID);
    let extension = matching
        .next()
        .ok_or(FibreCertificateError::MissingIdentityExtension)?;
    if matching.next().is_some() {
        return Err(FibreCertificateError::DuplicateIdentityExtensions);
    }
    Ok(extension.value)
}

fn decode_identity(extension: &[u8]) -> Result<SignedIdentity<'_>, FibreCertificateError> {
    SignedIdentity::from_der(extension).map_err(|error| {
        if matches!(error.kind(), der::ErrorKind::TrailingData { .. }) {
            FibreCertificateError::IdentityTrailingData(error)
        } else {
            FibreCertificateError::IdentityDer(error)
        }
    })
}

fn decode_binding(payload: &[u8]) -> Result<BindingPayload<'_>, FibreCertificateError> {
    BindingPayload::from_der(payload).map_err(|error| {
        if matches!(error.kind(), der::ErrorKind::TrailingData { .. }) {
            FibreCertificateError::BindingTrailingData(error)
        } else {
            FibreCertificateError::BindingDer(error)
        }
    })
}

fn verify_certificate(
    cert_der: &[u8],
    validator_key: &VerifyingKey,
    chain_id: &str,
    now: UnixTime,
) -> Result<(), FibreCertificateError> {
    let (remaining, cert) =
        parse_x509_certificate(cert_der).map_err(FibreCertificateError::CertificateParse)?;
    if !remaining.is_empty() {
        return Err(FibreCertificateError::CertificateTrailingData(
            remaining.len(),
        ));
    }

    let extension = identity_extension(cert.extensions())?;
    if extension.len() > MAX_IDENTITY_EXTENSION_SIZE {
        return Err(FibreCertificateError::IdentityExtensionTooLarge(
            extension.len(),
        ));
    }

    let identity = decode_identity(extension)?;
    let payload = identity.payload.as_bytes();
    if payload.is_empty() {
        return Err(FibreCertificateError::EmptyIdentityPayload);
    }
    if payload.len() > MAX_PAYLOAD_DER_SIZE {
        return Err(FibreCertificateError::IdentityPayloadTooLarge(
            payload.len(),
        ));
    }
    if identity.signature.as_bytes().is_empty() {
        return Err(FibreCertificateError::EmptyIdentitySignature);
    }

    let binding = decode_binding(payload)?;
    if binding.version != BINDING_VERSION {
        return Err(FibreCertificateError::UnsupportedIdentityVersion(
            binding.version,
        ));
    }

    let mut sign_input = Vec::with_capacity(SIGN_PREFIX.len() + payload.len());
    sign_input.extend_from_slice(SIGN_PREFIX);
    sign_input.extend_from_slice(payload);
    let signed_bytes =
        ctypes::canonical::raw_bytes_message_sign_bytes(chain_id, SIGN_UNIQUE_ID, &sign_input)
            .map_err(|_| FibreCertificateError::EmptyIdentityPayload)?;
    let signature = Signature::from_slice(identity.signature.as_bytes())
        .map_err(FibreCertificateError::InvalidIdentitySignature)?;
    validator_key
        .verify_strict(&signed_bytes, &signature)
        .map_err(FibreCertificateError::InvalidIdentitySignature)?;

    if cert.tbs_certificate.subject_pki.raw != binding.tls_pub_key.as_bytes() {
        return Err(FibreCertificateError::TlsPublicKeyMismatch);
    }

    let not_before = i128::from(binding.not_before);
    let not_after = i128::from(binding.not_after);
    if not_after <= not_before {
        return Err(FibreCertificateError::EmptyValidityWindow {
            not_before,
            not_after,
        });
    }
    if not_after - not_before > MAX_CERT_VALIDITY_SECONDS {
        return Err(FibreCertificateError::ValidityWindowTooLong(
            not_after - not_before,
        ));
    }
    let now = i128::from(now.as_secs());
    if now < not_before - CLOCK_SKEW_SECONDS || now > not_after + CLOCK_SKEW_SECONDS {
        return Err(FibreCertificateError::OutsideValidityWindow {
            now,
            not_before,
            not_after,
        });
    }

    if cert.validity().not_before.timestamp() != binding.not_before
        || cert.validity().not_after.timestamp() != binding.not_after
    {
        return Err(FibreCertificateError::CertificateValidityMismatch {
            signed_not_before: binding.not_before,
            signed_not_after: binding.not_after,
            certificate_not_before: cert.validity().not_before.timestamp(),
            certificate_not_after: cert.validity().not_after.timestamp(),
        });
    }

    let has_server_auth = cert
        .extended_key_usage()
        .map_err(FibreCertificateError::ExtendedKeyUsage)?
        .is_some_and(|usage| usage.value.server_auth);
    if !has_server_auth {
        return Err(FibreCertificateError::MissingServerAuth);
    }

    Ok(())
}

pub(super) async fn download() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use celestia_proto::celestia::fibre::v1::{DownloadShardRequest, DownloadShardResponse};
    use std::collections::BTreeMap;
    use tonic::transport::Endpoint;
    let args: Vec<_> = std::env::args().skip(2).collect();
    let [addr, public, chain, blob, output] = &args[..] else {
        anyhow::bail!("expected address key chain blob output")
    };
    let addr = addr.parse::<std::net::SocketAddr>()?;
    ensure!(addr.ip().is_loopback() && !chain.is_empty());
    let public = VerifyingKey::from_bytes(&hex::decode(public)?.as_slice().try_into()?)?;
    let blob = hex::decode(blob)?;
    ensure!(blob.len() == 33 && blob[0] == 0);
    let root: [u8; 32] = blob[1..].try_into()?;
    let mut config = fibre_tls_config(
        public,
        chain.clone(),
        Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()),
        None,
    );
    config.alpn_protocols = vec![b"h2".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let channel = Endpoint::from_shared(format!("http://{addr}"))?
        .connect_with_connector(tower::service_fn(move |_| {
            let connector = connector.clone();
            async move {
                let tcp = tokio::net::TcpStream::connect(addr).await?;
                let tls = connector
                    .connect(ServerName::IpAddress(addr.ip().into()), tcp)
                    .await?;
                Ok::<_, std::io::Error>(TokioIo::new(tls))
            }
        }))
        .await?;
    let mut client = tonic::client::Grpc::new(channel).max_decoding_message_size(64 << 20);
    client.ready().await?;
    let response: tonic::Response<DownloadShardResponse> = client
        .unary(
            tonic::Request::new(DownloadShardRequest { blob_id: blob }),
            axum::http::uri::PathAndQuery::from_static(
                "/celestia.fibre.v1.Fibre/DownloadShard",
            ),
            tonic::codec::ProstCodec::default(),
        )
        .await?;
    let shard = response.into_inner().shard.context("missing shard")?;
    let k = 4096;
    ensure!(shard.rlcs.len() == k * 16 && (k..=4 * k).contains(&shard.rows.len()));
    let rlcs = shard
        .rlcs
        .as_chunks::<16>()
        .0
        .iter()
        .map(rsema1d::GF128::from_bytes)
        .collect::<Vec<_>>();
    let width = shard.rows[0].data.len();
    ensure!((64..=(1 << 15)).contains(&width));
    let params = rsema1d::Parameters::new(k, 3 * k, width)?;
    let context = rsema1d::VerificationContext::new(&rlcs, &params)?;
    let mut rows = BTreeMap::new();
    for row in shard.rows {
        let index = row.index as usize;
        ensure!(index < 4 * k && row.data.len() == width && !rows.contains_key(&index));
        let proof = rsema1d::RowProof {
            index,
            row: row.data.into(),
            row_proof: row
                .proof
                .iter()
                .map(|p| p.as_slice().try_into())
                .collect::<Result<_, _>>()?,
        };
        rsema1d::verify_row_with_context(&proof, &root, &context)?;
        rows.insert(index, proof.row);
    }
    let indexes: Vec<_> = rows.keys().take(k).copied().collect();
    let values: Vec<&[u8]> = rows.values().take(k).map(|b| b.as_ref()).collect();
    let matrix = rsema1d::reconstruct(&values, &indexes, &params)?;
    let data = matrix.as_row_major();
    ensure!(data[0] == 0);
    let len = u32::from_be_bytes(data[1..5].try_into()?) as usize;
    ensure!((1..=(1 << 27) - 5).contains(&len));
    std::fs::write(output, data.get(5..5 + len).context("incomplete payload")?)?;
    Ok(())
}
