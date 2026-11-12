use abci_server::Application;
use celestia_proto::celestia_core_proto::abci::{request, response};
use std::{collections::HashSet, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = std::env::args().nth(1).ok_or("fixture")?;
    let repetitions: u32 = std::env::args().nth(2).ok_or("repetitions")?.parse()?;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/devnet")
        .join(&fixture)
        .join("abci.rec");
    let records = abci_server::record::parse(&std::fs::read(path)?)?;
    let adapter = app::AbciApp::new(app::App::new());
    let mut warmed = HashSet::new();
    for exchange in abci_server::record::exchanges(&records)? {
        let Some(req) = exchange.request.value else {
            continue;
        };
        if let request::Value::ProcessProposal(proposal) = &req {
            if proposal.txs.iter().any(|raw| warmed.contains(raw)) {
                let bytes: usize = proposal.txs.iter().map(Vec::len).sum();
                let started = Instant::now();
                for _ in 0..repetitions {
                    let result = adapter.handle(req.clone());
                    assert!(
                        matches!(result, response::Value::ProcessProposal(ref r) if r.status == 1)
                    );
                }
                println!(
                    "{}",
                    serde_json::json!({"fixture":fixture,"cache":std::env::var("CELESTIA_RS_BLOB_TX_CACHE").unwrap_or("on".into()),"repetitions":repetitions,"tx_bytes":bytes,"elapsed_seconds":started.elapsed().as_secs_f64()})
                );
                return Ok(());
            }
        }
        let candidate = if let request::Value::CheckTx(check) = &req {
            if check.r#type == 0 && txdecode::unmarshal_blob_tx_v9(&check.tx).is_some() {
                Some(check.tx.clone())
            } else {
                None
            }
        } else {
            None
        };
        let result = adapter.handle(req);
        if matches!(result, response::Value::CheckTx(ref r) if r.code == 0) {
            if let Some(raw) = candidate {
                warmed.insert(raw);
            }
        }
    }
    Err("no warmed proposal".into())
}
