//! Upload a signed payment promise and settle it through the existing wallet.
use super::{Options, Wallet};
use crate::bridge::fibre::Client;
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use celestia_proto::{Any, celestia::fibre::v1::MsgPayForFibre};
use fibre_types::nmt::Namespace;
use prost::Message;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

impl Wallet {
    pub(in crate::bridge) async fn submit_fibre(
        self: &Arc<Self>,
        client: &Client,
        params: &Value,
        wait: bool,
        states: nodestore::StateStore,
    ) -> Result<Value> {
        let [Value::String(ns), Value::String(payload), opts] = params
            .as_array()
            .context("expected Fibre upload parameters")?
            .as_slice()
        else {
            anyhow::bail!("expected namespace, data and transaction options");
        };
        let options = serde_json::from_value::<Option<Options>>(opts.clone())?.unwrap_or_default();
        self.check_options(&options)?;
        let permit = self
            .sequence
            .clone()
            .try_acquire_owned()
            .context("bridge signer busy")?;
        let namespace = Namespace::from_raw(&STANDARD.decode(ns)?)?;
        let data = STANDARD.decode(payload)?;
        let secret = self
            .signer
            .key()
            .and_then(|k| k.secret.as_ref())
            .context("signer has no private key")?;
        let key = k256::ecdsa::SigningKey::from_slice(secret)?;
        let msg = tokio::time::timeout(
            Duration::from_secs(90),
            client
                .inner
                .upload_and_prepare(&key, namespace, &data, &self.address().parse()?),
        )
        .await??;
        let msg = MsgPayForFibre::decode(msg.encode_to_vec().as_slice())?;
        let promise = msg
            .payment_promise
            .as_ref()
            .context("missing payment promise")?;
        let version = u8::try_from(promise.blob_version)?;
        let id = [[version].as_slice(), promise.commitment.as_slice()].concat();
        let mut result = json!({
            "blob_id": STANDARD.encode(id),
            "validator_signatures": msg.validator_signatures.iter().map(|s| STANDARD.encode(s)).collect::<Vec<_>>(),
            "payment_promise": {
                "chain_id":promise.chain_id, "namespace":STANDARD.encode(&promise.namespace),
                "blob_size":promise.blob_size, "commitment":hex::encode(&promise.commitment),
                "row_version":version, "valset_height":promise.height,
                "creation_timestamp":promise.creation_timestamp, "signature":STANDARD.encode(&promise.signature)
            }
        });
        let wallet = self.clone();
        let settle = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            wallet.broadcast(Any::pack(&msg), None, None, &options, &states)
        });
        if wait {
            let receipt = settle.await??;
            result["height"] = json!(receipt.height);
            result["tx_hash"] = json!(receipt.hash);
        } else {
            tokio::spawn(async move {
                match settle.await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => tracing::error!(%error, "Fibre settlement failed"),
                    Err(error) => tracing::error!(%error, "Fibre settlement worker stopped"),
                }
            });
        }
        Ok(result)
    }
}
