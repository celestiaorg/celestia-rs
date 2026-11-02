//! A tokio ABCI socket server.
//!
//! celestia-core opens several connections (consensus, mempool, query,
//! snapshot). Each is served by its own task; requests on one connection are
//! answered strictly in order. The application decides how to synchronize
//! state shared across connections.

use std::sync::Arc;

use celestia_core_proto::abci::{
    Request, Response, ResponseException, ResponseFlush, request, response,
};
use prost::Message;
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

use crate::codec::{read_frame, write_frame};

/// An ABCI application. `handle` is called for every request except
/// `Flush`, which the server answers itself.
pub trait Application: Send + Sync + 'static {
    /// Handles one request and returns the response value.
    fn handle(&self, request: request::Value) -> response::Value;

    /// Handles a request while preserving a response envelope whose value is
    /// absent. Most applications always return a value; Go BaseApp can return
    /// a nil Query response for an unknown P2P filter type.
    fn handle_envelope(&self, request: request::Value) -> Option<response::Value> {
        Some(self.handle(request))
    }
}

/// Serves `app` on `listener` until the listener fails.
pub async fn serve<A: Application>(listener: TcpListener, app: Arc<A>) -> anyhow::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        tracing::info!(%peer, "abci connection");
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            if let Err(err) = serve_conn(stream, app).await {
                tracing::warn!(%peer, %err, "abci connection closed with error");
            }
        });
    }
}

async fn serve_conn<A: Application>(stream: TcpStream, app: Arc<A>) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    let (r, w) = stream.into_split();
    let mut r = BufReader::new(r);
    let mut w = BufWriter::new(w);
    while let Some(frame) = read_frame(&mut r).await? {
        let req = Request::decode(frame.as_slice())?;
        let (value, flush) = match req.value {
            Some(request::Value::Flush(_)) => {
                (Some(response::Value::Flush(ResponseFlush {})), true)
            }
            Some(v) => {
                let app = Arc::clone(&app);
                (
                    tokio::task::spawn_blocking(move || app.handle_envelope(v)).await?,
                    false,
                )
            }
            None => (
                Some(response::Value::Exception(ResponseException {
                    error: "empty request".into(),
                })),
                true,
            ),
        };
        let resp = Response { value };
        write_frame(&mut w, &resp.encode_to_vec()).await?;
        if flush {
            w.flush().await?;
        }
    }
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_core_proto::abci::{RequestEcho, RequestFlush, ResponseEcho};

    struct Echo;
    impl Application for Echo {
        fn handle(&self, request: request::Value) -> response::Value {
            match request {
                request::Value::Echo(e) => {
                    response::Value::Echo(ResponseEcho { message: e.message })
                }
                _ => response::Value::Exception(ResponseException {
                    error: "unsupported".into(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn echo_and_flush() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(serve(listener, Arc::new(Echo)));
        let mut s = TcpStream::connect(addr).await?;
        for v in [
            request::Value::Echo(RequestEcho {
                message: "a".into(),
            }),
            request::Value::Flush(RequestFlush {}),
        ] {
            write_frame(&mut s, &Request { value: Some(v) }.encode_to_vec()).await?;
        }
        let a = Response::decode(read_frame(&mut s).await?.unwrap_or_default().as_slice())?;
        assert_eq!(
            a.value,
            Some(response::Value::Echo(ResponseEcho {
                message: "a".into()
            }))
        );
        let b = Response::decode(read_frame(&mut s).await?.unwrap_or_default().as_slice())?;
        assert_eq!(b.value, Some(response::Value::Flush(ResponseFlush {})));
        Ok(())
    }

    #[tokio::test]
    async fn malformed_connections_do_not_stop_other_clients() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(serve(listener, Arc::new(Echo)));
        // A framing overflow and an invalid protobuf request must each close
        // only their own connection, without dispatching a partial request.
        for bytes in [vec![0xff; 10], vec![1, 0xff]] {
            let mut client = TcpStream::connect(addr).await?;
            client.write_all(&bytes).await?;
            client.shutdown().await?;
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut client))
                    .await??;
            assert_eq!(frame, None);
        }
        let mut healthy = TcpStream::connect(addr).await?;
        write_frame(
            &mut healthy,
            &Request {
                value: Some(request::Value::Flush(RequestFlush {})),
            }
            .encode_to_vec(),
        )
        .await?;
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut healthy))
                .await??
                .expect("healthy client receives a response");
        assert_eq!(
            Response::decode(frame.as_slice())?.value,
            Some(response::Value::Flush(ResponseFlush {}))
        );
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn half_close_flushes_pending_responses_in_order() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut client = TcpStream::connect(listener.local_addr()?).await?;
        let (stream, _) = listener.accept().await?;
        let connection = tokio::spawn(serve_conn(stream, Arc::new(Echo)));
        for message in ["first", "second"] {
            write_frame(
                &mut client,
                &Request {
                    value: Some(request::Value::Echo(RequestEcho {
                        message: message.into(),
                    })),
                }
                .encode_to_vec(),
            )
            .await?;
        }
        // No Flush request: EOF on the read half must drain buffered replies.
        client.shutdown().await?;
        for message in ["first", "second"] {
            let frame =
                tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut client))
                    .await??
                    .expect("buffered reply survives half-close");
            assert_eq!(
                Response::decode(frame.as_slice())?.value,
                Some(response::Value::Echo(ResponseEcho {
                    message: message.into(),
                }))
            );
        }
        assert_eq!(read_frame(&mut client).await?, None);
        connection.await??;
        Ok(())
    }

    #[tokio::test]
    async fn empty_request_flushes_exception_without_closing_connection() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut client = TcpStream::connect(listener.local_addr()?).await?;
        let (stream, _) = listener.accept().await?;
        let connection = tokio::spawn(serve_conn(stream, Arc::new(Echo)));
        write_frame(&mut client, &Request { value: None }.encode_to_vec()).await?;
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut client))
                .await??
                .expect("exception is flushed immediately");
        assert_eq!(
            Response::decode(frame.as_slice())?.value,
            Some(response::Value::Exception(ResponseException {
                error: "empty request".into(),
            }))
        );
        write_frame(
            &mut client,
            &Request {
                value: Some(request::Value::Echo(RequestEcho {
                    message: "still connected".into(),
                })),
            }
            .encode_to_vec(),
        )
        .await?;
        client.shutdown().await?;
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut client))
                .await??
                .expect("connection accepts another request");
        assert_eq!(
            Response::decode(frame.as_slice())?.value,
            Some(response::Value::Echo(ResponseEcho {
                message: "still connected".into(),
            }))
        );
        connection.await??;
        Ok(())
    }
}
