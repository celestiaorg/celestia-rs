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
        states: nodestore::StateStore,
    ) -> Result<u64> {
        // Keep the sequence reserved until inclusion even when the HTTP caller disconnects.
        let permit = self
            .sequence
            .clone()
            .try_acquire_owned()
            .context("bridge signer busy")?;
        let params = params
            .as_array()
            .filter(|p| p.len() == 2)
            .context("expected blobs and transaction options")?;
        let blobs = params[0]
            .as_array()
            .filter(|v| !v.is_empty())
            .context("expected a nonempty blob array")?
            .iter()
            .map(parse_blob)
            .collect::<Result<Vec<_>>>()?;
        let options =
            serde_json::from_value::<Option<Options>>(params[1].clone())?.unwrap_or_default();
        let wallet = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            wallet.broadcast(blobs, &options, &states)
        })
        .await?
    }

    fn broadcast(
        &self,
        blobs: Vec<shares::Blob>,
        options: &Options,
        states: &nodestore::StateStore,
    ) -> Result<u64> {
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
        let msg = txs::msg_pay_for_blobs(&address, &blobs)?;
        let mut flags = self.flags.clone();
        let price = options.price(&self.app)?;
        flags.fee.gas = if options.gas == 0 {
            txs::pfb_gas(&msg)
        } else {
            options.gas
        };
        #[allow(clippy::cast_precision_loss)]
        let fee = (price * flags.fee.gas as f64).ceil();
        ensure!(
            fee.is_finite() && fee >= 0.0 && fee < 2_f64.powi(63),
            "fee outside int64 range"
        );
        flags.fee.fees = format!("{fee:.0}utia");
        flags.fee.granter.clone_from(&options.fee_granter_address);
        let res = client::send_with_signer(&self.home, &flags, &self.signer, false, |_| {
            Ok((vec![Any::pack(&msg)], Some(blobs)))
        })?;
        let hash = res["txhash"].as_str().context("missing transaction hash")?;
        committed_height(&txs::Node::new(&flags.node), states, hash)
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
        let priority = match self.tx_priority {
            1..=3 => self.tx_priority,
            _ => 0,
        };
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
        let max_price = if self.max_gas_price == 0.0 {
            100.0 * txs::DEFAULT_MIN_GAS_PRICE
        } else {
            self.max_gas_price
        };
        ensure!(
            price <= max_price,
            "estimated gas price exceeds max gas price in tx config"
        );
        Ok(price)
    }
}

fn parse_blob(value: &Value) -> Result<shares::Blob> {
    let bytes = |v: &Value| -> Result<Vec<u8>> {
        Ok(STANDARD.decode(v.as_str().context("expected base64 encoded bytes")?)?)
    };
    let namespace = shares::Namespace::from_slice(&bytes(&value["namespace"])?)?;
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
