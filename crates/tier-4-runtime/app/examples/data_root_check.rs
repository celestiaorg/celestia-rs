//! Checks committed mainnet blocks the way catch-up (block sync) does:
//! rebuild the data square from the block's txs with the builder of its
//! app version (`app::proposal::verified_block_eds`) and require the
//! header's data hash, and the block's `square_size`.
//!
//! Input (stdin): one JSON object per line, as
//! `conformance/mainnet-rpc/data-root/campaign.py fetch` writes them:
//! `{"height", "app_version", "data_hash" (hex), "square_size", "txs"
//! (base64)}`. Output (stdout): one JSON object per block with the result
//! and the block's shape (tx and blob counts, share versions, largest tx),
//! which the campaign uses to report its hard cases.
//!
//! ```sh
//! python3 conformance/mainnet-rpc/data-root/campaign.py fetch heights.txt \
//!   | cargo run -q --release -p app --example data_root_check
//! ```

use std::io::{BufRead, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

/// The bytes of a compact share's data region; a tx longer than this
/// spans more than one share.
const SHARE_DATA: usize = 478;

fn check(block: &Value) -> Result<Value, String> {
    let height = block["height"].as_u64().ok_or("no height")?;
    let app_version = block["app_version"].as_u64().ok_or("no app_version")?;
    let data_hash = hex::decode(block["data_hash"].as_str().ok_or("no data_hash")?)
        .map_err(|e| e.to_string())?;
    let square_size = block["square_size"].as_u64().ok_or("no square_size")?;
    let txs = block["txs"]
        .as_array()
        .ok_or("no txs")?
        .iter()
        .map(|t| STANDARD.decode(t.as_str().unwrap_or_default()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let (mut blob_txs, mut blobs, mut share_v1, mut largest_tx, mut long_txs) = (0, 0, 0, 0, 0);
    for tx in &txs {
        let inner = match txdecode::unmarshal_blob_tx_v9(tx) {
            Some(Ok(b)) => {
                blob_txs += 1;
                blobs += b.blobs.len();
                share_v1 += b.blobs.iter().filter(|b| b.share_version == 1).count();
                b.tx.len()
            }
            _ => tx.len(),
        };
        largest_tx = largest_tx.max(inner);
        if inner > SHARE_DATA {
            long_txs += 1;
        }
    }
    let shape = json!({
        "txs": txs.len(),
        "blob_txs": blob_txs,
        "blobs": blobs,
        "share_v1_blobs": share_v1,
        "largest_tx": largest_tx,
        "long_txs": long_txs,
        "square_size": square_size,
    });
    let result =
        app::proposal::verified_block_eds(app_version, &txs, &data_hash).and_then(|(eds, _)| {
            let width = eds.width() as u64 / 2;
            if width == square_size {
                Ok(())
            } else {
                Err(format!(
                    "data hash matches but square size {width} != committed {square_size}"
                ))
            }
        });
    let stateless = if app_version >= 9 {
        stateless_process_proposal(app_version, &txs).err()
    } else {
        None
    };
    Ok(json!({
        "height": height,
        "app_version": app_version,
        "ok": result.is_ok(),
        "error": result.err(),
        "stateless_process_proposal": stateless,
        "shape": shape,
    }))
}

/// The stateless checks of v9 and later ProcessProposal on each tx: its
/// size, the BlobTx envelope (go-square v4 rc4 at v9, the canonical v10
/// decoder after), the SDK decoding (the v9 registry at v9), no `MsgPayForBlobs` outside a BlobTx,
/// and `ValidateBlobTx` with share commitments. Message caps, the ante
/// handler and execution need the chain's state and are not run.
fn stateless_process_proposal(app_version: u64, txs: &[Vec<u8>]) -> Result<(), String> {
    const PFB: &str = "/celestia.blob.v1.MsgPayForBlobs";
    // v9 ProcessProposal (`process_v9`) decodes with the v9 registry.
    let decode = |raw: &[u8]| {
        if app_version == 9 {
            txdecode::decode_v9(raw)
        } else {
            txdecode::decode(raw)
        }
    };
    for (i, raw) in txs.iter().enumerate() {
        if raw.len() > app::proposal::MAX_TX_SIZE {
            return Err(format!("tx {i}: {} bytes", raw.len()));
        }
        let blob = if app_version == 9 {
            txdecode::unmarshal_blob_tx_v9(raw)
        } else {
            txdecode::unmarshal_blob_tx(raw)
        };
        match blob {
            Some(Err(e)) => return Err(format!("tx {i}: blob tx: {e:?}")),
            Some(Ok(u)) => {
                let tx = decode(&u.tx).map_err(|e| format!("tx {i}: {}", e.log))?;
                app::proposal::validate_blob_tx(&tx, &u)
                    .map_err(|e| format!("tx {i}: ValidateBlobTx: {}", e.log))?;
            }
            None => {
                let tx = decode(raw).map_err(|e| format!("tx {i}: {}", e.log))?;
                if tx.msgs().iter().any(|m| m.type_url == PFB) {
                    return Err(format!("tx {i}: MsgPayForBlobs outside a BlobTx"));
                }
            }
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<Value>(&line)
            .map_err(|e| e.to_string())
            .and_then(|b| check(&b).map_err(|e| format!("bad input: {e}")));
        let row = row.unwrap_or_else(|e| json!({"ok": false, "error": e}));
        writeln!(out, "{row}")?;
        out.flush()?;
    }
    Ok(())
}
