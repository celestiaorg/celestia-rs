//! `DELIVER_TRACE=<file>`: a running SHA-256 of every response of the Rust
//! app (CheckTx, PrepareProposal, ProcessProposal, FinalizeBlock, Commit;
//! errors by their text), written after each `Commit` as `h=<height>
//! <digest>`. The same scenario and seed must give a byte-identical file
//! whatever the build profile, thread count, signature cache or target
//! (MILESTONES.md C1.3 / F1.9); the first differing line names the height.

use std::io::Write as _;

use celestia_proto::celestia_core_proto::abci;

/// The Rust app, with each ABCI response folded into the trace. Every other
/// call goes to the app through `Deref`.
pub struct Traced {
    app: app::App,
    trace: Option<Trace>,
}

struct Trace {
    out: std::io::BufWriter<std::fs::File>,
    /// The responses since the last `Commit`.
    hash: hash::Sha256Hasher,
    /// `sha256(previous || block)` over the committed blocks.
    running: [u8; 32],
    height: i64,
}

impl Traced {
    /// The app, traced when `DELIVER_TRACE` names a file.
    pub fn new(app: app::App) -> anyhow::Result<Self> {
        let trace = match std::env::var("DELIVER_TRACE") {
            Ok(path) => Some(Trace {
                out: std::io::BufWriter::new(std::fs::File::create(path)?),
                hash: hash::Sha256Hasher::new(),
                running: [0; 32],
                height: 0,
            }),
            Err(_) => None,
        };
        Ok(Self { app, trace })
    }

    fn rec<T: prost::Message, E: std::fmt::Display>(&mut self, tag: u8, r: &Result<T, E>) {
        if let Some(t) = &mut self.trace {
            t.hash.update(&[tag]);
            match r {
                Ok(v) => {
                    let b = v.encode_to_vec();
                    t.hash.update(&[0]);
                    t.hash.update(&(b.len() as u64).to_be_bytes());
                    t.hash.update(&b);
                }
                Err(e) => {
                    let s = e.to_string();
                    t.hash.update(&[1]);
                    t.hash.update(&(s.len() as u64).to_be_bytes());
                    t.hash.update(s.as_bytes());
                }
            }
        }
    }

    pub fn check_tx(
        &mut self,
        req: &abci::RequestCheckTx,
    ) -> Result<abci::ResponseCheckTx, String> {
        let r = self.app.check_tx(req);
        self.rec(b'c', &r);
        r
    }

    pub fn prepare_proposal(
        &mut self,
        req: &abci::RequestPrepareProposal,
    ) -> sdk::Result<abci::ResponsePrepareProposal> {
        let r = self.app.prepare_proposal(req);
        self.rec(b'p', &r);
        r
    }

    pub fn process_proposal(
        &mut self,
        req: &abci::RequestProcessProposal,
    ) -> sdk::Result<abci::ResponseProcessProposal> {
        let r = self.app.process_proposal(req);
        self.rec(b'q', &r);
        r
    }

    pub fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> sdk::Result<abci::ResponseFinalizeBlock> {
        if let Some(t) = &mut self.trace {
            t.height = req.height;
        }
        let r = self.app.finalize_block(req);
        self.rec(b'f', &r);
        r
    }

    pub fn commit(&mut self) -> sdk::Result<abci::ResponseCommit> {
        let r = self.app.commit();
        self.rec(b'm', &r);
        if let Some(t) = &mut self.trace {
            let block = std::mem::take(&mut t.hash).finalize();
            let mut h = hash::Sha256Hasher::new();
            h.update(&t.running);
            h.update(&block);
            t.running = h.finalize();
            let d = t.running;
            // A trace that cannot be written is reported when the run's
            // traces are compared (a short file).
            let _ = writeln!(t.out, "h={} {}", t.height, hex::encode(d));
            let _ = t.out.flush();
        }
        r
    }
}

impl std::ops::Deref for Traced {
    type Target = app::App;
    fn deref(&self) -> &app::App {
        &self.app
    }
}

impl std::ops::DerefMut for Traced {
    fn deref_mut(&mut self) -> &mut app::App {
        &mut self.app
    }
}
