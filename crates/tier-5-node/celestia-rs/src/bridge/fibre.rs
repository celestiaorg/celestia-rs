//! Fibre provider discovery and commitment-verified reconstruction for both DA roles.
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use celestia_fibre::{BlobID, DownloadOptions, Endpoint, FibreClient, FibreClientConfig};
use rpc::json::J;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(super) struct Client {
    inner: Arc<FibreClient>,
    reads: Arc<Semaphore>,
}

impl Client {
    pub(super) fn new(url: String, chain: &str, token: Option<&str>) -> Result<Self> {
        let endpoint = Endpoint::new(url).timeout(Duration::from_secs(10));
        let endpoint = match token {
            Some(value) => endpoint.metadata("x-token", value),
            None => endpoint,
        };
        Ok(Self {
            inner: Arc::new(FibreClient::from_endpoint(
                endpoint,
                FibreClientConfig::new(chain)?,
            )?),
            reads: Arc::new(Semaphore::new(1)),
        })
    }

    pub(super) fn local(listen: &str, chain: &str) -> Result<Option<Self>> {
        if listen.is_empty() {
            return Ok(None);
        }
        let mut addr = crate::rpc_service::socket_addr(listen)?;
        if addr.ip().is_unspecified() {
            addr.set_ip(match addr.ip() {
                IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
                IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
            });
        }
        Self::new(format!("http://{addr}"), chain, None).map(Some)
    }

    pub(super) async fn download(&self, encoded: &str) -> Result<Value> {
        let id = BlobID::from_bytes(&STANDARD.decode(encoded)?)?;
        let _permit = self.reads.try_acquire().context("Fibre download busy")?;
        let blob = tokio::time::timeout(
            Duration::from_secs(90),
            self.inner.download(&id, DownloadOptions::default()),
        )
        .await??;
        Ok(json!({"data":STANDARD.encode(blob.data())}))
    }
}

pub(super) async fn rpc(client: Option<&Client>, params: &Value) -> Result<J, (i32, String)> {
    let Some([Value::String(id)]) = params.as_array().map(Vec::as_slice) else {
        return Err((-32602, "expected blob ID".into()));
    };
    let client = client.ok_or_else(|| {
        (
            1,
            "fibre module is not available: node is not connected to a core endpoint".to_owned(),
        )
    })?;
    client
        .download(id)
        .await
        .map(|v| J::Raw(v.to_string()))
        .map_err(|e| (1, e.to_string()))
}
