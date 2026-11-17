//! Record the signed CLI fixture against a fresh standalone v11 Go app.
//! See conformance/fixtures/devnet/v11-cli/README.md.
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::{SocketAddr, TcpStream};

use abci_server::record::{Direction, Record};
use anyhow::{Context as _, ensure};
use celestia_proto::celestia_core_proto::abci::{
    Request, RequestFinalizeBlock, RequestProcessProposal, Response, request, response,
};
use devnet_conformance::querycorpus::net::{read_frame, write_frame};
use prost::Message;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let [_, addr, output] = args.as_slice() else {
        anyhow::bail!("usage: record_v11_cli <loopback-abci-addr> <new-output.rec>");
    };
    let addr: SocketAddr = addr.parse()?;
    ensure!(addr.ip().is_loopback(), "expected a local Go app");
    let mut streams = BTreeMap::new();
    let mut capture = Vec::new();
    let flush = Request {
        value: Some(request::Value::Flush(Default::default())),
    }
    .encode_to_vec();
    for exchange in devnet_conformance::load_exchanges("cli")? {
        let mut req = exchange.request;
        match &mut req.value {
            Some(request::Value::InitChain(r)) => {
                r.consensus_params
                    .as_mut()
                    .context("missing consensus params")?
                    .version
                    .as_mut()
                    .context("missing app version")?
                    .app = 11;
            }
            Some(
                request::Value::ProcessProposal(RequestProcessProposal { header, .. })
                | request::Value::FinalizeBlock(RequestFinalizeBlock { header, .. }),
            ) => {
                header
                    .as_mut()
                    .context("missing block header")?
                    .version
                    .as_mut()
                    .context("missing header version")?
                    .app = 11;
            }
            Some(
                request::Value::PrepareProposal(_)
                | request::Value::CheckTx(_)
                | request::Value::Commit(_),
            ) => {}
            _ => continue,
        }
        let conn = exchange.conn;
        let stream = match streams.entry(conn) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let stream = TcpStream::connect(addr)?;
                stream.set_nodelay(true)?;
                e.insert(stream)
            }
        };
        let bytes = req.encode_to_vec();
        write_frame(stream, &bytes)?;
        write_frame(stream, &flush)?;
        let reply = read_frame(stream)?;
        let end = Response::decode(read_frame(stream)?.as_slice())?;
        ensure!(
            matches!(end.value, Some(response::Value::Flush(_))),
            "missing flush response"
        );
        for (dir, bytes) in [(Direction::Request, bytes), (Direction::Response, reply)] {
            capture.extend(Record { conn, dir, bytes }.encode());
        }
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&capture)?;
    Ok(())
}
