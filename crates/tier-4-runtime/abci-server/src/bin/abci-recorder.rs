//! Transparent ABCI proxy that records every frame exchanged between
//! celestia-core and an ABCI app.
//!
//! Usage: `abci-recorder <listen-addr> <upstream-addr> <out-file> [switch-cmd]`
//!
//! With `switch-cmd`, the recorder reproduces the celestia-app multiplexer's
//! binary switch: when a `FinalizeBlock` response changes the app version
//! (`consensus_param_updates.version.app`), the recorder holds the following
//! `Commit` response, runs `sh -c switch-cmd` (which stops the old app and
//! starts the next one on the same upstream address), reconnects every
//! connection to the new app, and only then forwards the `Commit` response,
//! exactly like the multiplexer, which switches apps inside `Commit`.

#![allow(clippy::print_stderr)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};

use abci_server::codec::{read_frame, write_frame};
use abci_server::record::{Direction, Record};
use celestia_core_proto::abci::{Response, response};
use prost::Message as _;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc, watch};

type Sink = Arc<Mutex<tokio::fs::File>>;

/// State shared by all connections.
struct Shared {
    upstream: String,
    sink: Sink,
    switch_cmd: Option<String>,
    /// Incremented after each completed switch.
    epoch: AtomicU64,
    /// `true` while a switch is running: no request may be forwarded.
    switching: watch::Sender<bool>,
    /// Requests awaiting a response on connections other than the one
    /// running the switch.
    outstanding: AtomicI64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (listen, upstream, out, switch_cmd) = match args.as_slice() {
        [_, l, u, o] => (l, u, o, None),
        [_, l, u, o, s] => (l, u, o, Some(s.clone())),
        _ => anyhow::bail!(
            "usage: abci-recorder <listen-addr> <upstream-addr> <out-file> [switch-cmd]"
        ),
    };
    let shared = Arc::new(Shared {
        upstream: upstream.clone(),
        sink: Arc::new(Mutex::new(tokio::fs::File::create(out).await?)),
        switch_cmd,
        epoch: AtomicU64::new(0),
        switching: watch::channel(false).0,
        outstanding: AtomicI64::new(0),
    });
    let listener = TcpListener::bind(listen).await?;
    let next = AtomicU32::new(0);
    loop {
        let (down, _) = listener.accept().await?;
        let conn = next.fetch_add(1, Ordering::SeqCst);
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            if let Err(e) = proxy(conn, down, shared).await {
                eprintln!("conn {conn}: {e}");
            }
        });
    }
}

async fn record(sink: &Sink, conn: u32, dir: Direction, bytes: &[u8]) -> anyhow::Result<()> {
    let mut f = sink.lock().await;
    f.write_all(
        &Record {
            conn,
            dir,
            bytes: bytes.to_vec(),
        }
        .encode(),
    )
    .await?;
    f.flush().await?;
    Ok(())
}

/// A frame read by one of the reader tasks; `None` is EOF. Upstream frames
/// carry the epoch of the connection they were read from.
enum Event {
    Down(Option<Vec<u8>>),
    Up(u64, Option<Vec<u8>>),
}

fn spawn_reader(
    r: OwnedReadHalf,
    tx: mpsc::UnboundedSender<Event>,
    wrap: impl Fn(Option<Vec<u8>>) -> Event + Send + 'static,
) {
    tokio::spawn(async move {
        let mut r = BufReader::new(r);
        while let Ok(Some(f)) = read_frame(&mut r).await {
            if tx.send(wrap(Some(f))).is_err() {
                return;
            }
        }
        let _ = tx.send(wrap(None));
    });
}

async fn connect_up(
    shared: &Shared,
    tx: &mpsc::UnboundedSender<Event>,
    epoch: u64,
) -> anyhow::Result<OwnedWriteHalf> {
    let up = TcpStream::connect(shared.upstream.as_str()).await?;
    up.set_nodelay(true)?;
    let (ur, uw) = up.into_split();
    spawn_reader(ur, tx.clone(), move |f| Event::Up(epoch, f));
    Ok(uw)
}

/// The app version a `FinalizeBlock` response switches to, if any.
fn finalize_app_version(frame: &[u8]) -> Option<u64> {
    match Response::decode(frame).ok()?.value? {
        response::Value::FinalizeBlock(r) => Some(r.consensus_param_updates?.version?.app),
        _ => None,
    }
}

fn is_commit(frame: &[u8]) -> bool {
    matches!(
        Response::decode(frame).ok().and_then(|r| r.value),
        Some(response::Value::Commit(_))
    )
}

/// Runs the switch command once every request of every connection is
/// answered; new requests are held meanwhile (`switching` is set).
async fn run_switch(shared: &Shared, cmd: &str) -> anyhow::Result<()> {
    while shared.outstanding.load(Ordering::SeqCst) > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!("switching app: {cmd}");
    let status = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .await?;
    anyhow::ensure!(status.success(), "switch command failed: {status}");
    shared.epoch.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

async fn proxy(conn: u32, down: TcpStream, shared: Arc<Shared>) -> anyhow::Result<()> {
    down.set_nodelay(true)?;
    let (dr, mut dw) = down.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    spawn_reader(dr, tx.clone(), Event::Down);
    let mut epoch = shared.epoch.load(Ordering::SeqCst);
    let mut uw = connect_up(&shared, &tx, epoch).await?;
    let mut outstanding: i64 = 0;
    let mut app_version: Option<u64> = None;
    let mut switch_pending = false;
    // Responses held back while this connection drives a switch.
    let mut held: Vec<Vec<u8>> = Vec::new();
    let mut switching_rx = shared.switching.subscribe();
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Down(None) => break,
            Event::Down(Some(frame)) => {
                record(&shared.sink, conn, Direction::Request, &frame).await?;
                // Hold requests while a switch runs (except on the
                // connection driving it, whose pipelined requests the old
                // app still answers).
                while held.is_empty() && *switching_rx.borrow_and_update() {
                    switching_rx.changed().await?;
                }
                let now = shared.epoch.load(Ordering::SeqCst);
                if now != epoch {
                    anyhow::ensure!(
                        outstanding == 0,
                        "app switched with {outstanding} requests in flight"
                    );
                    epoch = now;
                    uw = connect_up(&shared, &tx, epoch).await?;
                }
                outstanding += 1;
                shared.outstanding.fetch_add(1, Ordering::SeqCst);
                write_frame(&mut uw, &frame).await?;
            }
            Event::Up(e, None) => {
                // EOF of an app being replaced by a switch is expected.
                let replaced =
                    *shared.switching.borrow() || shared.epoch.load(Ordering::SeqCst) != epoch;
                if e == epoch && !replaced {
                    // The app closed the connection: pass it on.
                    break;
                }
            }
            Event::Up(e, Some(frame)) => {
                if e != epoch {
                    continue;
                }
                record(&shared.sink, conn, Direction::Response, &frame).await?;
                outstanding -= 1;
                shared.outstanding.fetch_sub(1, Ordering::SeqCst);
                let mut keep_back = !held.is_empty();
                if !keep_back && shared.switch_cmd.is_some() {
                    if let Some(v) = finalize_app_version(&frame) {
                        if app_version.is_some_and(|old| old != v) {
                            switch_pending = true;
                        }
                        app_version = Some(v);
                    } else if switch_pending && is_commit(&frame) {
                        switch_pending = false;
                        // Hold the Commit response (and anything after it)
                        // until the next app serves this address.
                        shared.switching.send_replace(true);
                        keep_back = true;
                    }
                }
                if keep_back {
                    held.push(frame);
                } else {
                    write_frame(&mut dw, &frame).await?;
                }
                if !held.is_empty() && outstanding == 0 {
                    if let Some(cmd) = &shared.switch_cmd {
                        run_switch(&shared, cmd).await?;
                    }
                    epoch = shared.epoch.load(Ordering::SeqCst);
                    uw = connect_up(&shared, &tx, epoch).await?;
                    for f in held.drain(..) {
                        write_frame(&mut dw, &f).await?;
                    }
                    shared.switching.send_replace(false);
                }
            }
        }
    }
    dw.shutdown().await?;
    Ok(())
}
