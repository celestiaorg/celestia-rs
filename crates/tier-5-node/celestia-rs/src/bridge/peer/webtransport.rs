//! Native WebTransport dialing. DNS resolution is performed by the containing transport.
mod certificate;
mod listener;
mod mux;
mod rotation;
mod server_mux;
mod stream;

use futures::{AsyncWriteExt, FutureExt, future::BoxFuture};
use libp2p::{
    Multiaddr, PeerId,
    core::{
        muxing::StreamMuxerBox,
        transport::{DialOpts, ListenerId, Transport, TransportError, TransportEvent},
        upgrade::OutboundConnectionUpgrade,
    },
    identity,
    multiaddr::Protocol,
    noise,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
};
use tobi_webtransport::client::{Client, ClientConfig};

pub(super) struct Native {
    key: identity::Keypair,
    directory: PathBuf,
    listeners: HashMap<ListenerId, tokio::task::JoinHandle<()>>,
    send: tokio::sync::mpsc::Sender<(ListenerId, listener::Event)>,
    events: tokio::sync::mpsc::Receiver<(ListenerId, listener::Event)>,
    removed: VecDeque<ListenerId>,
}
impl Native {
    pub(super) fn new(key: identity::Keypair, directory: PathBuf) -> Self {
        let (send, events) = tokio::sync::mpsc::channel(32);
        Self {
            key,
            directory,
            listeners: HashMap::new(),
            send,
            events,
            removed: VecDeque::new(),
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        for task in self.listeners.values() {
            task.abort();
        }
    }
}
impl Transport for Native {
    type Output = (PeerId, StreamMuxerBox);
    type Error = io::Error;
    type ListenerUpgrade = listener::Upgrade;
    type Dial = BoxFuture<'static, io::Result<Self::Output>>;
    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        let bind = listener::socket(&addr).ok_or(TransportError::MultiaddrNotSupported(addr))?;
        let driver = listener::start(
            id,
            bind,
            &self.directory,
            self.key.clone(),
            self.send.clone(),
        )
        .map_err(TransportError::Other)?;
        self.listeners.insert(id, driver);
        Ok(())
    }
    fn remove_listener(&mut self, id: ListenerId) -> bool {
        if let Some(task) = self.listeners.remove(&id) {
            task.abort();
            self.removed.push_back(id);
            true
        } else {
            false
        }
    }
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<listener::Event> {
        if let Some(id) = self.removed.pop_front() {
            return Poll::Ready(TransportEvent::ListenerClosed {
                listener_id: id,
                reason: Ok(()),
            });
        }
        loop {
            let Some((id, event)) = ready!(self.events.poll_recv(cx)) else {
                return Poll::Pending;
            };
            if self.listeners.contains_key(&id) {
                if matches!(event, TransportEvent::ListenerClosed { .. }) {
                    self.listeners.remove(&id);
                }
                return Poll::Ready(event);
            }
        }
    }
    fn dial(
        &mut self,
        addr: Multiaddr,
        _: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        let unsupported = || TransportError::MultiaddrNotSupported(addr.clone());
        let mut parts = addr.iter().peekable();
        let ip = match parts.next() {
            Some(Protocol::Ip4(ip)) => IpAddr::V4(ip),
            Some(Protocol::Ip6(ip)) => IpAddr::V6(ip),
            _ => return Err(unsupported()),
        };
        let Some(Protocol::Udp(port)) = parts.next() else {
            return Err(unsupported());
        };
        if ip.is_unspecified() || port == 0 || parts.next() != Some(Protocol::QuicV1) {
            return Err(unsupported());
        }
        let name = if let Some(Protocol::Sni(name)) =
            parts.next_if(|part| matches!(part, Protocol::Sni(_)))
        {
            name.into_owned()
        } else {
            ip.to_string()
        };
        if parts.next() != Some(Protocol::WebTransport) {
            return Err(unsupported());
        }
        let mut pins = HashSet::new();
        while let Some(Protocol::Certhash(pin)) = parts.peek() {
            pins.insert(*pin);
            parts.next();
        }
        let has_pin = pins
            .iter()
            .any(|pin| pin.code() == 0x12 && pin.digest().len() == 32);
        let valid_suffix =
            matches!(parts.next(), None | Some(Protocol::P2p(_))) && parts.next().is_none();
        if !has_pin || !valid_suffix {
            return Err(unsupported());
        }
        let key = self.key.clone();
        let remote = SocketAddr::new(ip, port);
        Ok(async move {
            let bind = SocketAddr::new(
                if ip.is_ipv4() {
                    IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                } else {
                    IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
                },
                0,
            );
            let tls = certificate::Verifier::config(pins.clone()).map_err(io::Error::other)?;
            let client = Client::builder()
                .with_tls_config(Arc::new(tls))
                .with_io(tobi_quic::IoConfig::new(bind))
                .start()
                .map_err(io::Error::other)?;
            let config = ClientConfig::new(
                remote.to_string(),
                "/.well-known/libp2p-webtransport?type=noise",
            );
            let session = Box::pin(client.connect(tobi_quic::Connect::new(remote, name), config))
                .await
                .map_err(io::Error::other)?;
            let auth = session
                .open_bidirectional_stream()
                .await
                .map_err(io::Error::other)?;
            let (peer, mut io) = noise::Config::new(&key)
                .map_err(io::Error::other)?
                .with_webtransport_certhashes(pins)
                .upgrade_outbound(stream::Stream::new(auth), "/noise")
                .await
                .map_err(io::Error::other)?;
            io.close().await?;
            Ok((peer, StreamMuxerBox::new(mux::Mux::new(session))))
        }
        .boxed())
    }
}
