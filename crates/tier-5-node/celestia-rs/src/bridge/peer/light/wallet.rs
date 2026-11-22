//! Sign transaction messages locally and submit them to the configured core.
use super::core::Core;
use crate::{
    bridge::{state_rpc, submit},
    client,
    keys::ACC_PREFIX,
    txs,
};
use anyhow::{Context, Result, ensure};
use celestia_proto::{
    celestia::core::v1::gas_estimation as gas,
    cosmos::{auth::v1beta1 as auth, tx::v1beta1 as tx},
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

pub(super) fn handles(method: &str) -> bool {
    state_rpc::writes(method) || matches!(method, "blob.Submit" | "state.SubmitPayForBlob")
}

pub(super) struct Wallet {
    signer: client::Signer,
    pub(super) address: String,
    name: String,
    chain: String,
    sequence: Arc<Semaphore>,
}

impl Wallet {
    pub(super) fn open(
        home: &Path,
        name: &str,
        backend: Option<String>,
        chain: String,
    ) -> Result<Arc<Self>> {
        let flags = client::TxFlags {
            from: name.to_owned(),
            keyring_backend: backend,
            ..Default::default()
        };
        let signer = client::resolve_from(home, &flags)?;
        Ok(Arc::new(Self {
            address: signer.bech32(ACC_PREFIX),
            signer,
            name: name.to_owned(),
            chain,
            sequence: Arc::new(Semaphore::new(1)),
        }))
    }

    pub(super) async fn submit(
        self: &Arc<Self>,
        core: Core,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let request = submit::transaction(args, method, &self.address)?;
        request.options.validate(&self.name, &self.address)?;
        let permit = self
            .sequence
            .clone()
            .try_acquire_owned()
            .context("light signer busy")?;
        let wallet = self.clone();
        let (height, hash) = tokio::spawn(async move {
            let _permit = permit;
            wallet.broadcast(&core, request).await
        })
        .await??;
        Ok(match method {
            "blob.Submit" => json!(height),
            "fibre.Deposit" | "fibre.Withdraw" => Value::Null,
            _ => json!({"height":height,"txhash":hash,"logs":null,"events":null}),
        })
    }

    async fn broadcast(&self, core: &Core, req: submit::Transaction) -> Result<(i64, String)> {
        let response: auth::QueryAccountInfoResponse = core
            .unary(
                "/cosmos.auth.v1beta1.Query/AccountInfo",
                auth::QueryAccountInfoRequest {
                    address: self.address.clone(),
                },
            )
            .await?;
        let account = response.info.context("missing signer account")?;
        ensure!(
            account.address == self.address,
            "core returned a different signer account"
        );
        let key = self.signer.key().context("missing light signing key")?;
        let mut utx = txs::UnsignedTx {
            msgs: vec![req.message],
            memo: String::new(),
            timeout_height: 0,
            chain_id: self.chain.clone(),
            account_number: account.account_number,
            sequence: account.sequence,
            fee: txs::FeeOptions {
                fees: "1utia".into(),
                ..Default::default()
            }
            .fee_with_gas(0)?,
        };
        let opts = &req.options;
        let (price, gas) = fees(core, opts, req.blob_gas, &utx, key).await?;
        #[allow(clippy::cast_precision_loss)]
        let amount = (gas as f64 * price).ceil();
        ensure!(
            amount.is_finite() && (0.0..2_f64.powi(63)).contains(&amount),
            "fee outside int64 range"
        );
        utx.fee = txs::FeeOptions {
            fees: format!("{amount:.0}utia"),
            granter: opts.fee_granter_address.clone(),
            ..Default::default()
        }
        .fee_with_gas(gas)?;
        let signed = txs::sign(&utx, key)?;
        let hash = signed.hash();
        let bytes = match req.blobs {
            Some(blobs) => txs::blob_tx(&signed.raw, &blobs),
            None => signed.raw,
        };
        let resp: tx::BroadcastTxResponse = core
            .unary(
                "/cosmos.tx.v1beta1.Service/BroadcastTx",
                tx::BroadcastTxRequest {
                    tx_bytes: bytes,
                    mode: tx::BroadcastMode::Sync.into(),
                },
            )
            .await?;
        let check = resp
            .tx_response
            .context("missing core transaction response")?;
        ensure!(check.code == 0, "transaction rejected: {}", check.raw_log);
        ensure!(
            check.txhash.eq_ignore_ascii_case(&hash),
            "core returned a different transaction hash"
        );
        let height = tokio::time::timeout(Duration::from_secs(120), confirm(core, &hash)).await??;
        Ok((height, hash))
    }
}

async fn fees(
    core: &Core,
    opts: &submit::Options,
    blob_gas: Option<u64>,
    tx: &txs::UnsignedTx,
    key: &keyring::KeyRecord,
) -> Result<(f64, u64)> {
    let explicit = opts.is_gas_price_set && opts.gas_price.to_bits() != (-1.0_f64).to_bits();
    let gas = if opts.gas != 0 {
        Some(opts.gas)
    } else {
        blob_gas
    };
    if explicit && let Some(gas) = gas {
        ensure!(
            opts.gas_price.is_finite() && opts.gas_price >= 0.0,
            "invalid gas price"
        );
        return Ok((opts.gas_price, gas));
    }
    let (price, units) = if let Some(gas) = gas {
        let est: gas::EstimateGasPriceResponse = core
            .unary(
                "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPrice",
                gas::EstimateGasPriceRequest {
                    tx_priority: opts.priority(),
                },
            )
            .await?;
        (est.estimated_gas_price, gas)
    } else {
        let est: gas::EstimateGasPriceAndUsageResponse = core
            .unary(
                "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPriceAndUsage",
                gas::EstimateGasPriceAndUsageRequest {
                    tx_priority: opts.priority(),
                    tx_bytes: txs::sign(tx, key)?.raw,
                },
            )
            .await?;
        (
            if explicit {
                opts.gas_price
            } else {
                est.estimated_gas_price
            },
            est.estimated_gas_used,
        )
    };
    opts.check_price(price)?;
    Ok((price, units))
}

async fn confirm(core: &Core, hash: &str) -> Result<i64> {
    loop {
        let result: Result<tx::GetTxResponse, _> = core
            .unary(
                "/cosmos.tx.v1beta1.Service/GetTx",
                tx::GetTxRequest {
                    hash: hash.to_owned(),
                },
            )
            .await;
        match result {
            Ok(response) => {
                let tx = response
                    .tx_response
                    .context("missing committed transaction")?;
                ensure!(
                    tx.height > 0 && tx.txhash.eq_ignore_ascii_case(hash),
                    "invalid core transaction receipt"
                );
                ensure!(tx.code == 0, "transaction failed: {}", tx.raw_log);
                return Ok(tx.height);
            }
            Err(err) if err.code() == tonic::Code::NotFound => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(err) => return Err(err.into()),
        }
    }
}
