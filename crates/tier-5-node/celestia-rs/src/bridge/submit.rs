//! Submit paid blobs through the local consensus RPC with an operator-selected account.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use celestia_proto::{Any, celestia::core::v1::gas_estimation as gas};
use prost::Message as _;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::{client, keys::ACC_PREFIX, txs};

pub(super) const BODY_LIMIT: usize = 2 * app::proposal::MAX_TX_SIZE;

pub(super) struct Receipt {
    pub(super) height: u64,
    pub(super) hash: String,
}

pub(crate) struct Wallet {
    app: app::AbciApp,
    home: PathBuf,
    flags: client::TxFlags,
    signer: client::Signer,
    sequence: Arc<Semaphore>,
}

impl Wallet {
    pub(crate) fn open(
        home: &Path,
        name: &str,
        listen: &str,
        app: app::AbciApp,
        chain_id: &str,
    ) -> Result<Arc<Self>> {
        let mut endpoint = crate::rpc_service::socket_addr(listen)?;
        if endpoint.ip().is_unspecified() {
            endpoint.set_ip(match endpoint.ip() {
                IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
                IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
            });
        }
        let flags = client::TxFlags {
            chain_id: chain_id.to_owned(),
            from: name.to_owned(),
            node: format!("tcp://{endpoint}"),
            ..Default::default()
        };
        let signer = client::resolve_from(home, &flags).context("load bridge signing account")?;
        Ok(Arc::new(Self {
            app,
            home: home.to_owned(),
            flags,
            signer,
            sequence: Arc::new(Semaphore::new(1)),
        }))
    }

    pub(super) fn address(&self) -> String {
        self.signer.bech32(ACC_PREFIX)
    }

    pub(super) async fn submit(
        self: &Arc<Self>,
        params: &Value,
        method: &str,
        states: nodestore::StateStore,
    ) -> Result<Receipt> {
        // Keep the sequence reserved until inclusion even when the HTTP caller disconnects.
        let permit = self
            .sequence
            .clone()
            .try_acquire_owned()
            .context("bridge signer busy")?;
        let params = params
            .as_array()
            .context("expected transaction parameters")?;
        let (options, args) = params
            .split_last()
            .context("expected transaction options")?;
        let options =
            serde_json::from_value::<Option<Options>>(options.clone())?.unwrap_or_default();
        let (message, blobs, blob_gas) = match (method, args) {
            (m, args) if super::state_rpc::writes(m) => (
                super::state_rpc::message(m, args, self.address())?,
                None,
                None,
            ),
            ("blob.Submit" | "state.SubmitPayForBlob", [values]) => {
                let blobs = values
                    .as_array()
                    .filter(|v| !v.is_empty())
                    .context("expected a nonempty blob array")?
                    .iter()
                    .map(|v| parse_blob(v, method == "state.SubmitPayForBlob"))
                    .collect::<Result<Vec<_>>>()?;
                let msg = txs::msg_pay_for_blobs(&self.address(), &blobs)?;
                let gas = txs::pfb_gas(&msg);
                (Any::pack(&msg), Some(blobs), Some(gas))
            }
            _ => anyhow::bail!("invalid transaction parameters"),
        };
        let wallet = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            wallet.broadcast(message, blobs, blob_gas, &options, &states)
        })
        .await?
    }

    fn broadcast(
        &self,
        message: Any,
        blobs: Option<Vec<shares::Blob>>,
        blob_gas: Option<u64>,
        options: &Options,
        states: &nodestore::StateStore,
    ) -> Result<Receipt> {
        let address = self.address();
        let requested = if options.signer_address.is_empty() {
            options.key_name.is_empty() || options.key_name == self.flags.from
        } else {
            options.signer_address == address
        };
        ensure!(
            requested,
            "requested account is not configured for bridge signing"
        );
        ensure!(
            options.fee_granter_address.is_empty()
                || client::acc_address_from_bech32(&options.fee_granter_address).is_some(),
            "invalid fee granter address"
        );
        let mut flags = self.flags.clone();
        let (price, gas) = if let Some(default_gas) = blob_gas {
            (
                options.price(&self.app)?,
                if options.gas == 0 {
                    default_gas
                } else {
                    options.gas
                },
            )
        } else {
            self.message_fee(&message, options)?
        };
        flags.fee.gas = gas;
        #[allow(clippy::cast_precision_loss)]
        let fee = (price * flags.fee.gas as f64).ceil();
        ensure!(
            fee.is_finite() && fee >= 0.0 && fee < 2_f64.powi(63),
            "fee outside int64 range"
        );
        flags.fee.fees = format!("{fee:.0}utia");
        flags.fee.granter.clone_from(&options.fee_granter_address);
        let res = client::send_with_signer(&self.home, &flags, &self.signer, false, |_| {
            Ok((vec![message], blobs))
        })?;
        let hash = res["txhash"].as_str().context("missing transaction hash")?;
        let height = committed_height(&txs::Node::new(&flags.node), states, hash)?;
        Ok(Receipt {
            height,
            hash: hash.to_owned(),
        })
    }
    fn message_fee(&self, message: &Any, opts: &Options) -> Result<(f64, u64)> {
        let explicit = opts.is_gas_price_set && opts.gas_price.to_bits() != (-1.0_f64).to_bits();
        if explicit && opts.gas != 0 {
            return Ok((opts.gas_price, opts.gas));
        }
        let (account_number, sequence) =
            txs::Node::new(&self.flags.node).account(&self.address())?;
        let estimate = txs::UnsignedTx {
            msgs: vec![message.clone()],
            memo: String::new(),
            timeout_height: 0,
            fee: txs::FeeOptions {
                fees: "1utia".into(),
                ..Default::default()
            }
            .fee_with_gas(0)?,
            chain_id: self.flags.chain_id.clone(),
            account_number,
            sequence,
        };
        let req = gas::EstimateGasPriceAndUsageRequest {
            tx_priority: opts.priority(),
            tx_bytes: txs::sign(
                &estimate,
                self.signer.key().context("missing configured signer")?,
            )?
            .raw,
        };
        let (bytes, _) = app::node_services::grpc_query(
            &self.app,
            "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPriceAndUsage",
            &req.encode_to_vec(),
            0,
        )
        .context("gas estimator unavailable")?
        .map_err(|e| anyhow::anyhow!("estimate gas usage: {e:?}"))?;
        let est = gas::EstimateGasPriceAndUsageResponse::decode(bytes.as_slice())?;
        let price = if explicit {
            opts.gas_price
        } else {
            est.estimated_gas_price
        };
        opts.check_price(price)?;
        Ok((
            price,
            if opts.gas != 0 {
                opts.gas
            } else {
                est.estimated_gas_used
            },
        ))
    }
}

// The pool may have removed the proposal before its commit receipt and DA height are published.
fn committed_height(node: &txs::Node, states: &nodestore::StateStore, hash: &str) -> Result<u64> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let receipt = node.wait_tx(hash, std::time::Duration::from_secs(120))?;
    ensure!(
        receipt["execution_code"].as_u64() == Some(0),
        "tx {hash} failed: {}",
        receipt["error"]
    );
    let height: i64 = receipt["height"]
        .as_str()
        .context("missing commit height")?
        .parse()?;
    ensure!(height > 0, "missing commit height");
    loop {
        if states
            .load()?
            .is_some_and(|s| s.last_block_height >= height)
        {
            return Ok(u64::try_from(height)?);
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "timed out waiting for tx {hash}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Options {
    gas_price: f64,
    is_gas_price_set: bool,
    max_gas_price: f64,
    tx_priority: i32,
    gas: u64,
    key_name: String,
    signer_address: String,
    fee_granter_address: String,
}

impl Options {
    fn price(&self, app: &app::AbciApp) -> Result<f64> {
        if self.is_gas_price_set && self.gas_price.to_bits() != (-1.0_f64).to_bits() {
            ensure!(self.gas_price >= 0.0, "invalid gas price");
            return Ok(self.gas_price);
        }
        let priority = self.priority();
        let req = gas::EstimateGasPriceRequest {
            tx_priority: priority,
        };
        let (bytes, _) = app::node_services::grpc_query(
            app,
            "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPrice",
            &req.encode_to_vec(),
            0,
        )
        .context("gas estimator unavailable")?
        .map_err(|e| anyhow::anyhow!("estimate gas price: {e:?}"))?;
        let price = gas::EstimateGasPriceResponse::decode(bytes.as_slice())?.estimated_gas_price;
        self.check_price(price)?;
        Ok(price)
    }

    fn priority(&self) -> i32 {
        match self.tx_priority {
            1..=3 => self.tx_priority,
            _ => 0,
        }
    }

    fn check_price(&self, price: f64) -> Result<()> {
        let max_price = if self.max_gas_price == 0.0 {
            100.0 * txs::DEFAULT_MIN_GAS_PRICE
        } else {
            self.max_gas_price
        };
        ensure!(
            price <= max_price,
            "estimated gas price exceeds max gas price in tx config"
        );
        Ok(())
    }
}

fn parse_blob(value: &Value, core_blob: bool) -> Result<shares::Blob> {
    let bytes = |v: &Value| -> Result<Vec<u8>> {
        Ok(STANDARD.decode(v.as_str().context("expected base64 encoded bytes")?)?)
    };
    let namespace = if core_blob {
        let version = value
            .get("namespace_version")
            .map_or(Some(0), Value::as_u64)
            .context("invalid namespace version")?;
        let id = bytes(&value["namespace_id"])?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid namespace ID length"))?;
        shares::Namespace::new(u8::try_from(version)?, id)?
    } else {
        shares::Namespace::from_slice(&bytes(&value["namespace"])?)?
    };
    ensure!(!namespace.is_reserved(), "reserved namespace");
    let version = value
        .get("share_version")
        .map_or(Some(0), Value::as_u64)
        .context("invalid share version")?;
    ensure!(
        version <= 1,
        "blob.Submit supports share versions zero and one"
    );
    let signer = value
        .get("signer")
        .filter(|v| !v.is_null())
        .map(|v| {
            bytes(v)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid signer length"))
        })
        .transpose()?;
    Ok(shares::Blob::new_with_signer(
        namespace,
        bytes(&value["data"])?,
        u8::try_from(version)?,
        signer,
    )?)
}
