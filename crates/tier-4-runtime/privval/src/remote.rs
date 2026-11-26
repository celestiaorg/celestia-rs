//! Node side of the CometBFT socket signer protocol.

use std::io;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ctypes::canonical::{
    proposal_sign_bytes, raw_bytes_message_sign_bytes, vote_extension_sign_bytes, vote_sign_bytes,
};
use ctypes::proto::privval as wire;
use ctypes::proto::types;
use ctypes::{PubKey, Time};
use p2p::NodeKey;
use p2p::secret_connection::{ByteRead, ByteWrite, SecretConnection, read_delimited};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc as async_mpsc, oneshot};
use wire::message::Sum;

use crate::{Error, PrivValidator, Result};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 10 * 1024; // celestia-core signer endpoint limit

/// Raw-bytes requests ([`RemoteRawBytesSigner`]) queued or in flight at
/// once; callers beyond it fail at once with a "busy" error.
pub const RAW_QUEUE_CAP: usize = 2;
/// A raw-bytes request not sent within this long after it was queued is
/// dropped unsent (its caller has most likely given up).
pub const RAW_DEADLINE: Duration = Duration::from_secs(1);

struct Request {
    message: Sum,
    reply: mpsc::SyncSender<Result<Sum>>,
    /// Raw-bytes requests only: dropped unsent after this instant.
    deadline: Option<std::time::Instant>,
    /// Raw-bytes requests only: their queue slot and the shared raw path.
    slot: Option<RawSlot>,
}

/// State shared by the raw-bytes handles and the connection worker.
#[derive(Default)]
struct RawPath {
    /// Set once a raw request cost the connection or the signer refused
    /// `SignRawBytes`: raw signing stays off until the node restarts.
    disabled: std::sync::OnceLock<String>,
    /// Raw requests queued or in flight.
    pending: std::sync::atomic::AtomicUsize,
}

impl RawPath {
    fn disable(&self, cause: &str) {
        let reason = format!(
            "{cause}. The remote signer must support SignRawBytes (Horcrux v3.3.3-celestia, \
             Celestia tmkms v0.14.0-celestia or later); fix the signer and restart the node"
        );
        if self.disabled.set(reason.clone()).is_ok() {
            tracing::error!(
                reason,
                "raw-bytes signing through the remote signer is disabled to protect consensus"
            );
        }
    }

    fn disabled_error(&self) -> Option<Error> {
        self.disabled
            .get()
            .map(|r| Error::new(format!("remote signer raw-bytes signing is disabled: {r}")))
    }
}

/// A raw request's place in the queue; released when the request is done.
struct RawSlot(std::sync::Arc<RawPath>);

impl RawSlot {
    fn acquire(path: &std::sync::Arc<RawPath>) -> Result<Self> {
        use std::sync::atomic::Ordering;
        if path.pending.fetch_add(1, Ordering::SeqCst) >= RAW_QUEUE_CAP {
            path.pending.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::new(
                "remote signer is busy with other raw-bytes requests",
            ));
        }
        Ok(Self(std::sync::Arc::clone(path)))
    }
}

impl Drop for RawSlot {
    fn drop(&mut self) {
        self.0
            .pending
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A socket signer that dials the node. All signing and double-sign state
/// live in the remote process. The consensus public key is pinned for the
/// lifetime of this client, including across reconnects.
pub struct RemotePv {
    chain_id: String,
    pub_key: PubKey,
    requests: async_mpsc::UnboundedSender<Request>,
    /// Requests of [`RemoteRawBytesSigner`] handles: served after any
    /// pending consensus request.
    raw_requests: async_mpsc::UnboundedSender<Request>,
    raw: std::sync::Arc<RawPath>,
    worker: Option<(oneshot::Sender<()>, JoinHandle<()>)>,
}

/// A cloneable, thread-safe handle that signs raw bytes (`SignRawBytes`)
/// through a [`RemotePv`]'s signer connection, for in-process services
/// other than consensus (the Fibre server).
///
/// Raw-bytes requests carry no height / round / step and do not touch the
/// signer's double-sign state. They share the connection with consensus,
/// so the handle protects consensus:
///
/// - The connection worker always serves a pending vote, proposal or
///   compact-block request first; a vote that arrives while a raw
///   exchange is on the wire waits for that one exchange.
/// - At most [`RAW_QUEUE_CAP`] raw requests are queued or in flight;
///   callers beyond it fail at once ("busy").
/// - A raw request still queued [`RAW_DEADLINE`] after it was made is
///   dropped unsent ("expired").
/// - A raw request that costs the connection (the signer hangs up, times
///   out or answers garbage) or that the signer refuses as unsupported
///   disables the raw path for the rest of the process
///   ([`RemoteRawBytesSigner::disabled`]): such a signer would otherwise
///   cost a reconnect, and missed votes, on every Fibre upload.
///
/// The handle fails with `remote signer worker stopped` once the
/// [`RemotePv`] is dropped. Calls block the calling thread until the
/// signer answers (at most the 5 s request timeout while connected), so
/// async callers use `spawn_blocking`.
#[derive(Clone)]
pub struct RemoteRawBytesSigner {
    chain_id: String,
    pub_key: PubKey,
    requests: async_mpsc::UnboundedSender<Request>,
    raw: std::sync::Arc<RawPath>,
}

impl std::fmt::Debug for RemoteRawBytesSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteRawBytesSigner")
            .field("chain_id", &self.chain_id)
            .finish_non_exhaustive()
    }
}

impl RemoteRawBytesSigner {
    /// The consensus public key pinned at connection time.
    #[must_use]
    pub fn pub_key(&self) -> PubKey {
        self.pub_key.clone()
    }

    /// `SignRawBytes`: signs `RawBytesMessageSignBytes(chain_id, unique_id,
    /// raw_bytes)` and checks the signature against the pinned key.
    pub fn sign_raw_bytes(
        &self,
        chain_id: &str,
        unique_id: &str,
        raw_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        if let Some(e) = self.raw.disabled_error() {
            return Err(e);
        }
        let slot = RawSlot::acquire(&self.raw)?;
        let deadline = std::time::Instant::now() + RAW_DEADLINE;
        let res = sign_raw(
            &self.chain_id,
            &self.pub_key,
            &self.requests,
            chain_id,
            unique_id,
            raw_bytes,
            Some((deadline, slot)),
        );
        if let Err(e) = &res {
            let refused = e.0.starts_with("remote signer error")
                && ["unsupported", "not supported", "unknown", "not implemented"]
                    .iter()
                    .any(|w| e.0.to_ascii_lowercase().contains(w));
            if refused || e.0.starts_with("expected raw bytes signature") {
                self.raw
                    .disable(&format!("the remote signer refused SignRawBytes ({e})"));
            }
        }
        res
    }

    /// Why raw-bytes signing is disabled, once a request cost the signer
    /// connection or the signer refused `SignRawBytes`; `None` while it
    /// works. It stays disabled until the node restarts.
    #[must_use]
    pub fn disabled(&self) -> Option<String> {
        self.raw.disabled.get().cloned()
    }
}

/// Sends `message` to the connection worker and waits for the answer.
fn send_request(
    expected_chain: &str,
    requests: &async_mpsc::UnboundedSender<Request>,
    chain_id: &str,
    message: Sum,
    raw: Option<(std::time::Instant, RawSlot)>,
) -> Result<Sum> {
    if chain_id != expected_chain {
        return Err(Error::new(format!(
            "remote signer chain ID mismatch: expected {expected_chain:?}, got {chain_id:?}"
        )));
    }
    let (reply, result) = mpsc::sync_channel(1);
    let (deadline, slot) = raw.map_or((None, None), |(d, s)| (Some(d), Some(s)));
    requests
        .send(Request {
            message,
            reply,
            deadline,
            slot,
        })
        .map_err(|_| worker_stopped())?;
    result.recv().map_err(|_| worker_stopped())?
}

fn sign_raw(
    expected_chain: &str,
    pub_key: &PubKey,
    requests: &async_mpsc::UnboundedSender<Request>,
    chain_id: &str,
    unique_id: &str,
    raw_bytes: &[u8],
    raw: Option<(std::time::Instant, RawSlot)>,
) -> Result<Vec<u8>> {
    let bytes = raw_bytes_message_sign_bytes(chain_id, unique_id, raw_bytes).map_err(type_error)?;
    let request = Sum::SignRawBytesRequest(wire::SignRawBytesRequest {
        chain_id: chain_id.to_owned(),
        unique_id: unique_id.to_owned(),
        raw_bytes: raw_bytes.to_vec(),
    });
    let Sum::SignedRawBytesResponse(response) =
        send_request(expected_chain, requests, chain_id, request, raw)?
    else {
        return Err(unexpected("raw bytes signature"));
    };
    check_remote_error(response.error)?;
    if !pub_key.verify_signature(&bytes, &response.signature) {
        return Err(Error::new("invalid raw bytes signature from remote signer"));
    }
    Ok(response.signature)
}

impl RemotePv {
    /// Listen for a signer at `tcp://host:port`, `host:port`, or `unix:///path`
    /// (on Unix). TCP uses SecretConnection with the node's transport key.
    /// Waits for the signer to supply its consensus public key.
    pub fn listen(address: &str, chain_id: &str, node_key: NodeKey) -> Result<Self> {
        let (requests, incoming) = async_mpsc::unbounded_channel();
        let (raw_requests, raw_incoming) = async_mpsc::unbounded_channel();
        let (ready, started) = mpsc::sync_channel(1);
        let (stop, stopped) = oneshot::channel();
        let worker_chain = chain_id.to_owned();
        let address = address.to_owned();
        let thread = thread::Builder::new()
            .name("privval".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match result {
                    Ok(runtime) => runtime.block_on(async {
                        tokio::select! {
                            () = run(&address, &worker_chain, &node_key, Queues { consensus: incoming, raw: raw_incoming }, ready) => {},
                            _ = stopped => {},
                        }
                    }),
                    Err(e) => {
                        let _ = ready.send(Err(Error::new(format!("start signer runtime: {e}"))));
                    }
                }
            })
            .map_err(|e| Error::new(format!("start signer thread: {e}")))?;
        let result = started.recv().unwrap_or_else(|_| Err(worker_stopped()));
        match result {
            Ok(pub_key) => Ok(Self {
                chain_id: chain_id.to_owned(),
                pub_key,
                requests,
                raw_requests,
                raw: std::sync::Arc::default(),
                worker: Some((stop, thread)),
            }),
            Err(e) => {
                let _ = thread.join();
                Err(e)
            }
        }
    }

    /// A handle that signs raw bytes through this signer's connection
    /// from other threads (see [`RemoteRawBytesSigner`]).
    #[must_use]
    pub fn raw_bytes_signer(&self) -> RemoteRawBytesSigner {
        RemoteRawBytesSigner {
            chain_id: self.chain_id.clone(),
            pub_key: self.pub_key.clone(),
            requests: self.raw_requests.clone(),
            raw: std::sync::Arc::clone(&self.raw),
        }
    }

    fn request(&self, chain_id: &str, message: Sum) -> Result<Sum> {
        send_request(&self.chain_id, &self.requests, chain_id, message, None)
    }

    fn verify(&self, bytes: &[u8], signature: &[u8], label: &str) -> Result<()> {
        if !self.pub_key.verify_signature(bytes, signature) {
            return Err(Error::new(format!(
                "invalid {label} signature from remote signer"
            )));
        }
        Ok(())
    }
}

impl Drop for RemotePv {
    fn drop(&mut self) {
        if let Some((stop, thread)) = self.worker.take() {
            let _ = stop.send(());
            let _ = thread.join();
        }
    }
}

impl PrivValidator for RemotePv {
    fn pub_key(&self) -> Result<PubKey> {
        Ok(self.pub_key.clone())
    }

    fn sign_vote(&mut self, chain_id: &str, vote: &mut types::Vote) -> Result<()> {
        let request = Sum::SignVoteRequest(wire::SignVoteRequest {
            chain_id: chain_id.to_owned(),
            vote: Some(vote.clone()),
        });
        let Sum::SignedVoteResponse(response) = self.request(chain_id, request)? else {
            return Err(unexpected("signed vote"));
        };
        check_remote_error(response.error)?;
        let signed = response.vote.ok_or_else(|| unexpected("vote"))?;
        // Horcrux returns just the signing fields. Check the signature over
        // the original vote with the returned timestamp before using it.
        let mut candidate = vote.clone();
        candidate.timestamp = Some(
            signed
                .timestamp
                .ok_or_else(|| unexpected("vote timestamp"))?,
        );
        Time::from_proto(candidate.timestamp.as_ref()).map_err(type_error)?;
        self.verify(
            &vote_sign_bytes(chain_id, &candidate).map_err(type_error)?,
            &signed.signature,
            "vote",
        )?;
        if !signed.extension_signature.is_empty() {
            self.verify(
                &vote_extension_sign_bytes(chain_id, vote),
                &signed.extension_signature,
                "vote extension",
            )?;
        }
        candidate.signature = signed.signature;
        candidate.extension_signature = signed.extension_signature;
        *vote = candidate;
        Ok(())
    }

    fn sign_proposal(&mut self, chain_id: &str, proposal: &mut types::Proposal) -> Result<()> {
        let request = Sum::SignProposalRequest(wire::SignProposalRequest {
            chain_id: chain_id.to_owned(),
            proposal: Some(proposal.clone()),
        });
        let Sum::SignedProposalResponse(response) = self.request(chain_id, request)? else {
            return Err(unexpected("signed proposal"));
        };
        check_remote_error(response.error)?;
        let signed = response.proposal.ok_or_else(|| unexpected("proposal"))?;
        let mut candidate = proposal.clone();
        candidate.timestamp = Some(
            signed
                .timestamp
                .ok_or_else(|| unexpected("proposal timestamp"))?,
        );
        Time::from_proto(candidate.timestamp.as_ref()).map_err(type_error)?;
        self.verify(
            &proposal_sign_bytes(chain_id, &candidate).map_err(type_error)?,
            &signed.signature,
            "proposal",
        )?;
        candidate.signature = signed.signature;
        *proposal = candidate;
        Ok(())
    }

    fn sign_raw_bytes(&self, chain_id: &str, unique_id: &str, raw_bytes: &[u8]) -> Result<Vec<u8>> {
        sign_raw(
            &self.chain_id,
            &self.pub_key,
            &self.requests,
            chain_id,
            unique_id,
            raw_bytes,
            None,
        )
    }
}

fn type_error(error: ctypes::Error) -> Error {
    Error::new(error.to_string())
}

fn worker_stopped() -> Error {
    Error::new("remote signer worker stopped")
}

fn unexpected(expected: &str) -> Error {
    Error::new(format!("expected {expected} from remote signer"))
}

fn check_remote_error(error: Option<wire::RemoteSignerError>) -> Result<()> {
    match error {
        Some(e) => Err(Error::new(format!(
            "remote signer error {}: {}",
            e.code, e.description
        ))),
        None => Ok(()),
    }
}

enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener, std::path::PathBuf),
}

impl Listener {
    async fn bind(address: &str) -> io::Result<Self> {
        let (protocol, endpoint) = address.split_once("://").unwrap_or(("tcp", address));
        match protocol {
            "tcp" if endpoint.starts_with(':') => Ok(Self::Tcp(
                TcpListener::bind(format!("0.0.0.0{endpoint}")).await?,
            )),
            "tcp" => Ok(Self::Tcp(TcpListener::bind(endpoint).await?)),
            #[cfg(unix)]
            "unix" => Ok(Self::Unix(
                tokio::net::UnixListener::bind(endpoint)?,
                endpoint.into(),
            )),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected tcp:// or unix:// signer address",
            )),
        }
    }

    async fn connect(&self, node_key: &NodeKey, chain_id: &str) -> Result<(Session, PubKey)> {
        let mut session = self.accept(node_key).await?;
        let Sum::PubKeyResponse(response) = session
            .exchange(Sum::PubKeyRequest(wire::PubKeyRequest {
                chain_id: chain_id.to_owned(),
            }))
            .await?
        else {
            return Err(unexpected("public key response"));
        };
        check_remote_error(response.error)?;
        let key = response.pub_key.ok_or_else(|| unexpected("public key"))?;
        Ok((session, PubKey::from_proto(&key).map_err(type_error)?))
    }

    async fn accept(&self, node_key: &NodeKey) -> Result<Session> {
        match self {
            Self::Tcp(listener) => {
                let (stream, _) = listener.accept().await.map_err(transport_error)?;
                stream.set_nodelay(true).map_err(transport_error)?;
                let conn =
                    tokio::time::timeout(IO_TIMEOUT, SecretConnection::handshake(stream, node_key))
                        .await
                        .map_err(|_| {
                            Error::new("remote signer SecretConnection handshake timed out")
                        })?
                        .map_err(|e| {
                            Error::new(format!("remote signer SecretConnection handshake: {e}"))
                        })?;
                Ok(Session::Tcp(Box::new(conn)))
            }
            #[cfg(unix)]
            Self::Unix(listener, _) => {
                let (stream, _) = listener.accept().await.map_err(transport_error)?;
                let (reader, writer) = stream.into_split();
                Ok(Session::Unix(reader, writer))
            }
        }
    }
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        if let Self::Unix(_, path) = self {
            let _ = std::fs::remove_file(path);
        }
    }
}

enum Session {
    Tcp(Box<SecretConnection<TcpStream>>),
    #[cfg(unix)]
    Unix(
        tokio::net::unix::OwnedReadHalf,
        tokio::net::unix::OwnedWriteHalf,
    ),
}

impl Session {
    async fn exchange(&mut self, request: Sum) -> Result<Sum> {
        let data = wire::Message { sum: Some(request) }.encode_length_delimited_to_vec();
        let result = tokio::time::timeout(IO_TIMEOUT, async {
            match self {
                Self::Tcp(conn) => exchange(&mut conn.reader, &mut conn.writer, &data).await,
                #[cfg(unix)]
                Self::Unix(reader, writer) => {
                    use p2p::secret_connection::Plain;
                    exchange(&mut Plain(reader), &mut Plain(writer), &data).await
                }
            }
        })
        .await
        .map_err(|_| Error::new("remote signer request timed out"))?;
        result.map_err(transport_error)
    }
}

async fn exchange(
    reader: &mut impl ByteRead,
    writer: &mut impl ByteWrite,
    data: &[u8],
) -> io::Result<Sum> {
    writer.write_all_bytes(data).await?;
    writer.flush_bytes().await?;
    let (body, _) = read_delimited(reader, MAX_RESPONSE_BYTES).await?;
    wire::Message::decode(body.as_slice())?
        .sum
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty signer response"))
}

fn transport_error(error: io::Error) -> Error {
    Error::new(format!("remote signer connection: {error}"))
}

/// The connection worker's request queues.
struct Queues {
    /// Consensus requests ([`RemotePv`]); closed when it is dropped.
    consensus: async_mpsc::UnboundedReceiver<Request>,
    /// [`RemoteRawBytesSigner`] requests, served after consensus ones.
    raw: async_mpsc::UnboundedReceiver<Request>,
}

impl Queues {
    /// The next request, consensus first; `None` once [`RemotePv`] is
    /// gone.
    async fn next(&mut self) -> Option<Request> {
        tokio::select! {
            biased;
            request = self.consensus.recv() => request,
            Some(request) = self.raw.recv() => Some(request),
        }
    }
}

async fn run(
    address: &str,
    chain_id: &str,
    node_key: &NodeKey,
    mut requests: Queues,
    ready: mpsc::SyncSender<Result<PubKey>>,
) {
    let start = tokio::time::timeout(STARTUP_TIMEOUT, async {
        let listener = Listener::bind(address).await.map_err(transport_error)?;
        tracing::info!(address, "waiting for remote validator signer");
        let (session, pub_key) = listener.connect(node_key, chain_id).await?;
        Ok((listener, session, pub_key))
    })
    .await
    .unwrap_or_else(|_| Err(Error::new("timed out waiting for remote validator signer")));
    let (listener, mut session, pub_key) = match start {
        Ok(start) => start,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if ready.send(Ok(pub_key.clone())).is_err() {
        return;
    }
    loop {
        tracing::info!(address, "remote validator signer connected");
        match serve(&mut session, &mut requests).await {
            Ok(()) => return,
            Err(e) => tracing::warn!(address, error = %e, "remote validator signer disconnected"),
        }
        drop(session);
        session = tokio::select! {
            next = reconnect(&listener, node_key, chain_id, &pub_key, address) => next,
            () = reject_while_disconnected(&mut requests) => return,
        };
    }
}

async fn reconnect(
    listener: &Listener,
    node_key: &NodeKey,
    chain_id: &str,
    pub_key: &PubKey,
    address: &str,
) -> Session {
    loop {
        match listener.connect(node_key, chain_id).await {
            Ok((session, key)) if key == *pub_key => return session,
            Ok(_) => tracing::warn!(
                address,
                "rejected remote signer with a different consensus public key"
            ),
            Err(e) => tracing::warn!(address, error = %e, "remote signer reconnect failed"),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn reject_while_disconnected(requests: &mut Queues) {
    while let Some(request) = requests.next().await {
        let _ = request
            .reply
            .send(Err(Error::new("remote signer is disconnected")));
    }
}

async fn serve(session: &mut Session, requests: &mut Queues) -> Result<()> {
    loop {
        tokio::select! {
            request = requests.next() => {
                let Some(request) = request else { return Ok(()) };
                if request.deadline.is_some_and(|d| std::time::Instant::now() > d) {
                    let _ = request.reply.send(Err(Error::new(
                        "remote signer raw-bytes request expired before it was sent",
                    )));
                    continue;
                }
                if let Some(e) = request.slot.as_ref().and_then(|s| s.0.disabled_error()) {
                    let _ = request.reply.send(Err(e));
                    continue;
                }
                let result = session.exchange(request.message).await;
                let connection_error = result.as_ref().err().cloned();
                if let (Some(error), Some(slot)) = (&connection_error, &request.slot) {
                    slot.0.disable(&format!(
                        "a raw-bytes (SignRawBytes) request cost the signer connection: {error}"
                    ));
                }
                // A timed-out request may already have been signed. Let consensus
                // decide what to request next, with the signer's persisted guard.
                let _ = request.reply.send(result);
                if let Some(error) = connection_error {
                    return Err(error);
                }
            }
            () = tokio::time::sleep(PING_INTERVAL) => {
                if !matches!(session.exchange(Sum::PingRequest(wire::PingRequest {})).await?, Sum::PingResponse(_)) {
                    return Err(unexpected("ping response"));
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use p2p::secret_connection::Plain;
    use tokio::io::AsyncWriteExt as _;

    use super::*;
    use crate::{FilePv, FilePvKey};

    const CHAIN: &str = "test-chain";

    /// How the fake signer treats requests.
    #[derive(Clone, Copy)]
    enum Mode {
        /// Signs everything.
        Normal,
        /// Closes the connection on a `SignRawBytesRequest` (a signer
        /// without raw-bytes support), then dials again.
        HangUpOnRaw,
        /// Answers votes after this delay.
        SlowVotes(Duration),
    }

    #[derive(Default)]
    struct Counters {
        connections: AtomicUsize,
        raw: AtomicUsize,
    }

    /// A socket signer over a Unix socket that signs with a [`FilePv`],
    /// dials again after a disconnect (as tmkms and Horcrux do) and counts
    /// its connections and the raw-bytes requests it received.
    fn spawn_signer(socket: PathBuf, mode: Mode) -> (FilePvKey, Arc<Counters>) {
        let key =
            FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(&[7; 32])).unwrap();
        let mut signer = FilePv::new(key.clone(), None, None);
        let counters = Arc::new(Counters::default());
        let c = Arc::clone(&counters);
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                loop {
                    let stream = loop {
                        match tokio::net::UnixStream::connect(&socket).await {
                            Ok(s) => break s,
                            Err(_)
                                if c.connections.load(Ordering::SeqCst) > 0 && !socket.exists() =>
                            {
                                return;
                            }
                            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                        }
                    };
                    c.connections.fetch_add(1, Ordering::SeqCst);
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = Plain(reader);
                    loop {
                        let Ok((bytes, _)) = read_delimited(&mut reader, 1 << 20).await else {
                            break;
                        };
                        let request = wire::Message::decode(bytes.as_slice()).unwrap();
                        let response = match request.sum.unwrap() {
                            Sum::PubKeyRequest(_) => Sum::PubKeyResponse(wire::PubKeyResponse {
                                pub_key: Some(signer.pub_key().unwrap().to_proto()),
                                error: None,
                            }),
                            Sum::SignVoteRequest(req) => {
                                if let Mode::SlowVotes(d) = mode {
                                    tokio::time::sleep(d).await;
                                }
                                let mut vote = req.vote.unwrap();
                                signer.sign_vote(&req.chain_id, &mut vote).unwrap();
                                Sum::SignedVoteResponse(wire::SignedVoteResponse {
                                    vote: Some(vote),
                                    error: None,
                                })
                            }
                            Sum::SignRawBytesRequest(req) => {
                                c.raw.fetch_add(1, Ordering::SeqCst);
                                if let Mode::HangUpOnRaw = mode {
                                    break;
                                }
                                Sum::SignedRawBytesResponse(wire::SignedRawBytesResponse {
                                    signature: signer
                                        .sign_raw_bytes(
                                            &req.chain_id,
                                            &req.unique_id,
                                            &req.raw_bytes,
                                        )
                                        .unwrap(),
                                    error: None,
                                })
                            }
                            Sum::PingRequest(_) => Sum::PingResponse(wire::PingResponse {}),
                            other => panic!("unexpected request {other:?}"),
                        };
                        let msg = wire::Message {
                            sum: Some(response),
                        };
                        if writer
                            .write_all(&msg.encode_length_delimited_to_vec())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            });
        });
        (key, counters)
    }

    fn listen(socket: &std::path::Path) -> RemotePv {
        RemotePv::listen(
            &format!("unix://{}", socket.display()),
            CHAIN,
            NodeKey::from_seed(&[1; 32]),
        )
        .unwrap()
    }

    fn prevote(height: i64) -> types::Vote {
        types::Vote {
            r#type: ctypes::vote::PREVOTE_TYPE,
            height,
            round: 0,
            timestamp: Some(pbjson_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }),
            validator_address: vec![0; 20],
            ..Default::default()
        }
    }

    #[test]
    fn raw_bytes_handle_shares_the_signer_connection() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("pv.sock");
        let (key, counters) = spawn_signer(socket.clone(), Mode::Normal);
        let mut pv = listen(&socket);
        let handle = pv.raw_bytes_signer();
        assert_eq!(handle.pub_key(), key.pub_key());

        // Raw-bytes signing from other threads while consensus signs votes.
        // Callers beyond the small raw queue get a quick "busy" error and
        // retry here.
        let workers: Vec<_> = (0..4u8)
            .map(|i| {
                let h = handle.clone();
                thread::spawn(move || {
                    for j in 0..5u8 {
                        let msg = [i, j];
                        let sig = loop {
                            match h.sign_raw_bytes(CHAIN, "fibre", &msg) {
                                Ok(sig) => break sig,
                                Err(e) if e.0.contains("busy") => {
                                    thread::sleep(Duration::from_millis(1));
                                }
                                Err(e) => panic!("{e}"),
                            }
                        };
                        let sb = raw_bytes_message_sign_bytes(CHAIN, "fibre", &msg).unwrap();
                        assert!(h.pub_key().verify_signature(&sb, &sig));
                    }
                })
            })
            .collect();
        for height in 1..=5 {
            let mut vote = prevote(height);
            pv.sign_vote(CHAIN, &mut vote).unwrap();
            assert_eq!(vote.signature.len(), 64);
        }
        for w in workers {
            w.join().unwrap();
        }
        assert_eq!(counters.raw.load(Ordering::SeqCst), 20);
        assert_eq!(counters.connections.load(Ordering::SeqCst), 1);
        assert_eq!(handle.disabled(), None);

        assert_eq!(
            handle
                .sign_raw_bytes("other-chain", "fibre", b"x")
                .unwrap_err()
                .0,
            "remote signer chain ID mismatch: expected \"test-chain\", got \"other-chain\""
        );

        drop(pv);
        assert_eq!(
            handle.sign_raw_bytes(CHAIN, "fibre", b"x").unwrap_err().0,
            "remote signer worker stopped"
        );
    }

    /// A signer that drops the connection on `SignRawBytes` costs exactly
    /// one reconnect: the raw path is then disabled for good, and votes
    /// keep signing.
    #[test]
    fn a_raw_request_that_costs_the_connection_disables_the_raw_path() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("pv.sock");
        let (_, counters) = spawn_signer(socket.clone(), Mode::HangUpOnRaw);
        let mut pv = listen(&socket);
        let handle = pv.raw_bytes_signer();

        let err = handle.sign_raw_bytes(CHAIN, "fibre", b"cert").unwrap_err();
        assert!(err.0.starts_with("remote signer connection"), "{err}");
        let reason = handle.disabled().expect("raw path disabled");
        assert!(reason.contains("SignRawBytes"), "{reason}");

        // Consensus recovers after the one reconnect.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut height = 1;
        loop {
            let mut vote = prevote(height);
            match pv.sign_vote(CHAIN, &mut vote) {
                Ok(()) => break,
                Err(_) if std::time::Instant::now() < deadline => {
                    height += 1;
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("{e}"),
            }
        }

        // Later raw requests fail at once without reaching the signer.
        for _ in 0..5 {
            let err = handle.sign_raw_bytes(CHAIN, "fibre", b"again").unwrap_err();
            assert!(err.0.contains("disabled"), "{err}");
        }
        for h in height + 1..height + 4 {
            pv.sign_vote(CHAIN, &mut prevote(h)).unwrap();
        }
        assert_eq!(counters.raw.load(Ordering::SeqCst), 1);
        assert_eq!(counters.connections.load(Ordering::SeqCst), 2);
    }

    /// With both queues non-empty the consensus request goes first.
    #[test]
    fn consensus_requests_are_served_before_raw_requests() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (consensus_tx, consensus) = async_mpsc::unbounded_channel();
        let (raw_tx, raw) = async_mpsc::unbounded_channel();
        let mut queues = Queues { consensus, raw };
        let request = |message| Request {
            message,
            reply: mpsc::sync_channel(1).0,
            deadline: None,
            slot: None,
        };
        raw_tx
            .send(request(Sum::PingRequest(wire::PingRequest {})))
            .unwrap_or_else(|_| panic!("send"));
        consensus_tx
            .send(request(Sum::PubKeyRequest(wire::PubKeyRequest::default())))
            .unwrap_or_else(|_| panic!("send"));
        let first = rt.block_on(queues.next()).unwrap();
        assert!(matches!(first.message, Sum::PubKeyRequest(_)));
        let second = rt.block_on(queues.next()).unwrap();
        assert!(matches!(second.message, Sum::PingRequest(_)));
    }

    /// A raw request that waited past its deadline behind a slow vote is
    /// dropped without reaching the signer; callers beyond the raw queue
    /// cap fail at once.
    #[test]
    fn stale_and_excess_raw_requests_are_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("pv.sock");
        let slow = RAW_DEADLINE + Duration::from_millis(500);
        let (_, counters) = spawn_signer(socket.clone(), Mode::SlowVotes(slow));
        let mut pv = listen(&socket);
        let handle = pv.raw_bytes_signer();
        let voter = thread::spawn(move || {
            pv.sign_vote(CHAIN, &mut prevote(1)).unwrap();
            pv
        });
        thread::sleep(Duration::from_millis(100));
        let raw: Vec<_> = (0..RAW_QUEUE_CAP + 2)
            .map(|i| {
                let h = handle.clone();
                let t =
                    thread::spawn(move || h.sign_raw_bytes(CHAIN, "fibre", &[0]).unwrap_err().0);
                if i < RAW_QUEUE_CAP {
                    thread::sleep(Duration::from_millis(50));
                }
                t
            })
            .collect();
        let errors: Vec<String> = raw.into_iter().map(|t| t.join().unwrap()).collect();
        let _pv = voter.join().unwrap();
        let expired = errors.iter().filter(|e| e.contains("expired")).count();
        let busy = errors.iter().filter(|e| e.contains("busy")).count();
        assert_eq!((expired, busy), (RAW_QUEUE_CAP, 2), "{errors:?}");
        assert_eq!(counters.raw.load(Ordering::SeqCst), 0);
        assert_eq!(handle.disabled(), None);
    }
}
