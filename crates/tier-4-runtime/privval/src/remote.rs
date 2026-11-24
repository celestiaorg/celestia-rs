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

struct Request {
    message: Sum,
    reply: mpsc::SyncSender<Result<Sum>>,
}

/// A socket signer that dials the node. All signing and double-sign state
/// live in the remote process. The consensus public key is pinned for the
/// lifetime of this client, including across reconnects.
pub struct RemotePv {
    chain_id: String,
    pub_key: PubKey,
    requests: async_mpsc::UnboundedSender<Request>,
    worker: Option<(oneshot::Sender<()>, JoinHandle<()>)>,
}

impl RemotePv {
    /// Listen for a signer at `tcp://host:port`, `host:port`, or `unix:///path`
    /// (on Unix). TCP uses SecretConnection with the node's transport key.
    /// Waits for the signer to supply its consensus public key.
    pub fn listen(address: &str, chain_id: &str, node_key: NodeKey) -> Result<Self> {
        let (requests, incoming) = async_mpsc::unbounded_channel();
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
                            () = run(&address, &worker_chain, &node_key, incoming, ready) => {},
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
                worker: Some((stop, thread)),
            }),
            Err(e) => {
                let _ = thread.join();
                Err(e)
            }
        }
    }

    fn request(&self, chain_id: &str, message: Sum) -> Result<Sum> {
        if chain_id != self.chain_id {
            return Err(Error::new(format!(
                "remote signer chain ID mismatch: expected {:?}, got {chain_id:?}",
                self.chain_id
            )));
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.requests
            .send(Request { message, reply })
            .map_err(|_| worker_stopped())?;
        result.recv().map_err(|_| worker_stopped())?
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
        let bytes =
            raw_bytes_message_sign_bytes(chain_id, unique_id, raw_bytes).map_err(type_error)?;
        let request = Sum::SignRawBytesRequest(wire::SignRawBytesRequest {
            chain_id: chain_id.to_owned(),
            unique_id: unique_id.to_owned(),
            raw_bytes: raw_bytes.to_vec(),
        });
        let Sum::SignedRawBytesResponse(response) = self.request(chain_id, request)? else {
            return Err(unexpected("raw bytes signature"));
        };
        check_remote_error(response.error)?;
        self.verify(&bytes, &response.signature, "raw bytes")?;
        Ok(response.signature)
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

async fn run(
    address: &str,
    chain_id: &str,
    node_key: &NodeKey,
    mut requests: async_mpsc::UnboundedReceiver<Request>,
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

async fn reject_while_disconnected(requests: &mut async_mpsc::UnboundedReceiver<Request>) {
    while let Some(request) = requests.recv().await {
        let _ = request
            .reply
            .send(Err(Error::new("remote signer is disconnected")));
    }
}

async fn serve(
    session: &mut Session,
    requests: &mut async_mpsc::UnboundedReceiver<Request>,
) -> Result<()> {
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { return Ok(()) };
                let result = session.exchange(request.message).await;
                let connection_error = result.as_ref().err().cloned();
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
