//! Negotiate the released host's stream security protocols and retain the
//! selected protocol for the connection-state RPC.
use super::connections::{Connections, PeerTransport};
use futures::{AsyncRead, AsyncWrite, future::Either};
use libp2p::{
    PeerId,
    core::{
        ConnectedPoint, Endpoint,
        muxing::StreamMuxerBox,
        upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade},
    },
    identity, noise, tls, yamux,
};
use multistream_select::{Negotiated, Version, dialer_select_proto, listener_select_proto};
use std::io;

#[derive(Clone)]
pub(super) struct Config {
    tls: tls::Config,
    noise: noise::Config,
}

impl Config {
    pub(super) fn new(key: &identity::Keypair) -> anyhow::Result<Self> {
        Ok(Self {
            tls: tls::Config::new(key)?,
            noise: noise::Config::new(key)?,
        })
    }

    pub(super) async fn connect<C>(
        self,
        socket: C,
        point: ConnectedPoint,
        observed: Connections,
        transport: fn(&'static str) -> PeerTransport,
    ) -> io::Result<(PeerId, StreamMuxerBox)>
    where
        C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let dial = matches!(
            point,
            ConnectedPoint::Dialer {
                role_override: Endpoint::Dialer,
                ..
            }
        );
        let (security, socket) = select(socket, dial, ["/tls/1.0.0", "/noise"]).await?;
        let (peer, secure) = if security == "/tls/1.0.0" {
            let (peer, stream) = if dial {
                self.tls.upgrade_outbound(socket, security).await
            } else {
                self.tls.upgrade_inbound(socket, security).await
            }
            .map_err(io::Error::other)?;
            (peer, Either::Left(stream))
        } else {
            let (peer, stream) = if dial {
                self.noise.upgrade_outbound(socket, security).await
            } else {
                self.noise.upgrade_inbound(socket, security).await
            }
            .map_err(io::Error::other)?;
            (peer, Either::Right(stream))
        };
        let (protocol, socket) = select(secure, dial, ["/yamux/1.0.0"]).await?;
        let yamux = yamux::Config::default();
        let mux = if dial {
            yamux.upgrade_outbound(socket, protocol).await?
        } else {
            yamux.upgrade_inbound(socket, protocol).await?
        };
        Ok((
            peer,
            StreamMuxerBox::new(observed.observe(peer, &point, mux, transport(security))),
        ))
    }
}

async fn select<C: AsyncRead + AsyncWrite + Unpin, const N: usize>(
    socket: C,
    dial: bool,
    protocols: [&'static str; N],
) -> io::Result<(&'static str, Negotiated<C>)> {
    if dial {
        dialer_select_proto(socket, protocols, Version::V1Lazy).await
    } else {
        listener_select_proto(socket, protocols).await
    }
    .map_err(io::Error::other)
}
