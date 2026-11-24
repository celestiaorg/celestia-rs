//! Upload a signed payment promise and settle it through the existing wallet.
use super::{Options, Wallet};
use crate::bridge::fibre::Client;
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use celestia_proto::{Any, celestia::fibre::v1::MsgPayForFibre};
use fibre_prost::Message as _;
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
    ) -> Result<Value> {
        let args = Upload::parse(
            params
                .as_array()
                .context("expected Fibre upload parameters")?,
        )?;
        self.check_options(&args.options)?;
        let permit = self
            .sequence
            .clone()
            .try_acquire_owned()
            .context("bridge signer busy")?;
        let (message, mut result, options) =
            args.prepare(client, &self.signer, &self.address()).await?;
        let wallet = self.clone();
        let settle = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            wallet.broadcast(message, None, None, &options, &wallet.states)
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

pub(in crate::bridge) struct Upload<'a> {
    namespace: &'a str,
    payload: &'a str,
    pub(in crate::bridge) options: Options,
}

impl<'a> Upload<'a> {
    pub(in crate::bridge) fn parse(args: &'a [Value]) -> Result<Self> {
        let [Value::String(namespace), Value::String(payload), opts] = args else {
            anyhow::bail!("expected namespace, data and transaction options");
        };
        let options = serde_json::from_value::<Option<Options>>(opts.clone())?.unwrap_or_default();
        Ok(Self {
            namespace,
            payload,
            options,
        })
    }
    pub(in crate::bridge) async fn prepare(
        self,
        client: &Client,
        signer: &crate::client::Signer,
        address: &str,
    ) -> Result<(Any, Value, Options)> {
        let namespace = Namespace::from_raw(&STANDARD.decode(self.namespace)?)?;
        let data = STANDARD.decode(self.payload)?;
        let key = k256::ecdsa::SigningKey::from_slice(
            signer
                .key()
                .and_then(|k| k.secret.as_ref())
                .context("signer has no private key")?,
        )?;
        let msg = tokio::time::timeout(
            Duration::from_secs(90),
            client
                .inner
                .upload_and_prepare(&key, namespace, &data, &address.parse()?),
        )
        .await??;
        let msg = MsgPayForFibre::decode(msg.encode_to_vec().as_slice())?;
        let promise = msg
            .payment_promise
            .as_ref()
            .context("missing payment promise")?;
        let version = u8::try_from(promise.blob_version)?;
        let id = [[version].as_slice(), promise.commitment.as_slice()].concat();
        let result = json!({
            "blob_id": STANDARD.encode(id),
            "validator_signatures": msg.validator_signatures.iter().map(|s| STANDARD.encode(s)).collect::<Vec<_>>(),
            "payment_promise": {
                "chain_id":promise.chain_id, "namespace":STANDARD.encode(&promise.namespace),
                "blob_size":promise.blob_size, "commitment":hex::encode(&promise.commitment),
                "row_version":version, "valset_height":promise.height,
                "creation_timestamp":promise.creation_timestamp, "signature":STANDARD.encode(&promise.signature)
            }
        });
        Ok((Any::pack(&msg), result, self.options))
    }
}
