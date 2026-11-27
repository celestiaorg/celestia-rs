//! The shape of every 07-tendermint `MsgUpdateClient` in a stream of
//! blocks, to compare the code paths mainnet's relayers take with the
//! cases of the `-tmclient` oracle vectors.
//!
//! Input (stdin): blocks as `conformance/mainnet-rpc/data-root/campaign.py
//! fetch` writes them. Output (stdout): one JSON object per message: the
//! block height and app version, the client, the counterparty chain ID,
//! the header and trusted heights (and whether they are adjacent), the
//! sizes of the validator set, the trusted set and the commit, the absent
//! and nil signatures, and whether the set changed from the trusted one.
//!
//! ```sh
//! python3 conformance/mainnet-rpc/data-root/campaign.py fetch heights.txt \
//!   | cargo run -q --release -p app --example update_client_census
//! ```

use std::io::{BufRead, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use celestia_proto::ibc::core::client::v1::MsgUpdateClient;
use celestia_proto::ibc::lightclients::tendermint::v1::Header;
use prost::Message;
use serde_json::{Value, json};

const UPDATE: &str = "/ibc.core.client.v1.MsgUpdateClient";
const TM_HEADER: &str = "/ibc.lightclients.tendermint.v1.Header";

fn census(block: &Value, out: &mut impl Write) -> std::io::Result<()> {
    let height = block["height"].as_u64().unwrap_or_default();
    let app_version = block["app_version"].as_u64().unwrap_or_default();
    for raw in block["txs"].as_array().into_iter().flatten() {
        let Ok(bytes) = STANDARD.decode(raw.as_str().unwrap_or_default()) else {
            continue;
        };
        let Ok(tx) = txdecode::decode(&bytes) else {
            continue;
        };
        for msg in tx.msgs().iter().filter(|m| m.type_url == UPDATE) {
            let Ok(update) = MsgUpdateClient::decode(msg.value.as_slice()) else {
                continue;
            };
            let Some(any) = update.client_message else {
                continue;
            };
            if any.type_url != TM_HEADER {
                writeln!(
                    out,
                    "{}",
                    json!({"height": height, "app_version": app_version, "client": update.client_id, "client_message": any.type_url})
                )?;
                continue;
            }
            let Ok(h) = Header::decode(any.value.as_slice()) else {
                continue;
            };
            let sh = h.signed_header.unwrap_or_default();
            let header = sh.header.unwrap_or_default();
            let commit = sh.commit.unwrap_or_default();
            let trusted = h.trusted_height.unwrap_or_default();
            let vals = h.validator_set.unwrap_or_default();
            let tvals = h.trusted_validators.unwrap_or_default();
            let flags = |f: i32| {
                commit
                    .signatures
                    .iter()
                    .filter(|s| s.block_id_flag == f)
                    .count()
            };
            let addrs =
                |s: &[celestia_proto::celestia_core_proto::tendermint::types::Validator]| {
                    s.iter().map(|v| v.address.clone()).collect::<Vec<_>>()
                };
            writeln!(
                out,
                "{}",
                json!({
                    "height": height,
                    "app_version": app_version,
                    "client": update.client_id,
                    "chain_id": header.chain_id,
                    "header_height": header.height,
                    "trusted_revision": trusted.revision_number,
                    "trusted_height": trusted.revision_height,
                    "adjacent": u64::try_from(header.height).ok() == trusted.revision_height.checked_add(1),
                    "validators": vals.validators.len(),
                    "trusted_validators": tvals.validators.len(),
                    "commit_signatures": commit.signatures.len(),
                    "absent": flags(1),
                    "commit": flags(2),
                    "nil": flags(3),
                    "set_changed": addrs(&vals.validators) != addrs(&tvals.validators),
                })
            )?;
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if let Ok(block) = serde_json::from_str::<Value>(&line) {
            census(&block, &mut out)?;
            out.flush()?;
        }
    }
    Ok(())
}
