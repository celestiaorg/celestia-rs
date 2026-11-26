//! Fibre gRPC TLS identity: `fibre/internal/tlsid` of celestia-app v11.0.0
//! (normative spec `specs/src/fibre_tls_identity.md`).
//!
//! A Fibre server presents a self-signed certificate for an ephemeral
//! Ed25519 key. A custom, non-critical extension (OID
//! `1.3.6.1.4.1.66463.1.1`) carries
//!
//! ```text
//! SignedIdentity ::= SEQUENCE { payload OCTET STRING, signature OCTET STRING }
//! BindingPayload ::= SEQUENCE { version INTEGER, notBefore INTEGER,
//!                               notAfter INTEGER, tlsPubKey OCTET STRING }
//! ```
//!
//! where `payload` is the DER of the `BindingPayload` and `signature` is the
//! validator consensus key's `SignRawBytes(chainID, "celestia-fibre-tls-v1",
//! "celestia-fibre-tls:" || payload)`. [`verify_cert`] runs Go's
//! `verifyCertAt` checks with Go's error text.
//!
//! Certificates are encoded by hand so they are byte-identical to Go's
//! `x509.CreateCertificate` output for the same inputs (Ed25519 signatures
//! are deterministic). [`ServerCert`] holds the DER certificate and a PKCS#8
//! v1 private key, as rustls `CertificateDer` / `PrivateKeyDer::Pkcs8`
//! expect. The server must offer TLS 1.3 only, as Go does.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// `SignUniqueID`: privval domain-separation tag of the endorsement.
pub const SIGN_UNIQUE_ID: &str = "celestia-fibre-tls-v1";
/// `SignPrefix`: prepended to the payload DER before signing.
pub const SIGN_PREFIX: &str = "celestia-fibre-tls:";
/// `core.RawBytesSignBytesPrefix`.
pub const RAW_BYTES_SIGN_BYTES_PREFIX: &str = "COMET::RAW_BYTES::SIGN";
/// `bindingVersion`.
pub const BINDING_VERSION: i64 = 1;
/// `MaxPayloadDERSize`.
pub const MAX_PAYLOAD_DER_SIZE: usize = 4096;
/// `MaxIdentityExtensionSize`.
pub const MAX_IDENTITY_EXTENSION_SIZE: usize = 8192;
/// `CertValidity` in seconds (365 days).
pub const CERT_VALIDITY_SECS: i64 = 365 * 24 * 3600;
/// `clockSkew` in seconds: NotBefore backdating and verification tolerance.
pub const CLOCK_SKEW_SECS: i64 = 5 * 60;
/// `MaxCertValidity` in seconds.
pub const MAX_CERT_VALIDITY_SECS: i64 = CERT_VALIDITY_SECS + 2 * CLOCK_SKEW_SECS;
/// The dotted form of `signedIDExtensionOID`.
pub const SIGNED_ID_EXTENSION_OID: &str = "1.3.6.1.4.1.66463.1.1";
/// The certificate subject and issuer common name.
pub const CERT_COMMON_NAME: &str = "celestia-fibre";

/// DER content octets of `signedIDExtensionOID`.
const SIGNED_ID_OID_DER: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x84, 0x87, 0x1f, 0x01, 0x01];
const OID_ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const OID_EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
/// id-kp-serverAuth.
pub const OID_KP_SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
/// id-kp-clientAuth.
pub const OID_KP_CLIENT_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02];

// --- DER writing -------------------------------------------------------------

fn der_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        #[allow(clippy::cast_possible_truncation)]
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        #[allow(clippy::cast_possible_truncation)]
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    der_len(&mut out, content.len());
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// Minimal two's-complement INTEGER.
fn der_i64(v: i64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut start = 0;
    while start < 7 {
        let redundant = (b[start] == 0x00 && b[start + 1] & 0x80 == 0)
            || (b[start] == 0xff && b[start + 1] & 0x80 != 0);
        if !redundant {
            break;
        }
        start += 1;
    }
    tlv(0x02, &b[start..])
}

/// Non-negative INTEGER from a big-endian magnitude.
fn der_uint(magnitude: &[u8]) -> Vec<u8> {
    let skip = magnitude.iter().take_while(|&&b| b == 0).count();
    let m = &magnitude[skip..];
    let mut content = Vec::with_capacity(m.len() + 1);
    if m.is_empty() || m[0] & 0x80 != 0 {
        content.push(0);
    }
    content.extend_from_slice(m);
    tlv(0x02, &content)
}

/// Days since 1970-01-01 to (year, month, day) (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

struct Civil {
    year: i64,
    month: u32,
    day: u32,
    hour: i64,
    minute: i64,
    second: i64,
}

fn civil(unix: i64) -> Civil {
    let (year, month, day) = civil_from_days(unix.div_euclid(86_400));
    let secs = unix.rem_euclid(86_400);
    Civil {
        year,
        month,
        day,
        hour: secs / 3600,
        minute: secs % 3600 / 60,
        second: secs % 60,
    }
}

/// X.509 `Time`: UTCTime for 1950..2050, else GeneralizedTime (Go's rule).
fn der_time(unix: i64) -> Vec<u8> {
    let c = civil(unix);
    if (1950..2050).contains(&c.year) {
        let s = format!(
            "{:02}{:02}{:02}{:02}{:02}{:02}Z",
            c.year % 100,
            c.month,
            c.day,
            c.hour,
            c.minute,
            c.second
        );
        tlv(0x17, s.as_bytes())
    } else {
        let s = format!(
            "{:04}{:02}{:02}{:02}{:02}{:02}Z",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        );
        tlv(0x18, s.as_bytes())
    }
}

/// Go `time.Format(time.RFC3339)` of a UTC instant.
fn rfc3339(unix: i64) -> String {
    let c = civil(unix);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// Go `time.Duration.String` for a whole, non-negative number of seconds.
fn go_duration(secs: i128) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m{}s", secs / 3600, secs % 3600 / 60, secs % 60)
    }
}

/// `x509.MarshalPKIXPublicKey` of an Ed25519 public key.
#[must_use]
pub fn ed25519_spki_der(public_key: &[u8; 32]) -> Vec<u8> {
    let mut bits = vec![0u8];
    bits.extend_from_slice(public_key);
    seq(&[&seq(&[&tlv(0x06, OID_ED25519)]), &tlv(0x03, &bits)])
}

/// PKCS#8 v1 `PrivateKeyInfo` of an Ed25519 seed (RFC 8410).
#[must_use]
pub fn ed25519_pkcs8_der(seed: &[u8; 32]) -> Vec<u8> {
    seq(&[
        &der_i64(0),
        &seq(&[&tlv(0x06, OID_ED25519)]),
        &tlv(0x04, &tlv(0x04, seed)),
    ])
}

/// DER of `BindingPayload`.
#[must_use]
pub fn encode_binding_payload(
    version: i64,
    not_before: i64,
    not_after: i64,
    tls_pub_spki: &[u8],
) -> Vec<u8> {
    seq(&[
        &der_i64(version),
        &der_i64(not_before),
        &der_i64(not_after),
        &tlv(0x04, tls_pub_spki),
    ])
}

/// DER of `SignedIdentity` (the extension value).
#[must_use]
pub fn encode_signed_identity(payload: &[u8], signature: &[u8]) -> Vec<u8> {
    seq(&[&tlv(0x04, payload), &tlv(0x04, signature)])
}

/// `signedBytes`: [`SIGN_PREFIX`] followed by the payload DER.
#[must_use]
pub fn signed_bytes(payload_der: &[u8]) -> Vec<u8> {
    [SIGN_PREFIX.as_bytes(), payload_der].concat()
}

fn put_uvarint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        #[allow(clippy::cast_possible_truncation)]
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    #[allow(clippy::cast_possible_truncation)]
    out.push(v as u8);
}

/// CometBFT `types.RawBytesMessageSignBytes`: the prefix followed by the
/// length-delimited `privval.SignRawBytesRequest{chain_id, raw_bytes,
/// unique_id}`.
pub fn raw_bytes_message_sign_bytes(
    chain_id: &str,
    unique_id: &str,
    raw: &[u8],
) -> Result<Vec<u8>, String> {
    if chain_id.is_empty() {
        return Err("chainID cannot be empty".into());
    }
    if unique_id.is_empty() {
        return Err("uniqueID cannot be empty".into());
    }
    if raw.is_empty() {
        return Err("rawBytes cannot be empty".into());
    }
    let mut body = Vec::new();
    for (tag, b) in [
        (0x0a, chain_id.as_bytes()),
        (0x12, raw),
        (0x1a, unique_id.as_bytes()),
    ] {
        body.push(tag);
        put_uvarint(&mut body, b.len() as u64);
        body.extend_from_slice(b);
    }
    let mut out = RAW_BYTES_SIGN_BYTES_PREFIX.as_bytes().to_vec();
    put_uvarint(&mut out, body.len() as u64);
    out.extend_from_slice(&body);
    Ok(out)
}

// --- Certificate building ------------------------------------------------------

/// The validator's raw-bytes signer (`core.PrivValidator.SignRawBytes`).
pub trait RawBytesSigner {
    /// Signs `RawBytesMessageSignBytes(chain_id, unique_id, raw)` with the
    /// consensus key.
    fn sign_raw_bytes(
        &self,
        chain_id: &str,
        unique_id: &str,
        raw: &[u8],
    ) -> Result<Vec<u8>, String>;
}

impl<F> RawBytesSigner for F
where
    F: Fn(&str, &str, &[u8]) -> Result<Vec<u8>, String>,
{
    fn sign_raw_bytes(
        &self,
        chain_id: &str,
        unique_id: &str,
        raw: &[u8],
    ) -> Result<Vec<u8>, String> {
        self(chain_id, unique_id, raw)
    }
}

/// A local Ed25519 consensus key signing raw bytes (CometBFT `FilePV` /
/// `MockPV` behavior).
#[derive(Clone)]
pub struct Ed25519RawBytesSigner {
    private_key: crypto::ed25519::PrivateKeyBytes,
}

impl Ed25519RawBytesSigner {
    /// A signer for the key with this 32-byte seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            private_key: crypto::ed25519::private_key_from_seed(seed),
        }
    }

    /// The consensus public key.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&self.private_key[32..]);
        pk
    }
}

impl fmt::Debug for Ed25519RawBytesSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ed25519RawBytesSigner")
            .field("public_key", &hex::encode(self.public_key()))
            .finish_non_exhaustive()
    }
}

impl RawBytesSigner for Ed25519RawBytesSigner {
    fn sign_raw_bytes(
        &self,
        chain_id: &str,
        unique_id: &str,
        raw: &[u8],
    ) -> Result<Vec<u8>, String> {
        let msg = raw_bytes_message_sign_bytes(chain_id, unique_id, raw)?;
        crypto::ed25519::sign(&self.private_key, &msg)
            .map(|s| s.to_vec())
            .map_err(|e| e.to_string())
    }
}

/// A server certificate and its private key.
#[derive(Clone)]
pub struct ServerCert {
    /// The self-signed X.509 certificate (DER).
    pub cert_der: Vec<u8>,
    /// The ephemeral Ed25519 TLS key as PKCS#8 v1 DER.
    pub private_key_pkcs8_der: Vec<u8>,
    /// The TLS public key.
    pub tls_public_key: [u8; 32],
    /// Signed and certificate NotBefore (unix seconds).
    pub not_before: i64,
    /// Signed and certificate NotAfter (unix seconds).
    pub not_after: i64,
}

impl fmt::Debug for ServerCert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerCert")
            .field("tls_public_key", &hex::encode(self.tls_public_key))
            .field("not_before", &self.not_before)
            .field("not_after", &self.not_after)
            .finish_non_exhaustive()
    }
}

/// A certificate building failure; the text is Go's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildCertError {
    /// Empty chain ID.
    #[error("chain ID must not be empty")]
    EmptyChainId,
    /// The OS random source failed.
    #[error("generate TLS keypair: {0}")]
    Random(String),
    /// The signer failed.
    #[error("sign TLS binding: {0}")]
    Sign(String),
}

/// The fields of the certificate template (Go's `x509.Certificate`).
struct CertTemplate<'a> {
    serial: &'a [u8],
    not_before: i64,
    not_after: i64,
    ext_key_usage: &'a [&'a [u8]],
    identity_extension: Option<&'a [u8]>,
}

/// `x509.CreateCertificate(rand, tmpl, tmpl, pub, priv)` for an Ed25519
/// self-signed certificate, byte-identical to Go's encoding.
fn create_certificate(t: &CertTemplate<'_>, seed: &[u8; 32]) -> Result<Vec<u8>, String> {
    let private_key = crypto::ed25519::private_key_from_seed(seed);
    let mut public_key = [0u8; 32];
    public_key.copy_from_slice(&private_key[32..]);

    let alg = seq(&[&tlv(0x06, OID_ED25519)]);
    let name = seq(&[&tlv(
        0x31,
        &seq(&[
            &tlv(0x06, OID_COMMON_NAME),
            &tlv(0x13, CERT_COMMON_NAME.as_bytes()),
        ]),
    )]);
    let validity = seq(&[&der_time(t.not_before), &der_time(t.not_after)]);

    // KeyUsage digitalSignature, critical.
    let key_usage = seq(&[
        &tlv(0x06, OID_KEY_USAGE),
        &tlv(0x01, &[0xff]),
        &tlv(0x04, &tlv(0x03, &[0x07, 0x80])),
    ]);
    let mut extensions = vec![key_usage];
    if !t.ext_key_usage.is_empty() {
        let oids: Vec<Vec<u8>> = t.ext_key_usage.iter().map(|o| tlv(0x06, o)).collect();
        let refs: Vec<&[u8]> = oids.iter().map(Vec::as_slice).collect();
        extensions.push(seq(&[
            &tlv(0x06, OID_EXT_KEY_USAGE),
            &tlv(0x04, &seq(&refs)),
        ]));
    }
    if let Some(value) = t.identity_extension {
        extensions.push(seq(&[&tlv(0x06, SIGNED_ID_OID_DER), &tlv(0x04, value)]));
    }
    let ext_refs: Vec<&[u8]> = extensions.iter().map(Vec::as_slice).collect();

    let tbs = seq(&[
        &tlv(0xa0, &der_i64(2)),
        &der_uint(t.serial),
        &alg,
        &name,
        &validity,
        &name,
        &ed25519_spki_der(&public_key),
        &tlv(0xa3, &seq(&ext_refs)),
    ]);
    let signature = crypto::ed25519::sign(&private_key, &tbs).map_err(|e| e.to_string())?;
    let mut sig_bits = vec![0u8];
    sig_bits.extend_from_slice(&signature);
    Ok(seq(&[&tbs, &alg, &tlv(0x03, &sig_bits)]))
}

/// `buildServerCertWithKey`: the certificate for a given TLS key seed,
/// serial (big-endian magnitude), issue time and validity, with
/// `NotBefore = now - 5m` and `NotAfter = now + validity` (unix seconds).
pub fn build_server_cert_with(
    signer: &dyn RawBytesSigner,
    chain_id: &str,
    now: i64,
    validity_secs: i64,
    tls_seed: &[u8; 32],
    serial: &[u8],
) -> Result<ServerCert, BuildCertError> {
    if chain_id.is_empty() {
        return Err(BuildCertError::EmptyChainId);
    }
    let private_key = crypto::ed25519::private_key_from_seed(tls_seed);
    let mut tls_public_key = [0u8; 32];
    tls_public_key.copy_from_slice(&private_key[32..]);
    let spki = ed25519_spki_der(&tls_public_key);

    let not_before = now - CLOCK_SKEW_SECS;
    let not_after = now + validity_secs;
    let payload = encode_binding_payload(BINDING_VERSION, not_before, not_after, &spki);
    let signature = signer
        .sign_raw_bytes(chain_id, SIGN_UNIQUE_ID, &signed_bytes(&payload))
        .map_err(BuildCertError::Sign)?;
    let extension = encode_signed_identity(&payload, &signature);

    let cert_der = create_certificate(
        &CertTemplate {
            serial,
            not_before,
            not_after,
            ext_key_usage: &[OID_KP_SERVER_AUTH, OID_KP_CLIENT_AUTH],
            identity_extension: Some(&extension),
        },
        tls_seed,
    )
    .map_err(BuildCertError::Sign)?;
    Ok(ServerCert {
        cert_der,
        private_key_pkcs8_der: ed25519_pkcs8_der(tls_seed),
        tls_public_key,
        not_before,
        not_after,
    })
}

/// `BuildServerCert`: a fresh Ed25519 TLS key and a random 128-bit serial,
/// issued now for [`CERT_VALIDITY_SECS`]. The signer is called once.
pub fn build_server_cert(
    signer: &dyn RawBytesSigner,
    chain_id: &str,
) -> Result<ServerCert, BuildCertError> {
    if chain_id.is_empty() {
        return Err(BuildCertError::EmptyChainId);
    }
    let mut seed = [0u8; 32];
    let mut serial = [0u8; 16];
    getrandom::fill(&mut seed).map_err(|e| BuildCertError::Random(e.to_string()))?;
    getrandom::fill(&mut serial).map_err(|e| BuildCertError::Random(e.to_string()))?;
    build_server_cert_with(
        signer,
        chain_id,
        unix_now(),
        CERT_VALIDITY_SECS,
        &seed,
        &serial,
    )
}

/// The current time in whole unix seconds (Go's `Truncate(time.Second)`).
#[must_use]
pub fn unix_now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => {
            let d = e.duration();
            -i64::try_from(d.as_secs()).unwrap_or(i64::MAX) - i64::from(d.subsec_nanos() > 0)
        }
    }
}

// --- Verification ----------------------------------------------------------------

/// A validator consensus public key (`crypto.PubKey.VerifySignature`).
pub trait ConsensusPubKey {
    /// Whether `sig` is a valid signature of `msg`.
    fn verify_signature(&self, msg: &[u8], sig: &[u8]) -> bool;
}

/// A CometBFT Ed25519 consensus key (ZIP-215 verification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ed25519ConsensusKey(pub [u8; 32]);

impl ConsensusPubKey for Ed25519ConsensusKey {
    fn verify_signature(&self, msg: &[u8], sig: &[u8]) -> bool {
        let Ok(sig) = <[u8; 64]>::try_from(sig) else {
            return false;
        };
        crypto::ed25519::verify(&self.0, msg, &sig)
    }
}

/// A rejected peer certificate. `Display` is Go's error text; [`code`]
/// is the stable error name of the golden vectors.
///
/// [`code`]: VerifyError::code
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// The peer sent no certificate.
    #[error("peer presented no certificate")]
    NoCertificate,
    /// The leaf is not a parsable certificate.
    #[error("parse peer cert: {0}")]
    ParseCert(String),
    /// Empty chain ID.
    #[error("chain ID must not be empty")]
    EmptyChainId,
    /// No identity extension.
    #[error("peer cert is missing the fibre identity extension")]
    ExtensionMissing,
    /// The extension exceeds [`MAX_IDENTITY_EXTENSION_SIZE`].
    #[error("identity extension size {0} exceeds maximum {MAX_IDENTITY_EXTENSION_SIZE}")]
    ExtensionTooLarge(usize),
    /// The extension is not a `SignedIdentity`.
    #[error("unmarshal identity extension: {0}")]
    ExtensionMalformed(String),
    /// Bytes after the `SignedIdentity`.
    #[error("trailing bytes in identity extension")]
    ExtensionTrailingData,
    /// Empty payload.
    #[error("empty identity payload")]
    PayloadEmpty,
    /// The payload exceeds [`MAX_PAYLOAD_DER_SIZE`].
    #[error("identity payload size {0} exceeds maximum {MAX_PAYLOAD_DER_SIZE}")]
    PayloadTooLarge(usize),
    /// Empty signature.
    #[error("empty identity signature")]
    SignatureEmpty,
    /// The payload is not a `BindingPayload`.
    #[error("unmarshal binding payload: {0}")]
    PayloadMalformed(String),
    /// Bytes after the `BindingPayload`.
    #[error("trailing bytes in binding payload")]
    BindingTrailingData,
    /// A binding version other than 1.
    #[error("unsupported fibre identity version {0}")]
    UnsupportedVersion(i64),
    /// The signed-bytes envelope could not be built.
    #[error("compute signed bytes: {0}")]
    SignedBytes(String),
    /// The endorsement does not verify under the expected key.
    #[error("peer cert signature is invalid")]
    SignatureInvalid,
    /// The certificate key is not the signed TLS key.
    #[error("peer cert public key does not match signed identity")]
    TlsKeyMismatch,
    /// notAfter <= notBefore.
    #[error("fibre identity validity window is empty")]
    WindowEmpty,
    /// The window exceeds [`MAX_CERT_VALIDITY_SECS`].
    #[error("fibre identity validity window {window} exceeds maximum {max}")]
    WindowTooLong {
        /// Go duration text of the window.
        window: String,
        /// Go duration text of the maximum.
        max: String,
    },
    /// The verification time is outside the window (with skew).
    #[error("peer fibre identity not valid at {0}")]
    OutsideValidityWindow(String),
    /// The certificate validity differs from the signed window.
    #[error("certificate validity does not match signed identity")]
    CertWindowMismatch,
    /// No serverAuth extended key usage.
    #[error("peer cert missing serverAuth extended key usage")]
    EkuMissing,
}

impl VerifyError {
    /// The error name used by Go's golden vectors (`vectorErrSubstrings`).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoCertificate => "no_certificate",
            Self::ParseCert(_) => "parse_cert",
            Self::EmptyChainId => "empty_chain_id",
            Self::ExtensionMissing => "extension_missing",
            Self::ExtensionTooLarge(_) => "extension_too_large",
            Self::ExtensionMalformed(_) => "extension_malformed",
            Self::ExtensionTrailingData => "extension_trailing_data",
            Self::PayloadEmpty => "payload_empty",
            Self::PayloadTooLarge(_) => "payload_too_large",
            Self::SignatureEmpty => "signature_empty",
            Self::PayloadMalformed(_) => "payload_malformed",
            Self::BindingTrailingData => "binding_trailing_data",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::SignedBytes(_) => "signed_bytes",
            Self::SignatureInvalid => "signature_invalid",
            Self::TlsKeyMismatch => "tls_key_mismatch",
            Self::WindowEmpty => "window_empty",
            Self::WindowTooLong { .. } => "window_too_long",
            Self::OutsideValidityWindow(_) => "outside_validity_window",
            Self::CertWindowMismatch => "cert_window_mismatch",
            Self::EkuMissing => "eku_missing",
        }
    }
}

/// A strict DER reader with Go `encoding/asn1` semantics for the two
/// identity structures.
mod asn1 {
    pub(super) struct Element<'a> {
        pub(super) tag: u8,
        pub(super) content: &'a [u8],
    }

    /// One element at the start of `b` and the bytes after it.
    pub(super) fn element(b: &[u8]) -> Result<(Element<'_>, &[u8]), String> {
        let syntax = |m: &str| format!("asn1: syntax error: {m}");
        let (&tag, rest) = b
            .split_first()
            .ok_or_else(|| syntax("sequence truncated"))?;
        if tag & 0x1f == 0x1f {
            return Err(
                "asn1: structure error: high-tag-number form is not used by the fibre identity"
                    .into(),
            );
        }
        let (&first, mut rest) = rest
            .split_first()
            .ok_or_else(|| syntax("truncated tag or length"))?;
        let len = if first & 0x80 == 0 {
            usize::from(first)
        } else {
            let n = usize::from(first & 0x7f);
            if n == 0 {
                return Err(syntax("indefinite length found (not DER)"));
            }
            let mut len = 0usize;
            for _ in 0..n {
                let (&byte, r) = rest
                    .split_first()
                    .ok_or_else(|| syntax("truncated tag or length"))?;
                rest = r;
                if len >= 1 << 23 {
                    return Err("asn1: structure error: length too large".into());
                }
                len = (len << 8) | usize::from(byte);
                if len == 0 {
                    return Err("asn1: structure error: superfluous leading zeros in length".into());
                }
            }
            if len < 0x80 {
                return Err("asn1: structure error: non-minimal length".into());
            }
            len
        };
        if len > rest.len() {
            return Err(syntax("data truncated"));
        }
        let (content, after) = rest.split_at(len);
        Ok((Element { tag, content }, after))
    }

    fn expect(b: &[u8], tag: u8) -> Result<(&[u8], &[u8]), String> {
        let (el, rest) = element(b)?;
        if el.tag != tag {
            return Err(format!(
                "asn1: structure error: tags don't match ({} vs {{class:{} tag:{} length:{} isCompound:{}}})",
                tag & 0x1f,
                el.tag >> 6,
                el.tag & 0x1f,
                el.content.len(),
                el.tag & 0x20 != 0
            ));
        }
        Ok((el.content, rest))
    }

    /// A SEQUENCE: its content and the bytes after it.
    pub(super) fn sequence(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
        expect(b, 0x30)
    }

    /// An OCTET STRING field.
    pub(super) fn octet_string(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
        expect(b, 0x04)
    }

    /// An INTEGER field that must fit an `int64` (`parseInt64`).
    pub(super) fn int64(b: &[u8]) -> Result<(i64, &[u8]), String> {
        let (c, rest) = expect(b, 0x02)?;
        if c.is_empty() {
            return Err("asn1: structure error: empty integer".into());
        }
        if c.len() > 1 && ((c[0] == 0 && c[1] & 0x80 == 0) || (c[0] == 0xff && c[1] & 0x80 != 0)) {
            return Err("asn1: structure error: integer not minimally-encoded".into());
        }
        if c.len() > 8 {
            return Err("asn1: structure error: integer too large".into());
        }
        let mut v: i64 = if c[0] & 0x80 != 0 { -1 } else { 0 };
        for &byte in c {
            v = (v << 8) | i64::from(byte);
        }
        Ok((v, rest))
    }
}

struct Binding<'a> {
    version: i64,
    not_before: i64,
    not_after: i64,
    tls_pub_key: &'a [u8],
}

/// A parsed `signedIdentity` and the bytes after it.
#[derive(Debug)]
struct SignedIdentity<'a> {
    payload: &'a [u8],
    signature: &'a [u8],
    rest: &'a [u8],
}

/// `asn1.Unmarshal` into `signedIdentity`. Extra elements inside the
/// SEQUENCE are ignored, as in Go.
fn parse_signed_identity(b: &[u8]) -> Result<SignedIdentity<'_>, String> {
    let (inner, rest) = asn1::sequence(b)?;
    let (payload, inner) = asn1::octet_string(inner)?;
    let (signature, _) = asn1::octet_string(inner)?;
    Ok(SignedIdentity {
        payload,
        signature,
        rest,
    })
}

/// `asn1.Unmarshal` into `bindingPayload`.
fn parse_binding(b: &[u8]) -> Result<(Binding<'_>, &[u8]), String> {
    let (inner, rest) = asn1::sequence(b)?;
    let (version, inner) = asn1::int64(inner)?;
    let (not_before, inner) = asn1::int64(inner)?;
    let (not_after, inner) = asn1::int64(inner)?;
    let (tls_pub_key, _) = asn1::octet_string(inner)?;
    Ok((
        Binding {
            version,
            not_before,
            not_after,
            tls_pub_key,
        },
        rest,
    ))
}

/// `VerifyPeer`: verifies the leaf of the peer's chain (`raw_certs[0]`).
pub fn verify_peer(
    raw_certs: &[&[u8]],
    expected: &dyn ConsensusPubKey,
    chain_id: &str,
    now: i64,
) -> Result<(), VerifyError> {
    let leaf = raw_certs.first().ok_or(VerifyError::NoCertificate)?;
    verify_cert(leaf, expected, chain_id, now)
}

/// `verifyCertAt`: accepts `cert_der` iff its identity extension carries a
/// valid endorsement by `expected` (the validator's consensus key from the
/// validator set) for `chain_id`, binding the certificate's key and
/// validity, at unix time `now`. Hostnames and CAs are not checked; the
/// peer's possession of the key is proven by TLS 1.3 `CertificateVerify`.
pub fn verify_cert(
    cert_der: &[u8],
    expected: &dyn ConsensusPubKey,
    chain_id: &str,
    now: i64,
) -> Result<(), VerifyError> {
    let (rest, cert) = x509_parser::parse_x509_certificate(cert_der)
        .map_err(|e| VerifyError::ParseCert(format!("x509: {e}")))?;
    if !rest.is_empty() {
        return Err(VerifyError::ParseCert("x509: trailing data".into()));
    }
    if chain_id.is_empty() {
        return Err(VerifyError::EmptyChainId);
    }

    let ext = cert
        .extensions()
        .iter()
        .find(|e| e.oid.as_bytes() == SIGNED_ID_OID_DER)
        .ok_or(VerifyError::ExtensionMissing)?;
    let ext_bytes = ext.value;
    if ext_bytes.len() > MAX_IDENTITY_EXTENSION_SIZE {
        return Err(VerifyError::ExtensionTooLarge(ext_bytes.len()));
    }

    let SignedIdentity {
        payload,
        signature,
        rest,
    } = parse_signed_identity(ext_bytes).map_err(VerifyError::ExtensionMalformed)?;
    if !rest.is_empty() {
        return Err(VerifyError::ExtensionTrailingData);
    }
    if payload.is_empty() {
        return Err(VerifyError::PayloadEmpty);
    }
    if payload.len() > MAX_PAYLOAD_DER_SIZE {
        return Err(VerifyError::PayloadTooLarge(payload.len()));
    }
    if signature.is_empty() {
        return Err(VerifyError::SignatureEmpty);
    }

    let (bp, rest) = parse_binding(payload).map_err(VerifyError::PayloadMalformed)?;
    if !rest.is_empty() {
        return Err(VerifyError::BindingTrailingData);
    }
    if bp.version != BINDING_VERSION {
        return Err(VerifyError::UnsupportedVersion(bp.version));
    }

    let signed = raw_bytes_message_sign_bytes(chain_id, SIGN_UNIQUE_ID, &signed_bytes(payload))
        .map_err(VerifyError::SignedBytes)?;
    if !expected.verify_signature(&signed, signature) {
        return Err(VerifyError::SignatureInvalid);
    }

    if cert.public_key().raw != bp.tls_pub_key {
        return Err(VerifyError::TlsKeyMismatch);
    }

    if bp.not_after <= bp.not_before {
        return Err(VerifyError::WindowEmpty);
    }
    let window = i128::from(bp.not_after) - i128::from(bp.not_before);
    if window > i128::from(MAX_CERT_VALIDITY_SECS) {
        return Err(VerifyError::WindowTooLong {
            window: go_duration(window),
            max: go_duration(i128::from(MAX_CERT_VALIDITY_SECS)),
        });
    }
    let now128 = i128::from(now);
    if now128 < i128::from(bp.not_before) - i128::from(CLOCK_SKEW_SECS)
        || now128 > i128::from(bp.not_after) + i128::from(CLOCK_SKEW_SECS)
    {
        return Err(VerifyError::OutsideValidityWindow(rfc3339(now)));
    }

    let validity = cert.validity();
    if validity.not_before.timestamp() != bp.not_before
        || validity.not_after.timestamp() != bp.not_after
    {
        return Err(VerifyError::CertWindowMismatch);
    }

    let server_auth = cert
        .extended_key_usage()
        .ok()
        .flatten()
        .is_some_and(|eku| eku.value.server_auth);
    if !server_auth {
        return Err(VerifyError::EkuMissing);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct VectorFile {
        constants: Constants,
        cases: Vec<Case>,
    }
    #[derive(Deserialize)]
    struct Constants {
        extension_oid: String,
        sign_unique_id: String,
        sign_prefix: String,
        envelope_prefix: String,
        binding_version: i64,
        max_identity_extension_size: usize,
        max_payload_der_size: usize,
        max_cert_validity_seconds: i64,
        clock_skew_seconds: i64,
    }
    #[derive(Deserialize)]
    struct Binding {
        version: i64,
        not_before: i64,
        not_after: i64,
    }
    #[derive(Deserialize)]
    struct Expected {
        valid: bool,
        #[serde(default)]
        error: String,
    }
    #[derive(Deserialize)]
    struct Case {
        name: String,
        chain_id: String,
        consensus_priv_seed: String,
        consensus_pub: String,
        tls_priv_seed: String,
        tls_pub_raw: String,
        tls_pub_spki_der: String,
        binding: Option<Binding>,
        payload_der: String,
        sign_input: String,
        signed_bytes: String,
        signature: String,
        extension_der: String,
        cert_serial: String,
        cert_der: String,
        verifier_chain_id: String,
        verifier_consensus_pub: String,
        verify_at: i64,
        expected: Expected,
    }

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }
    fn h32(s: &str) -> [u8; 32] {
        h(s).try_into().expect("32 bytes")
    }

    fn vectors() -> VectorFile {
        let path = format!(
            "{}/../../../conformance/fixtures/fibre-server/identity_vectors.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).expect("fixture")).expect("json")
    }

    fn serial_bytes(decimal: &str) -> Vec<u8> {
        let v: u64 = decimal.parse().expect("serial");
        v.to_be_bytes().to_vec()
    }

    #[test]
    fn constants_match_go() {
        let c = vectors().constants;
        assert_eq!(c.extension_oid, SIGNED_ID_EXTENSION_OID);
        assert_eq!(c.sign_unique_id, SIGN_UNIQUE_ID);
        assert_eq!(c.sign_prefix, SIGN_PREFIX);
        assert_eq!(c.envelope_prefix, RAW_BYTES_SIGN_BYTES_PREFIX);
        assert_eq!(c.binding_version, BINDING_VERSION);
        assert_eq!(c.max_identity_extension_size, MAX_IDENTITY_EXTENSION_SIZE);
        assert_eq!(c.max_payload_der_size, MAX_PAYLOAD_DER_SIZE);
        assert_eq!(c.max_cert_validity_seconds, MAX_CERT_VALIDITY_SECS);
        assert_eq!(c.clock_skew_seconds, CLOCK_SKEW_SECS);
    }

    /// Producer direction: payload, sign input, signed bytes, signature,
    /// extension and (for every case) the certificate are reproduced
    /// byte-for-byte.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn producer_matches_go_vectors() {
        let f = vectors();
        assert_eq!(f.cases.len(), 21);
        for c in &f.cases {
            let name = c.name.as_str();
            let cons = Ed25519RawBytesSigner::from_seed(&h32(&c.consensus_priv_seed));
            assert_eq!(hex::encode(cons.public_key()), c.consensus_pub, "{name}");
            let tls_seed = h32(&c.tls_priv_seed);
            let tls_pub = crypto::ed25519::private_key_from_seed(&tls_seed);
            assert_eq!(hex::encode(&tls_pub[32..]), c.tls_pub_raw, "{name}");
            let spki = ed25519_spki_der(&h32(&c.tls_pub_raw));
            assert_eq!(hex::encode(&spki), c.tls_pub_spki_der, "{name}");

            if let Some(b) = &c.binding {
                let mut payload =
                    encode_binding_payload(b.version, b.not_before, b.not_after, &spki);
                if name == "binding_trailing_data" {
                    payload.push(0);
                }
                assert_eq!(hex::encode(&payload), c.payload_der, "{name} payload");
            }
            if !c.payload_der.is_empty() || name == "payload_empty" {
                let payload = h(&c.payload_der);
                let input = signed_bytes(&payload);
                assert_eq!(hex::encode(&input), c.sign_input, "{name} sign input");
                let env =
                    raw_bytes_message_sign_bytes(&c.chain_id, SIGN_UNIQUE_ID, &input).expect("env");
                assert_eq!(hex::encode(&env), c.signed_bytes, "{name} signed bytes");
                let sig = cons
                    .sign_raw_bytes(&c.chain_id, SIGN_UNIQUE_ID, &input)
                    .expect("sign");
                match name {
                    "signature_tampered" => {
                        let mut t = sig.clone();
                        t[0] ^= 1;
                        assert_eq!(hex::encode(t), c.signature, "{name}");
                    }
                    "signature_empty" => assert_eq!(c.signature, ""),
                    _ => assert_eq!(hex::encode(&sig), c.signature, "{name} signature"),
                }
                let mut ext = encode_signed_identity(&payload, &h(&c.signature));
                if name == "extension_trailing_data" {
                    ext.push(0);
                }
                assert_eq!(hex::encode(&ext), c.extension_der, "{name} extension");
            }

            // Re-encode the certificate from the parsed Go certificate's
            // fields and the case's keys.
            let der = h(&c.cert_der);
            let (_, cert) = x509_parser::parse_x509_certificate(&der).expect("cert");
            let v = cert.validity();
            let eku = cert
                .extended_key_usage()
                .expect("eku")
                .expect("eku present")
                .value;
            let mut ekus: Vec<&[u8]> = Vec::new();
            if eku.server_auth {
                ekus.push(OID_KP_SERVER_AUTH);
            }
            if eku.client_auth {
                ekus.push(OID_KP_CLIENT_AUTH);
            }
            let ext = h(&c.extension_der);
            let serial = serial_bytes(&c.cert_serial);
            // tls_key_mismatch presents the key of seed 0xa5 (Go's certTLSSeed).
            let cert_seed = if name == "tls_key_mismatch" {
                [0xa5; 32]
            } else {
                tls_seed
            };
            let rebuilt = create_certificate(
                &CertTemplate {
                    serial: &serial,
                    not_before: v.not_before.timestamp(),
                    not_after: v.not_after.timestamp(),
                    ext_key_usage: &ekus,
                    identity_extension: (name != "extension_missing").then_some(ext.as_slice()),
                },
                &cert_seed,
            )
            .expect("cert");
            assert_eq!(hex::encode(&rebuilt), c.cert_der, "{name} certificate");

            // Production-path cases come out of build_server_cert_with.
            if matches!(
                name,
                "valid"
                    | "wrong_validator"
                    | "wrong_chain_id"
                    | "window_too_long"
                    | "expired"
                    | "not_yet_valid"
            ) {
                let b = c.binding.as_ref().expect("binding");
                let issue = b.not_before + CLOCK_SKEW_SECS;
                let sc = build_server_cert_with(
                    &cons,
                    &c.chain_id,
                    issue,
                    b.not_after - issue,
                    &tls_seed,
                    &serial,
                )
                .expect("build");
                assert_eq!(
                    hex::encode(&sc.cert_der),
                    c.cert_der,
                    "{name} production cert"
                );
                assert_eq!(sc.not_before, b.not_before);
                assert_eq!(sc.not_after, b.not_after);
            }
        }
    }

    /// Verifier direction: Go's verdict for every committed certificate.
    #[test]
    fn verifier_matches_go_vectors() {
        for c in vectors().cases {
            let key = Ed25519ConsensusKey(h32(&c.verifier_consensus_pub));
            let res = verify_cert(&h(&c.cert_der), &key, &c.verifier_chain_id, c.verify_at);
            if c.expected.valid {
                assert_eq!(res, Ok(()), "{}", c.name);
            } else {
                let err = res.expect_err(&c.name);
                assert_eq!(err.code(), c.expected.error, "{}: {err}", c.name);
            }
        }
    }

    #[test]
    fn verifier_error_text() {
        let f = vectors();
        let case = |n: &str| f.cases.iter().find(|c| c.name == n).expect("case");
        let check = |n: &str, want: &str| {
            let c = case(n);
            let key = Ed25519ConsensusKey(h32(&c.verifier_consensus_pub));
            let err =
                verify_cert(&h(&c.cert_der), &key, &c.verifier_chain_id, c.verify_at).expect_err(n);
            assert_eq!(err.to_string(), want, "{n}");
        };
        check(
            "extension_too_large",
            "identity extension size 8394 exceeds maximum 8192",
        );
        check(
            "window_too_long",
            "fibre identity validity window 8761h15m0s exceeds maximum 8760h10m0s",
        );
        check(
            "expired",
            "peer fibre identity not valid at 2026-01-01T00:00:00Z",
        );
        check("unknown_version", "unsupported fibre identity version 2");
        check(
            "payload_too_large",
            "identity payload size 4224 exceeds maximum 4096",
        );
        let valid = case("valid");
        let key = Ed25519ConsensusKey(h32(&valid.verifier_consensus_pub));
        assert_eq!(
            verify_cert(&h(&valid.cert_der), &key, "", valid.verify_at),
            Err(VerifyError::EmptyChainId)
        );
        assert_eq!(
            verify_peer(&[], &key, "c", 0),
            Err(VerifyError::NoCertificate)
        );
        assert!(matches!(
            verify_peer(&[&[0x30, 0x00]], &key, "c", 0),
            Err(VerifyError::ParseCert(_))
        ));
    }

    #[test]
    fn asn1_reader_is_strict_der() {
        // Go accepts extra elements inside the SEQUENCE.
        let mut inner = [tlv(0x04, b"p"), tlv(0x04, b"s"), tlv(0x05, &[])].concat();
        let outer = tlv(0x30, &inner);
        let id = parse_signed_identity(&outer).expect("ok");
        assert_eq!(
            (id.payload, id.signature, id.rest),
            (&b"p"[..], &b"s"[..], &b""[..])
        );
        // Indefinite and non-minimal lengths, non-minimal integers.
        assert!(
            parse_signed_identity(&[0x30, 0x80, 0, 0])
                .unwrap_err()
                .contains("indefinite length")
        );
        assert!(
            parse_signed_identity(&[0x30, 0x81, 0x01, 0x00])
                .unwrap_err()
                .contains("non-minimal length")
        );
        assert!(
            asn1::int64(&[0x02, 0x02, 0x00, 0x01])
                .unwrap_err()
                .contains("minimally")
        );
        assert!(
            asn1::int64(&[0x02, 0x00])
                .unwrap_err()
                .contains("empty integer")
        );
        assert_eq!(asn1::int64(&der_i64(-129)).expect("int").0, -129);
        assert_eq!(asn1::int64(&der_i64(i64::MIN)).expect("int").0, i64::MIN);
        // Missing field.
        inner.truncate(3);
        assert!(
            parse_signed_identity(&tlv(0x30, &inner))
                .unwrap_err()
                .contains("truncated")
        );
    }

    #[test]
    fn times_and_durations() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(der_time(2_524_608_000)[0], 0x18); // 2050-01-01: GeneralizedTime
        assert_eq!(der_time(2_524_607_999)[0], 0x17);
        assert_eq!(go_duration(31_536_600), "8760h10m0s");
        assert_eq!(go_duration(61), "1m1s");
        assert_eq!(go_duration(5), "5s");
    }

    #[test]
    fn build_server_cert_verifies() {
        let signer = Ed25519RawBytesSigner::from_seed(&[7; 32]);
        let key = Ed25519ConsensusKey(signer.public_key());
        let sc = build_server_cert(&signer, "celestia").expect("build");
        assert_eq!(
            sc.not_after - sc.not_before,
            CERT_VALIDITY_SECS + CLOCK_SKEW_SECS
        );
        assert_eq!(
            verify_cert(&sc.cert_der, &key, "celestia", unix_now()),
            Ok(())
        );
        assert_eq!(
            verify_cert(&sc.cert_der, &key, "other", unix_now()),
            Err(VerifyError::SignatureInvalid)
        );
        assert_eq!(
            build_server_cert(&signer, "").unwrap_err().to_string(),
            "chain ID must not be empty"
        );
        let failing =
            |_: &str, _: &str, _: &[u8]| -> Result<Vec<u8>, String> { Err("hsm offline".into()) };
        assert_eq!(
            build_server_cert(&failing, "c").unwrap_err().to_string(),
            "sign TLS binding: hsm offline"
        );
    }

    /// The certificate and key serve a TLS 1.3-only rustls server, and a
    /// client that verifies the peer with `verify_cert` completes the
    /// handshake (TLS 1.3 CertificateVerify proves possession of the key).
    #[test]
    fn rustls_tls13_handshake() {
        use rustls::client::danger::{
            HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
        };
        use rustls::pki_types::{
            CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
        };
        use rustls::{DigitallySignedStruct, SignatureScheme};
        use std::sync::Arc;

        #[derive(Debug)]
        struct FibreVerifier {
            key: Ed25519ConsensusKey,
            provider: Arc<rustls::crypto::CryptoProvider>,
        }
        impl ServerCertVerifier for FibreVerifier {
            fn verify_server_cert(
                &self,
                end_entity: &CertificateDer<'_>,
                _: &[CertificateDer<'_>],
                _: &ServerName<'_>,
                _: &[u8],
                _: UnixTime,
            ) -> Result<ServerCertVerified, rustls::Error> {
                verify_cert(end_entity, &self.key, "celestia", unix_now())
                    .map(|()| ServerCertVerified::assertion())
                    .map_err(|e| rustls::Error::General(e.to_string()))
            }
            fn verify_tls12_signature(
                &self,
                _: &[u8],
                _: &CertificateDer<'_>,
                _: &DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                Err(rustls::Error::General("TLS 1.2 is not allowed".into()))
            }
            fn verify_tls13_signature(
                &self,
                message: &[u8],
                cert: &CertificateDer<'_>,
                dss: &DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                rustls::crypto::verify_tls13_signature(
                    message,
                    cert,
                    dss,
                    &self.provider.signature_verification_algorithms,
                )
            }
            fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
                vec![SignatureScheme::ED25519]
            }
        }

        let signer = Ed25519RawBytesSigner::from_seed(&[9; 32]);
        let sc = build_server_cert(&signer, "celestia").expect("build");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server_cfg = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("tls13")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(sc.cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(sc.private_key_pkcs8_der.clone())),
            )
            .expect("server config");
        let client_cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("tls13")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(FibreVerifier {
                key: Ed25519ConsensusKey(signer.public_key()),
                provider,
            }))
            .with_no_client_auth();

        let mut client = rustls::ClientConnection::new(
            Arc::new(client_cfg),
            ServerName::try_from("validator.example").expect("name"),
        )
        .expect("client");
        let mut server = rustls::ServerConnection::new(Arc::new(server_cfg)).expect("server");
        for _ in 0..10 {
            let mut buf = Vec::new();
            client.write_tls(&mut buf).expect("client write");
            if !buf.is_empty() {
                server.read_tls(&mut buf.as_slice()).expect("server read");
                server.process_new_packets().expect("server process");
            }
            let mut buf = Vec::new();
            server.write_tls(&mut buf).expect("server write");
            if !buf.is_empty() {
                client.read_tls(&mut buf.as_slice()).expect("client read");
                client.process_new_packets().expect("client process");
            }
            if !client.is_handshaking() && !server.is_handshaking() {
                break;
            }
        }
        assert!(!client.is_handshaking() && !server.is_handshaking());
        assert_eq!(
            client.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
    }
}
