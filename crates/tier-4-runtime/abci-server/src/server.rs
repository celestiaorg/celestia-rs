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
            Some(request::Value::Flush(_)) => (response::Value::Flush(ResponseFlush {}), true),
            Some(v) => {
                let app = Arc::clone(&app);
                (
                    tokio::task::spawn_blocking(move || app.handle(v)).await?,
                    false,
                )
            }
            None => (
                response::Value::Exception(ResponseException {
                    error: "empty request".into(),
                }),
                true,
            ),
        };
        let resp = Response { value: Some(value) };
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
}
