//! Fixed-size NAT-PMP requests and replies (RFC 6886, sections 3.2 and 3.3).
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
    time::Duration,
};
use tokio::net::UdpSocket;

#[derive(PartialEq, Eq)]
pub(super) struct Client {
    router: SocketAddrV4,
    pub(super) local: Ipv4Addr,
}

impl Client {
    pub(super) async fn new(router: Ipv4Addr) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        socket.connect((router, 5351)).await?;
        let std::net::IpAddr::V4(local) = socket.local_addr()?.ip() else {
            return Err(io::ErrorKind::Unsupported.into());
        };
        Ok(Self {
            router: SocketAddrV4::new(router, 5351),
            local,
        })
    }

    async fn request<const N: usize>(&self, bytes: &[u8]) -> io::Result<[u8; N]> {
        // A fresh connected socket makes delayed replies from previous requests
        // unambiguous and accepts packets only from the selected router.
        let socket = UdpSocket::bind((self.local, 0)).await?;
        socket.connect(self.router).await?;
        for delay in [250, 500, 1000] {
            socket.send(bytes).await?;
            let mut reply = [0u8; 32];
            let Ok(received) =
                tokio::time::timeout(Duration::from_millis(delay), socket.recv(&mut reply)).await
            else {
                continue;
            };
            if received? != N
                || reply[0] != 0
                || reply[1] != bytes[1] | 128
                || reply[2..4] != [0, 0]
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid or unsuccessful NAT-PMP response",
                ));
            }
            return reply[..N].try_into().map_err(io::Error::other);
        }
        Err(io::ErrorKind::TimedOut.into())
    }

    pub(super) async fn external(&self) -> io::Result<Ipv4Addr> {
        let response = self.request::<12>(&[0, 0]).await?;
        Ok(Ipv4Addr::new(
            response[8],
            response[9],
            response[10],
            response[11],
        ))
    }

    pub(super) async fn map(
        &self,
        opcode: u8,
        inside: u16,
        outside: u16,
        seconds: u32,
    ) -> io::Result<(u16, u32)> {
        let mut request = [0u8; 12];
        request[1] = opcode;
        request[4..6].copy_from_slice(&inside.to_be_bytes());
        request[6..8].copy_from_slice(&outside.to_be_bytes());
        request[8..12].copy_from_slice(&seconds.to_be_bytes());
        let response = self.request::<16>(&request).await?;
        let port = u16::from_be_bytes([response[10], response[11]]);
        let lifetime = u32::from_be_bytes(response[12..16].try_into().map_err(io::Error::other)?);
        if response[8..10] != inside.to_be_bytes() || (seconds != 0 && (port == 0 || lifetime == 0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NAT-PMP mapping does not match the requested port or lease",
            ));
        }
        Ok((port, lifetime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accepts_the_assigned_external_port_and_rejects_another_internal_port() {
        let router = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = Client {
            router: SocketAddrV4::new(Ipv4Addr::LOCALHOST, router.local_addr().unwrap().port()),
            local: Ipv4Addr::LOCALHOST,
        };
        let gateway = tokio::spawn(async move {
            for inside in [9000u16, 8000] {
                let mut request = [0u8; 12];
                let (n, source) = router.recv_from(&mut request).await.unwrap();
                assert_eq!(n, 12);
                assert_eq!(request, [0, 2, 0, 0, 0x23, 0x28, 0x23, 0x28, 0, 0, 0, 60]);
                let mut reply = [0u8; 16];
                reply[1] = 130;
                reply[8..10].copy_from_slice(&inside.to_be_bytes());
                reply[10..12].copy_from_slice(&4567u16.to_be_bytes());
                reply[12..16].copy_from_slice(&45u32.to_be_bytes());
                router.send_to(&reply, source).await.unwrap();
            }
        });
        assert_eq!(client.map(2, 9000, 9000, 60).await.unwrap(), (4567, 45));
        assert_eq!(
            client.map(2, 9000, 9000, 60).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        gateway.await.unwrap();
    }
}
