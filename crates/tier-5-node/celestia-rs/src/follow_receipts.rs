//! Append-only evidence for explicitly requested RPC follower runs.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use ctypes::Header;
use nodestore::{State, StateStore};
use serde_json::{Value, json};

use crate::follow::{FollowOptions, FollowStats};

/// Optional receipt output for one follower invocation.
#[derive(Debug, Clone)]
pub struct FollowReceiptOptions {
    /// New JSONL file. Existing files are never overwritten or appended to.
    pub path: PathBuf,
    /// Full lowercase source revision claimed by the operator. Bind it to the
    /// executable with a separate clean-source build/qualification receipt.
    pub candidate_commit: String,
}

#[derive(Debug)]
pub(crate) struct Recorder {
    file: BufWriter<File>,
    last_verified_height: Option<i64>,
}

impl Recorder {
    pub(crate) fn create(
        config: &FollowReceiptOptions,
        opts: &FollowOptions,
        chain: &str,
    ) -> anyhow::Result<Self> {
        if config.candidate_commit.len() != 40
            || !config
                .candidate_commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("receipt candidate must be a full lowercase Git commit SHA");
        }
        let executable = std::env::current_exe().context("receipt executable path")?;
        let binary = std::fs::read(executable).context("hash receipt executable")?;
        let binary_sha256 = hex::encode(hash::sha256(&binary));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&config.path)
            .with_context(|| format!("create new receipt file {}", config.path.display()))?;
        let mut recorder = Self {
            file: BufWriter::new(file),
            last_verified_height: None,
        };
        recorder.record("run_start", json!({
            "candidate_commit":config.candidate_commit,
            "binary_sha256":binary_sha256,
            "software_version":env!("CARGO_PKG_VERSION"),
            "chain_id":chain,
            "requested_stop_height":opts.stop_height,
            "process_proposal_checked":opts.verify_data,
            "witness_interval":opts.witness_interval,
            "reference_server_count":opts.rpc.len(),
            "scope":"local execution compared with authenticated network header commitments; not an independent signature proof"
        }))?;
        Ok(recorder)
    }

    fn record(&mut self, event: &str, data: Value) -> anyhow::Result<()> {
        let row = json!({"schema_version":1,"event":event,
            "time_utc":consensus::ticker::system_now().canonical_string(),"data":data});
        serde_json::to_writer(&mut self.file, &row)?;
        self.file.write_all(b"\n")?;
        // Readers can inspect complete lines during a run. Durability is
        // requested on the terminal record; an absent terminal stays incomplete.
        self.file.flush()?;
        Ok(())
    }

    pub(crate) fn baseline(&mut self, state: &State, first_height: i64) -> anyhow::Result<()> {
        self.record(
            "baseline",
            json!({
                "anchor_height":state.last_block_height,
                "anchor_block_hash":hex::encode(&state.last_block_id.hash),
                "anchor_app_hash":hex::encode(&state.app_hash),
                "first_execution_height":first_height
            }),
        )
    }

    /// Called only after comparing and authenticating the next header.
    pub(crate) fn verified(
        &mut self,
        state: &State,
        header: &Header,
        server: usize,
        tx_count: Option<usize>,
        store: &StateStore,
    ) -> anyhow::Result<()> {
        if state.last_block_height == 0 {
            return Ok(());
        }
        let height = state.last_block_height;
        if header.height != height + 1
            || state.app_hash != header.app_hash
            || state.last_results_hash != header.last_results_hash
        {
            bail!("receipt state/header commitment mismatch at {height}");
        }
        let header_hash = header
            .hash()
            .ok_or_else(|| anyhow!("receipt header {height} has no hash"))?;
        if let Some(count) = tx_count {
            let response = store
                .load_finalize_block_response(height)
                .map_err(|e| anyhow!("receipt results at {height}: {e}"))?
                .ok_or_else(|| anyhow!("receipt results unavailable at {height}"))?;
            if response.tx_results.len() != count
                || ctypes::results_hash(&response.tx_results) != state.last_results_hash
            {
                bail!("receipt transaction-result commitment mismatch at {height}");
            }
            let results: Vec<_> = response
                .tx_results
                .iter()
                .map(|result| {
                    json!({
                        "code":result.code,
                        "data":base64::engine::general_purpose::STANDARD.encode(&result.data),
                        "gas_wanted":result.gas_wanted,
                        "gas_used":result.gas_used
                    })
                })
                .collect();
            self.record(
                "height",
                json!({
                    "executed_height":height,
                    "local_block_hash":hex::encode(&state.last_block_id.hash),
                    "local_app_hash":hex::encode(&state.app_hash),
                    "reference_app_hash":hex::encode(&header.app_hash),
                    "local_results_hash":hex::encode(&state.last_results_hash),
                    "reference_results_hash":hex::encode(&header.last_results_hash),
                    "reference_header_height":header.height,
                    "reference_header_hash":hex::encode(&header_hash),
                    "reference_server_index":server,
                    "next_header_app_version":header.version.app,
                    "tx_count":count,"tx_results":results
                }),
            )?;
        } else {
            // A restored/resumed anchor is not execution by this invocation.
            self.record(
                "anchor_verified",
                json!({"height":height,
                "reference_header_height":header.height,
                "reference_header_hash":hex::encode(&header_hash)}),
            )?;
        }
        self.last_verified_height = Some(height);
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        totals: &FollowStats,
        stop_height: i64,
        interrupted: bool,
        failed: bool,
    ) -> anyhow::Result<()> {
        let status = if interrupted {
            "interrupted"
        } else if failed {
            "failed"
        } else if stop_height > 0
            && totals.last >= stop_height
            && self
                .last_verified_height
                .is_some_and(|height| height >= totals.last)
        {
            "complete"
        } else {
            "incomplete"
        };
        self.record(
            "run_end",
            json!({"status":status,"execution_error":failed,
            "first_execution_height":totals.first,"last_executed_height":totals.last,
            "last_verified_height":self.last_verified_height,
            "executed_blocks":totals.blocks,"executed_transactions":totals.txs}),
        )?;
        self.file.get_ref().sync_all()?;
        Ok(())
    }
}
