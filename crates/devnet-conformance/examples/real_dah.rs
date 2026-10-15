//! Spot-check of square construction on real blocks: for each CometBFT
//! `/block` JSON response given on the command line (e.g. fetched from a
//! mainnet or Mocha RPC), rebuilds the data square from the block's txs with
//! the app version's rules and compares square size and data root with the
//! header; every tx must decode with the app version's decoder.
//!
//! `real_dah <app_version> <block.json>...`
#![allow(clippy::print_stdout)]

use base64::Engine as _;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let app_version: u64 = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: real_dah <app_version> <block.json>..."))?
        .parse()?;
    let (mut ok, mut bad) = (0, 0);
    for path in args {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let block = &v["result"]["block"];
        let height = block["header"]["height"].as_str().unwrap_or("?");
        let want_root = block["header"]["data_hash"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let want_size = block["data"]["square_size"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok());
        let txs = block["data"]["txs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| base64::engine::general_purpose::STANDARD.decode(t))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        // ProcessProposal accepted the block, so every tx decodes.
        for (i, tx) in txs.iter().enumerate() {
            if let Err(e) = txdecode::decode_for(app_version, tx) {
                bad += 1;
                println!("height {height}: tx {i} does not decode: {}", e.log);
            }
        }
        match app::proposal::block_data_root(app_version, &txs) {
            Ok((size, root)) => {
                let root = hex::encode(root);
                let size_ok = want_size.is_none_or(|w| w == size);
                if root == want_root && size_ok {
                    ok += 1;
                    println!("height {height}: {} txs, square {size}: match", txs.len());
                } else {
                    bad += 1;
                    println!(
                        "height {height}: MISMATCH square {size} (header {want_size:?}) root {root} (header {want_root})"
                    );
                }
            }
            Err(e) => {
                bad += 1;
                println!("height {height}: error {e}");
            }
        }
    }
    println!("{ok} blocks match, {bad} differ");
    anyhow::ensure!(bad == 0, "{bad} blocks differ");
    Ok(())
}
