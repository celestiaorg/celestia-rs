//! Transparent ABCI proxy that records every frame exchanged between
//! celestia-core and an ABCI app.
//!
//! Usage: `abci-recorder <listen-addr> <upstream-addr> <out-file>`

#![allow(clippy::print_stderr)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use abci_server::codec::{read_frame, write_frame};
use abci_server::record::{Direction, Record};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

type Sink = Arc<Mutex<tokio::fs::File>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, listen, upstream, out] = args.as_slice() else {
        anyhow::bail!("usage: abci-recorder <listen-addr> <upstream-addr> <out-file>");
    };
    let sink: Sink = Arc::new(Mutex::new(tokio::fs::File::create(out).await?));
    let listener = TcpListener::bind(listen).await?;
    let next = AtomicU32::new(0);
    loop {
        let (down, _) = listener.accept().await?;
        let conn = next.fetch_add(1, Ordering::SeqCst);
        let up = TcpStream::connect(upstream.as_str()).await?;
        let sink = Arc::clone(&sink);
        tokio::spawn(async move {
            if let Err(e) = proxy(conn, down, up, sink).await {
                eprintln!("conn {conn}: {e}");
            }
        });
    }
}

async fn proxy(conn: u32, down: TcpStream, up: TcpStream, sink: Sink) -> anyhow::Result<()> {
    down.set_nodelay(true)?;
    up.set_nodelay(true)?;
    let (dr, dw) = down.into_split();
    let (ur, uw) = up.into_split();
    let a = tokio::spawn(pipe(conn, Direction::Request, dr, uw, Arc::clone(&sink)));
    let b = tokio::spawn(pipe(conn, Direction::Response, ur, dw, sink));
    a.await??;
    b.await??;
    Ok(())
}

async fn pipe(
    conn: u32,
    dir: Direction,
    r: tokio::net::tcp::OwnedReadHalf,
    mut w: tokio::net::tcp::OwnedWriteHalf,
    sink: Sink,
) -> anyhow::Result<()> {
    let mut r = BufReader::new(r);
    while let Some(frame) = read_frame(&mut r).await? {
        {
            let mut f = sink.lock().await;
            f.write_all(
                &Record {
                    conn,
                    dir,
                    bytes: frame.clone(),
                }
                .encode(),
            )
            .await?;
            f.flush().await?;
        }
        write_frame(&mut w, &frame).await?;
    }
    w.shutdown().await?;
    Ok(())
}
