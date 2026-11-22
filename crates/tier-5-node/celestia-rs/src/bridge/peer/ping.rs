//! The libp2p ping protocol echoes a 32-byte challenge for each round trip.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use futures::{AsyncReadExt as _, AsyncWriteExt as _, StreamExt as _};
use libp2p::{PeerId, StreamProtocol};
use libp2p_stream::Control;
use rand::{RngCore as _, rngs::OsRng};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, oneshot};

const PROTOCOL: StreamProtocol = StreamProtocol::new("/ipfs/ping/1.0.0");
const TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct Ping {
    control: Control,
    outgoing: Arc<Semaphore>,
}

impl Ping {
    pub(super) fn start(mut control: Control) -> Result<Self> {
        let incoming = control.accept(PROTOCOL)?;
        tokio::spawn(
            incoming.for_each_concurrent(32, |(_, mut stream)| async move {
                let mut frame = [0; 32];
                loop {
                    let echo = async {
                        stream.read_exact(&mut frame).await?;
                        stream.write_all(&frame).await?;
                        stream.flush().await
                    };
                    if !matches!(tokio::time::timeout(TIMEOUT, echo).await, Ok(Ok(()))) {
                        break;
                    }
                }
            }),
        );
        Ok(Self {
            control,
            outgoing: Arc::new(Semaphore::new(32)),
        })
    }

    pub(super) fn measure(&self, peer: PeerId, mut reply: oneshot::Sender<Result<Value>>) {
        let Ok(slot) = self.outgoing.clone().try_acquire_owned() else {
            let _ = reply.send(Err(anyhow::anyhow!("too many pending pings")));
            return;
        };
        let mut control = self.control.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let exchange = async {
                let mut stream = control.open_stream(peer, PROTOCOL).await?;
                let mut challenge = [0; 32];
                OsRng.try_fill_bytes(&mut challenge)?;
                let start = Instant::now();
                stream.write_all(&challenge).await?;
                stream.flush().await?;
                let mut echo = [0; 32];
                stream.read_exact(&mut echo).await?;
                ensure!(challenge == echo, "invalid libp2p ping response");
                Ok(json!(u64::try_from(start.elapsed().as_nanos())?))
            };
            tokio::select! {
                () = reply.closed() => (),
                result = tokio::time::timeout(TIMEOUT, exchange) => {
                    let _ = reply.send(result.unwrap_or_else(|e| Err(e.into())));
                }
            }
        });
    }
}
