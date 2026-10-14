//! Authenticated encryption for peer connections (port of
//! `p2p/conn/secret_connection.go`, the STS protocol).
//!
//! Handshake: exchange X25519 ephemeral keys (delimited `BytesValue`s), sort
//! them into a Merlin transcript, derive the ChaCha20-Poly1305 send/receive
//! keys with HKDF-SHA256 from the DH secret, extract a 32-byte challenge from
//! the transcript, and exchange `AuthSigMessage`s (ed25519 signature over the
//! challenge) over the encrypted channel. Data then flows in sealed frames of
//! `4-byte LE length || up to 1024 bytes || zero padding` (1028 bytes) plus a
//! 16-byte tag.

use std::fmt;
use std::io;

use celestia_core_proto::tendermint::{crypto as pbcrypto, p2p as pb};
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use hkdf::Hkdf;
use merlin::Transcript;
use prost::Message as _;
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use x25519_dalek::{PublicKey as XPublicKey, StaticSecret};

use crate::key::{NodeId, NodeKey};

/// Length prefix inside a frame.
pub const DATA_LEN_SIZE: usize = 4;
/// Maximum plaintext bytes per frame.
pub const DATA_MAX_SIZE: usize = 1024;
/// Plaintext frame size (length prefix + data).
pub const TOTAL_FRAME_SIZE: usize = DATA_MAX_SIZE + DATA_LEN_SIZE;
/// Poly1305 tag size.
pub const AEAD_SIZE_OVERHEAD: usize = 16;
/// Sealed frame size on the wire.
pub const SEALED_FRAME_SIZE: usize = TOTAL_FRAME_SIZE + AEAD_SIZE_OVERHEAD;

/// Max size of a handshake message (Go uses `1024*1024`).
const HANDSHAKE_MAX_MSG: usize = 1024 * 1024;

const KEY_AND_CHALLENGE_GEN: &[u8] = b"TENDERMINT_SECRET_CONNECTION_KEY_AND_CHALLENGE_GEN";

/// Errors from the SecretConnection handshake or framing.
#[derive(Debug)]
pub enum SecretConnectionError {
    /// I/O error on the underlying stream.
    Io(io::Error),
    /// The remote ephemeral key is a low-order point.
    SmallOrderRemotePubKey,
    /// A handshake message did not decode.
    Decode(String),
    /// The remote key is not ed25519.
    UnsupportedKey(String),
    /// The remote signature over the challenge did not verify.
    ChallengeVerificationFailed,
}

impl fmt::Display for SecretConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::SmallOrderRemotePubKey => {
                f.write_str("detected low order point from remote peer")
            }
            Self::Decode(e) => write!(f, "handshake decode: {e}"),
            Self::UnsupportedKey(e) => write!(f, "expected ed25519 pubkey, got {e}"),
            Self::ChallengeVerificationFailed => f.write_str("challenge verification failed"),
        }
    }
}

impl std::error::Error for SecretConnectionError {}

impl From<io::Error> for SecretConnectionError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// `google.protobuf.BytesValue`.
#[derive(Clone, PartialEq, prost::Message)]
struct BytesValue {
    #[prost(bytes = "vec", tag = "1")]
    value: Vec<u8>,
}

/// Append a protobuf uvarint.
pub fn put_uvarint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        // Truncation intended: low 7 bits plus continuation bit.
        #[allow(clippy::cast_possible_truncation)]
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    #[allow(clippy::cast_possible_truncation)]
    buf.push(v as u8);
}

/// Length-delimited protobuf encoding (`protoio.NewDelimitedWriter`).
#[must_use]
pub fn encode_delimited<M: prost::Message>(m: &M) -> Vec<u8> {
    let body = m.encode_to_vec();
    let mut out = Vec::with_capacity(body.len() + 5);
    put_uvarint(&mut out, body.len() as u64);
    out.extend_from_slice(&body);
    out
}

fn uvarint_overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "binary: varint overflows a 64-bit integer",
    )
}

/// Read a uvarint byte by byte (Go's `binary.ReadUvarint`); returns the value
/// and the number of bytes consumed.
pub async fn read_uvarint<R: ByteRead + ?Sized>(r: &mut R) -> io::Result<(u64, usize)> {
    let mut x: u64 = 0;
    let mut s: u32 = 0;
    for i in 0..10 {
        let mut b = [0u8; 1];
        r.read_exact_bytes(&mut b).await?;
        let b = b[0];
        if b < 0x80 {
            if i == 9 && b > 1 {
                return Err(uvarint_overflow());
            }
            return Ok((x | (u64::from(b) << s), i + 1));
        }
        x |= u64::from(b & 0x7f) << s;
        s += 7;
    }
    Err(uvarint_overflow())
}

/// Read one length-delimited message body of at most `max` bytes
/// (`protoio.NewDelimitedReader(r, max).ReadMsg`). Returns the body and the
/// total number of bytes read.
pub async fn read_delimited<R: ByteRead + ?Sized>(
    r: &mut R,
    max: usize,
) -> io::Result<(Vec<u8>, usize)> {
    let (len, n) = read_uvarint(r).await?;
    let len = usize::try_from(len)
        .ok()
        .filter(|l| i64::try_from(*l).is_ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid out-of-range message length {len}"),
            )
        })?;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message exceeds max size ({len} > {max})"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact_bytes(&mut body).await?;
    Ok((body, n + len))
}

/// A byte source with `read_exact`.
pub trait ByteRead: Send {
    /// Fill `buf` completely.
    fn read_exact_bytes(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<()>> + Send;
}

/// A byte sink with `write_all` and `flush`.
pub trait ByteWrite: Send {
    /// Write all of `buf`.
    fn write_all_bytes(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<()>> + Send;
    /// Flush buffered data.
    fn flush_bytes(&mut self) -> impl Future<Output = io::Result<()>> + Send;
    /// Close the write side.
    fn shutdown_bytes(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}

/// Wraps a plain tokio stream half as a [`ByteRead`] / [`ByteWrite`]
/// (unencrypted; used for tests and Go MConnection vectors).
#[derive(Debug)]
pub struct Plain<T>(pub T);

impl<T: AsyncRead + Unpin + Send> ByteRead for Plain<T> {
    async fn read_exact_bytes(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_exact(buf).await.map(|_| ())
    }
}

impl<T: AsyncWrite + Unpin + Send> ByteWrite for Plain<T> {
    async fn write_all_bytes(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf).await
    }
    async fn flush_bytes(&mut self) -> io::Result<()> {
        self.0.flush().await
    }
    async fn shutdown_bytes(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

/// A 96-bit nonce whose last 8 bytes are a little-endian counter.
#[derive(Debug, Default)]
struct NonceCounter(u64);

impl NonceCounter {
    fn next(&mut self) -> io::Result<Nonce> {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.0.to_le_bytes());
        // Go panics on overflow to terminate the session; we error out.
        self.0 = self
            .0
            .checked_add(1)
            .ok_or_else(|| io::Error::other("can't increase nonce without overflow"))?;
        Ok(*Nonce::from_slice(&n))
    }
}

/// The receiving half of a SecretConnection.
pub struct SecretReader<R> {
    inner: R,
    cipher: ChaCha20Poly1305,
    nonce: NonceCounter,
    buf: Vec<u8>,
    pos: usize,
}

impl<R> fmt::Debug for SecretReader<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretReader")
            .field("nonce", &self.nonce.0)
            .finish_non_exhaustive()
    }
}

impl<R: AsyncRead + Unpin + Send> SecretReader<R> {
    /// Read and decrypt one frame into the internal buffer.
    async fn fill(&mut self) -> io::Result<()> {
        let mut sealed = [0u8; SEALED_FRAME_SIZE];
        self.inner.read_exact(&mut sealed).await?;
        let (frame, tag) = sealed.split_at_mut(TOTAL_FRAME_SIZE);
        let nonce = self.nonce.next()?;
        self.cipher
            .decrypt_in_place_detached(&nonce, &[], frame, Tag::from_slice(tag))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "failed to decrypt SecretConnection",
                )
            })?;
        let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        if len > DATA_MAX_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunkLength is greater than dataMaxSize",
            ));
        }
        self.buf.clear();
        self.buf
            .extend_from_slice(&frame[DATA_LEN_SIZE..DATA_LEN_SIZE + len]);
        self.pos = 0;
        Ok(())
    }

    /// Read up to `out.len()` decrypted bytes (one frame at most), like Go's
    /// `SecretConnection.Read`. Returns 0 only for an empty frame.
    pub async fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buf.len() {
            self.fill().await?;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl<R: AsyncRead + Unpin + Send> ByteRead for SecretReader<R> {
    async fn read_exact_bytes(&mut self, out: &mut [u8]) -> io::Result<()> {
        let mut filled = 0;
        while filled < out.len() {
            filled += self.read(&mut out[filled..]).await?;
        }
        Ok(())
    }
}

/// The sending half of a SecretConnection.
pub struct SecretWriter<W> {
    inner: W,
    cipher: ChaCha20Poly1305,
    nonce: NonceCounter,
    /// Plaintext frame buffer, reused without zeroing: like Go's
    /// `sendFrame`, the padding after a short chunk keeps stale bytes from
    /// earlier frames (receivers ignore it; kept for byte-exact output).
    frame: [u8; TOTAL_FRAME_SIZE],
}

impl<W> fmt::Debug for SecretWriter<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretWriter")
            .field("nonce", &self.nonce.0)
            .finish_non_exhaustive()
    }
}

impl<W: AsyncWrite + Unpin + Send> SecretWriter<W> {
    /// Seal `data` into frames (Go's `SecretConnection.Write`). Empty data
    /// writes nothing.
    pub fn seal(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(data.len().div_ceil(DATA_MAX_SIZE) * SEALED_FRAME_SIZE);
        for chunk in data.chunks(DATA_MAX_SIZE) {
            let mut frame = self.frame;
            // chunk.len() <= 1024.
            #[allow(clippy::cast_possible_truncation)]
            frame[..DATA_LEN_SIZE].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            frame[DATA_LEN_SIZE..DATA_LEN_SIZE + chunk.len()].copy_from_slice(chunk);
            self.frame = frame;
            let nonce = self.nonce.next()?;
            let tag = self
                .cipher
                .encrypt_in_place_detached(&nonce, &[], &mut frame)
                .map_err(|_| io::Error::other("encrypt"))?;
            out.extend_from_slice(&frame);
            out.extend_from_slice(&tag);
        }
        Ok(out)
    }
}

impl<W: AsyncWrite + Unpin + Send> ByteWrite for SecretWriter<W> {
    async fn write_all_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        let sealed = self.seal(data)?;
        self.inner.write_all(&sealed).await
    }
    async fn flush_bytes(&mut self) -> io::Result<()> {
        self.inner.flush().await
    }
    async fn shutdown_bytes(&mut self) -> io::Result<()> {
        self.inner.shutdown().await
    }
}

/// An authenticated, encrypted connection.
pub struct SecretConnection<S> {
    /// Receiving half.
    pub reader: SecretReader<ReadHalf<S>>,
    /// Sending half.
    pub writer: SecretWriter<WriteHalf<S>>,
    remote_pubkey: [u8; 32],
}

impl<S> fmt::Debug for SecretConnection<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SecretConnection(remote={})",
            NodeId::from_pubkey(&self.remote_pubkey)
        )
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> SecretConnection<S> {
    /// Perform the handshake with a fresh random ephemeral key
    /// (`MakeSecretConnection`).
    pub async fn handshake(stream: S, node_key: &NodeKey) -> Result<Self, SecretConnectionError> {
        let eph: [u8; 32] = rand::random();
        Self::handshake_with_ephemeral(stream, node_key, eph).await
    }

    /// Perform the handshake with a given X25519 ephemeral private key (test
    /// hook for deterministic vectors; production code uses
    /// [`Self::handshake`]).
    pub async fn handshake_with_ephemeral(
        stream: S,
        node_key: &NodeKey,
        eph_secret: [u8; 32],
    ) -> Result<Self, SecretConnectionError> {
        let (mut rd, mut wr) = tokio::io::split(stream);

        let secret = StaticSecret::from(eph_secret);
        let loc_eph_pub = XPublicKey::from(&secret).to_bytes();

        // Exchange ephemeral public keys concurrently.
        let out = encode_delimited(&BytesValue {
            value: loc_eph_pub.to_vec(),
        });
        let (w, r) = tokio::join!(wr.write_all(&out), async {
            let mut src = Plain(&mut rd);
            read_delimited(&mut src, HANDSHAKE_MAX_MSG).await
        });
        w?;
        let (body, _) = r?;
        let bv = BytesValue::decode(body.as_slice())
            .map_err(|e| SecretConnectionError::Decode(e.to_string()))?;
        let mut rem_eph_pub = [0u8; 32];
        let n = bv.value.len().min(32);
        rem_eph_pub[..n].copy_from_slice(&bv.value[..n]);

        let (lo, hi) = if loc_eph_pub < rem_eph_pub {
            (loc_eph_pub, rem_eph_pub)
        } else {
            (rem_eph_pub, loc_eph_pub)
        };
        let mut transcript = Transcript::new(b"TENDERMINT_SECRET_CONNECTION_TRANSCRIPT_HASH");
        transcript.append_message(b"EPHEMERAL_LOWER_PUBLIC_KEY", &lo);
        transcript.append_message(b"EPHEMERAL_UPPER_PUBLIC_KEY", &hi);
        let loc_is_least = loc_eph_pub == lo;

        let shared = secret.diffie_hellman(&XPublicKey::from(rem_eph_pub));
        if !shared.was_contributory() {
            return Err(SecretConnectionError::SmallOrderRemotePubKey);
        }
        let dh = shared.to_bytes();
        transcript.append_message(b"DH_SECRET", &dh);

        let (recv_key, send_key) = derive_secrets(&dh, loc_is_least);
        let mut challenge = [0u8; 32];
        transcript.challenge_bytes(b"SECRET_CONNECTION_MAC", &mut challenge);

        let mut reader = SecretReader {
            inner: rd,
            cipher: ChaCha20Poly1305::new(Key::from_slice(&recv_key)),
            nonce: NonceCounter::default(),
            buf: Vec::new(),
            pos: 0,
        };
        let mut writer = SecretWriter {
            inner: wr,
            cipher: ChaCha20Poly1305::new(Key::from_slice(&send_key)),
            nonce: NonceCounter::default(),
            frame: [0u8; TOTAL_FRAME_SIZE],
        };

        let sig = node_key.sign(&challenge);
        let auth = pb::AuthSigMessage {
            pub_key: Some(pbcrypto::PublicKey {
                sum: Some(pbcrypto::public_key::Sum::Ed25519(
                    node_key.public_key().to_vec(),
                )),
            }),
            sig: sig.to_vec(),
        };
        let out = encode_delimited(&auth);
        let (w, r) = tokio::join!(
            writer.write_all_bytes(&out),
            read_delimited(&mut reader, HANDSHAKE_MAX_MSG)
        );
        w?;
        let (body, _) = r?;
        let rem = pb::AuthSigMessage::decode(body.as_slice())
            .map_err(|e| SecretConnectionError::Decode(e.to_string()))?;
        let remote_pubkey: [u8; 32] = match rem.pub_key.and_then(|k| k.sum) {
            Some(pbcrypto::public_key::Sum::Ed25519(k)) => {
                k.as_slice().try_into().map_err(|_| {
                    SecretConnectionError::Decode(format!("invalid ed25519 key size {}", k.len()))
                })?
            }
            Some(pbcrypto::public_key::Sum::Secp256k1(_)) => {
                return Err(SecretConnectionError::UnsupportedKey(
                    "secp256k1.PubKey".into(),
                ));
            }
            None => return Err(SecretConnectionError::Decode("nil PublicKey".into())),
        };
        let sig_ok = <[u8; 64]>::try_from(rem.sig.as_slice())
            .is_ok_and(|s| crypto::ed25519::verify(&remote_pubkey, &challenge, &s));
        if !sig_ok {
            return Err(SecretConnectionError::ChallengeVerificationFailed);
        }
        Ok(Self {
            reader,
            writer,
            remote_pubkey,
        })
    }
}

impl<S> SecretConnection<S> {
    /// The authenticated remote ed25519 public key.
    #[must_use]
    pub fn remote_pubkey(&self) -> [u8; 32] {
        self.remote_pubkey
    }

    /// The remote node ID.
    #[must_use]
    pub fn remote_id(&self) -> NodeId {
        NodeId::from_pubkey(&self.remote_pubkey)
    }

    /// Split into the receiving and sending halves.
    #[must_use]
    pub fn into_split(self) -> (SecretReader<ReadHalf<S>>, SecretWriter<WriteHalf<S>>) {
        (self.reader, self.writer)
    }
}

/// HKDF-SHA256 over the DH secret: 32-byte receive key, 32-byte send key
/// (order swapped when the local ephemeral key is the larger one).
fn derive_secrets(dh: &[u8; 32], loc_is_least: bool) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(None, dh);
    let mut okm = [0u8; 96];
    // 96 <= 255 * 32, expand cannot fail.
    let _ = hk.expand(KEY_AND_CHALLENGE_GEN, &mut okm);
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    a.copy_from_slice(&okm[..32]);
    b.copy_from_slice(&okm[32..64]);
    if loc_is_least { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handshake_roundtrip_and_data() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let ka = NodeKey::from_seed(&[1; 32]);
        let kb = NodeKey::from_seed(&[2; 32]);
        let (ra, rb) = tokio::join!(
            SecretConnection::handshake(a, &ka),
            SecretConnection::handshake(b, &kb)
        );
        let (ra, rb) = (ra.unwrap(), rb.unwrap());
        assert_eq!(ra.remote_id(), kb.id());
        assert_eq!(rb.remote_id(), ka.id());
        let (_, mut wa) = ra.into_split();
        let (mut rdb, _) = rb.into_split();
        let msg: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        wa.write_all_bytes(&msg).await.unwrap();
        let mut got = vec![0u8; msg.len()];
        rdb.read_exact_bytes(&mut got).await.unwrap();
        assert_eq!(got, msg);
    }

    /// Low-order points (from the Go test suite's blacklist) are rejected.
    #[tokio::test]
    async fn rejects_low_order_points() {
        let low_order: [[u8; 32]; 3] = [
            [0; 32],
            {
                let mut p = [0; 32];
                p[0] = 1;
                p
            },
            hex::decode("e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800")
                .unwrap()
                .try_into()
                .unwrap(),
        ];
        for point in low_order {
            let (a, mut b) = tokio::io::duplex(1 << 16);
            let k = NodeKey::from_seed(&[3; 32]);
            let peer = async move {
                b.write_all(&encode_delimited(&BytesValue {
                    value: point.to_vec(),
                }))
                .await
                .unwrap();
                b
            };
            let (res, _b) = tokio::join!(SecretConnection::handshake(a, &k), peer);
            assert!(matches!(
                res,
                Err(SecretConnectionError::SmallOrderRemotePubKey)
            ));
        }
    }
}
