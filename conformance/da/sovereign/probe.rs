// Compiled inside the pinned Sovereign SDK checkout to use its locked production adapter.
use anyhow::{ensure, Result};
use celestia_types::nmt::Namespace;
use sov_celestia_adapter::{
    verifier::{CelestiaVerifier, RollupParams},
    BlobReaderTrait, BlockHeaderTrait, CelestiaConfig, CelestiaService, DaService, DaVerifier,
};
use sov_shutdown::SecondaryShutdownController;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let params = RollupParams {
        rollup_batch_namespace: Namespace::const_v0(*b"sov-rs-bat"),
        rollup_proof_namespace: Namespace::const_v0(*b"sov-rs-prf"),
    };
    let config: CelestiaConfig = serde_json::from_value(serde_json::json!({
        "verify_on_fetch_mode": "return_error",
        "background_stat_polling_interval_secs": 0
    }))?;
    let shutdown = SecondaryShutdownController::new();
    let service = CelestiaService::new(config, params, &shutdown).await;
    let start = service.get_head_block_header().await?.height();
    let batch = b"paid Sovereign batch via the Rust consensus node".repeat(76);
    let proof = b"paid Sovereign proof via the Rust consensus node".to_vec();
    let batch_receipt = service.send_transaction(&batch).await.await??;
    let proof_receipt = service.send_proof(&proof).await.await??;
    let verifier = CelestiaVerifier::new(params);
    let mut found_batch = false;
    let mut found_proof = false;
    for height in start..=start + 120 {
        let block = service.get_block_at(height).await?;
        let mut blobs = service.extract_relevant_blobs(&block);
        for blob in &mut blobs.batch_blobs {
            if blob.hash == batch_receipt.blob_hash {
                blob.advance(blob.total_len());
                ensure!(
                    blob.verified_data() == batch
                        && Some(blob.sender()) == service.get_signer().await,
                    "batch mismatch"
                );
                found_batch = true;
            }
        }
        for blob in &mut blobs.proof_blobs {
            if blob.hash == proof_receipt.blob_hash {
                blob.advance(blob.total_len());
                ensure!(
                    blob.verified_data() == proof
                        && Some(blob.sender()) == service.get_signer().await,
                    "proof mismatch"
                );
                ensure!(
                    service.get_proofs_at(height).await?.contains(&proof),
                    "proof absent from blob.GetAll"
                );
                found_proof = true;
            }
        }
        let proofs = service.get_extraction_proof(&block, &blobs).await;
        verifier.verify_relevant_tx_list(block.header(), &blobs, proofs)?;
        if found_batch && found_proof {
            println!(
                "Sovereign SDK verified both paid namespaces between heights {start} and {height}"
            );
            return Ok(());
        }
    }
    anyhow::bail!("Sovereign SDK did not recover both paid namespaces")
}
