//! Native WebTransport dialing. DNS resolution is performed by the containing transport.
mod certificate;
mod mux;
mod stream;

use futures::{
    AsyncWriteExt, FutureExt,
    future::{BoxFuture, Pending},
};
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
    collections::HashSet,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tobi_webtransport::client::{Client, ClientConfig};

pub(super) struct Dialer(pub(super) identity::Keypair);

impl Transport for Dialer {
    type Output = (PeerId, StreamMuxerBox);
    type Error = io::Error;
    type ListenerUpgrade = Pending<io::Result<Self::Output>>;
    type Dial = BoxFuture<'static, io::Result<Self::Output>>;
    fn listen_on(
        &mut self,
        _: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        Err(TransportError::MultiaddrNotSupported(addr))
    }
    fn remove_listener(&mut self, _: ListenerId) -> bool {
        false
    }
    fn poll(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Poll::Pending
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
        let key = self.0.clone();
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
