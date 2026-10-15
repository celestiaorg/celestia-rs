//! The Go side: a `conformance/fuzzoracle/{v9,v10}` process answering one
//! request per line.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, bail};

use crate::Out;

/// A running Go oracle.
pub struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// The app version it implements.
    pub app_version: u64,
}

/// Repository root (two levels above this crate).
#[must_use]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The oracle binary for `app_version`: `$FUZZ_ORACLE_V9` / `$FUZZ_ORACLE_V10`
/// if set, else `target/fuzzoracle/fuzzoracle-v<N>`, built with `go build`
/// when missing (needs Go and the module cache / proxy; `GOPATH` and
/// `GOTOOLCHAIN` are taken from the environment).
pub fn oracle_binary(app_version: u64) -> anyhow::Result<PathBuf> {
    let dir = if app_version < 10 { "v9" } else { "v10" };
    if let Ok(p) = std::env::var(format!("FUZZ_ORACLE_{}", dir.to_uppercase())) {
        return Ok(PathBuf::from(p));
    }
    let root = repo_root();
    let out = root.join(format!("target/fuzzoracle/fuzzoracle-{dir}"));
    if out.exists() {
        return Ok(out);
    }
    let status = Command::new("go")
        .args(["build", "-o"])
        .arg(&out)
        .arg(".")
        .current_dir(root.join("conformance/fuzzoracle").join(dir))
        .env("GOFLAGS", "-mod=mod")
        .status()
        .context("running go build (is Go installed?)")?;
    if !status.success() {
        bail!("go build of the {dir} oracle failed");
    }
    Ok(out)
}

/// The driver restarts the oracle after this many inputs, so a slow leak
/// or fragmentation in the Go process cannot grow without bound.
pub const RESTART_EVERY: u64 = 50_000;

/// The oracle's answer for an input above its size cap (skipped).
pub const TOO_LARGE: &str = "oracle: input too large";

impl Oracle {
    /// Starts the oracle for `app_version` (9 or 10).
    pub fn start(app_version: u64) -> anyhow::Result<Self> {
        let bin = oracle_binary(app_version)?;
        // Bound the Go heap: the oracle shares the machine with other
        // nodes (the driver also restarts it every RESTART_EVERY inputs).
        let mut child = Command::new(&bin)
            .env(
                "GOMEMLIMIT",
                std::env::var("GOMEMLIMIT").unwrap_or_else(|_| "1500MiB".into()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| bin.display().to_string())?;
        let stdin = child.stdin.take().context("stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            app_version,
        })
    }

    /// Sends a batch of `(op, input)` requests and reads the answers.
    pub fn query_batch(&mut self, reqs: &[(&str, &[u8])]) -> anyhow::Result<Vec<Out>> {
        let mut buf = String::new();
        for (op, input) in reqs {
            buf.push_str(op);
            buf.push(' ');
            buf.push_str(&hex::encode(input));
            buf.push('\n');
        }
        // Write from a thread so a large batch cannot deadlock on full pipes.
        let mut out = Vec::with_capacity(reqs.len());
        std::thread::scope(|s| -> anyhow::Result<()> {
            let stdin = &mut self.stdin;
            let w = s.spawn(move || {
                stdin.write_all(buf.as_bytes())?;
                stdin.flush()
            });
            for _ in 0..reqs.len() {
                let mut line = String::new();
                if self.stdout.read_line(&mut line)? == 0 {
                    bail!("oracle exited");
                }
                out.push(serde_json::from_str(&line).with_context(|| line.clone())?);
            }
            w.join().map_err(|_| anyhow::anyhow!("writer panicked"))??;
            Ok(())
        })?;
        Ok(out)
    }

    /// One request.
    pub fn query(&mut self, op: &str, input: &[u8]) -> anyhow::Result<Out> {
        let mut v = self.query_batch(&[(op, input)])?;
        v.pop().context("no answer")
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
