use super::{
    rotation::{Epoch, Resolver},
    server_mux,
};
use futures::{AsyncWriteExt, StreamExt, TryFutureExt, future::BoxFuture};
use libp2p::{
    Multiaddr, PeerId,
    core::{
        muxing::StreamMuxerBox,
        transport::{ListenerId, TransportEvent},
        upgrade::InboundConnectionUpgrade,
    },
    identity,
    multiaddr::Protocol,
    multihash::Multihash,
    noise,
};
use std::{
    collections::HashSet,
    io,
    net::{IpAddr, SocketAddr},
    path::Path,
};
use tobi_webtransport::server::{ServerLimits, WtSessionRequest};
use tokio::sync::mpsc;

pub(super) type Upgrade = BoxFuture<'static, io::Result<(PeerId, StreamMuxerBox)>>;
pub(super) type Event = TransportEvent<Upgrade, io::Error>;

fn address(socket: SocketAddr, pins: &[Multihash<64>]) -> Multiaddr {
    let mut addr = Multiaddr::from(socket.ip())
        .with(Protocol::Udp(socket.port()))
        .with(Protocol::QuicV1)
        .with(Protocol::WebTransport);
    for pin in pins {
        addr.push(Protocol::Certhash(*pin));
    }
    addr
}

pub(super) fn socket(addr: &Multiaddr) -> Option<SocketAddr> {
    let mut parts = addr.iter();
    let ip = match parts.next()? {
        Protocol::Ip4(ip) => IpAddr::V4(ip),
        Protocol::Ip6(ip) => IpAddr::V6(ip),
        _ => return None,
    };
    let Protocol::Udp(port) = parts.next()? else {
        return None;
    };
    (parts.next() == Some(Protocol::QuicV1)
        && parts.next() == Some(Protocol::WebTransport)
        && parts.next().is_none())
    .then_some(SocketAddr::new(ip, port))
}

pub(super) fn start(
    id: ListenerId,
    bind: SocketAddr,
    dir: &Path,
    key: identity::Keypair,
    events: mpsc::Sender<(ListenerId, Event)>,
) -> io::Result<tokio::task::JoinHandle<()>> {
    let dir = dir.to_owned();
    let digest = hash::sha256(&key.public().to_peer_id().to_bytes());
    let offset = i64::from(u16::from_be_bytes([digest[0], digest[1]]));
    let mut epoch = Epoch::load(&dir, offset)?;
    let (resolver, tls) = Resolver::tls(&epoch)?;
    let mut config = tobi_quic::core::connection::ConnectionConfig::default();
    config.transport_params.reset_stream_at = true;
    let mut server = tobi_quic::Server::builder()
        .with_tls(tls)
        .with_io(tobi_quic::IoConfig::new(bind))
        .with_transport(config)
        .with_datagram(tobi_quic::DatagramConfig::default())
        .start()
        .map_err(io::Error::other)?;
    let bound = server.local_addr();
    let mut watch = if bind.ip().is_unspecified() {
        Some(if_watch::tokio::IfWatcher::new()?)
    } else {
        None
    };
    Ok(tokio::spawn(async move {
        let report = |event| {
            events
                .send((id, event))
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
        };
        let result: io::Result<()> = async {
            let mut ips = HashSet::new();
            if watch.is_none() {
                ips.insert(bound.ip());
                report(Event::NewAddress { listener_id: id, listen_addr: address(bound, &epoch.advertised) }).await?;
            }
            let delay = tokio::time::sleep(epoch.remaining);
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    result = server.accept() => {
                        let event = match result {
                            Ok(connection) => incoming(id, connection, key.clone(), &epoch).await,
                            Err(error) => Err(io::Error::other(error)),
                        };
                        report(event.unwrap_or_else(|error| Event::ListenerError { listener_id: id, error })).await?;
                    }
                    event = async { match &mut watch { Some(watcher) => watcher.next().await, None => std::future::pending().await } } => {
                        let (ip, up) = match event.ok_or(io::ErrorKind::UnexpectedEof)?? {
                            if_watch::IfEvent::Up(network) => (network.addr(), true),
                            if_watch::IfEvent::Down(network) => (network.addr(), false),
                        };
                        if ip.is_ipv4() != bound.is_ipv4() { continue; }
                        let listen_addr = address(SocketAddr::new(ip, bound.port()), &epoch.advertised);
                        if up && ips.insert(ip) {
                            report(Event::NewAddress { listener_id: id, listen_addr }).await?;
                        } else if !up && ips.remove(&ip) {
                            report(Event::AddressExpired { listener_id: id, listen_addr }).await?;
                        }
                    }
                    () = &mut delay => {
                        let next = Epoch::load(&dir, offset)?;
                        resolver.rotate(&next);
                        for ip in &ips {
                            let socket = SocketAddr::new(*ip, bound.port());
                            report(Event::NewAddress {listener_id: id, listen_addr: address(socket, &next.advertised)}).await?;
                            report(Event::AddressExpired {listener_id: id, listen_addr: address(socket, &epoch.advertised)}).await?;
                        }
                        epoch = next;
                        delay.as_mut().reset(tokio::time::Instant::now() + epoch.remaining);
                    }
                }
            }
        }.await;
        let _ = report(Event::ListenerClosed {
            listener_id: id,
            reason: result,
        })
        .await;
    }))
}

async fn incoming(
    id: ListenerId,
    connection: tobi_quic::Connection,
    key: identity::Keypair,
    epoch: &Epoch,
) -> io::Result<Event> {
    let remote = connection.remote_addr().await.map_err(io::Error::other)?;
    let local = connection.local_addr().await.map_err(io::Error::other)?;
    let bindings = epoch.bindings;
    let upgrade = Box::pin(async move {
        let limits = ServerLimits {
            max_sessions: 1,
            ..Default::default()
        };
        let request = Box::pin(WtSessionRequest::from_connection(connection, limits, false))
            .await
            .map_err(io::Error::other)?;
        if request.path() != "/.well-known/libp2p-webtransport?type=noise" {
            request.reject(404).await.map_err(io::Error::other)?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown libp2p WebTransport path",
            ));
        }
        let session = Box::pin(request.accept()).await.map_err(io::Error::other)?;
        let stream = session
            .connection()
            .accept_bidirectional_stream()
            .await
            .map_err(io::Error::other)?;
        let auth = server_mux::incoming(stream, session.session_id()).await?;
        let (peer, mut auth) = noise::Config::new(&key)
            .map_err(io::Error::other)?
            .with_webtransport_certhashes(bindings.into_iter().collect())
            .upgrade_inbound(auth, "/noise")
            .await
            .map_err(io::Error::other)?;
        auth.close().await?;
        Ok((
            peer,
            StreamMuxerBox::new(server_mux::Mux::new(session, auth)),
        ))
    });
    Ok(Event::Incoming {
        listener_id: id,
        local_addr: address(local, &epoch.advertised),
        send_back_addr: address(remote, &[]),
        upgrade,
    })
}
