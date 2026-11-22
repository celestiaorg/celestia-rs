//! Optional state queries anchored to a locally sampled extended header.
use super::HeaderWire;
use anyhow::{Context, Result, ensure};
use axum::http::uri::PathAndQuery;
use celestia_proto::cosmos::base::tendermint::v1beta1::AbciQueryRequest;
use kvdb::{KvBackend as _, RedbBackend};
use prost::Message as _;
use serde_json::{Value, json};
use std::time::Duration;
use tonic::{
    client::Grpc,
    codec::ProstCodec,
    transport::{Channel, Endpoint},
};

#[derive(Clone)]
pub(super) struct Core {
    #[cfg(feature = "da-fibre")]
    pub(super) fibre: crate::bridge::fibre::Client,
    channel: Channel,
    token: Option<tonic::metadata::AsciiMetadataValue>,
}

fn token(dir: &std::path::Path) -> Result<tonic::metadata::AsciiMetadataValue> {
    for name in ["xtoken.json", "x-token.json"] {
        let raw = match std::fs::read(dir.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            result => result?,
        };
        let file: std::collections::HashMap<String, String> = serde_json::from_slice(&raw)?;
        let value = ["x-token", "xtoken", "token"]
            .iter()
            .find_map(|key| file.get(*key).filter(|s| !s.is_empty()))
            .context("missing core authentication token")?;
        let mut metadata = tonic::metadata::AsciiMetadataValue::try_from(value.as_str())
            .context("invalid core authentication metadata")?;
        metadata.set_sensitive(true);
        return Ok(metadata);
    }
    anyhow::bail!("core authentication token file not found")
}

pub(super) fn channel(
    #[cfg(feature = "da-fibre")] chain: &str,
    host: &str,
    port: u16,
    tls: bool,
    token_dir: Option<&std::path::Path>,
) -> Result<Core> {
    let host = if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        format!("[{ip}]")
    } else {
        ensure!(
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')),
            "invalid core hostname"
        );
        host.to_owned()
    };
    let scheme = if tls { "https" } else { "http" };
    let url = format!("{scheme}://{host}:{port}");
    let mut endpoint = Endpoint::from_shared(url.clone())?;
    if tls {
        endpoint =
            endpoint.tls_config(tonic::transport::ClientTlsConfig::new().with_native_roots())?;
    }
    let channel = endpoint
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .connect_lazy();
    let token = if tls {
        token_dir.map(token).transpose()?
    } else {
        None
    };
    #[cfg(feature = "da-fibre")]
    let fibre = crate::bridge::fibre::Client::new(
        url,
        chain,
        token.as_ref().map(|v| v.to_str()).transpose()?,
    )?;
    Ok(Core {
        #[cfg(feature = "da-fibre")]
        fibre,
        channel,
        token,
    })
}

pub(super) async fn balance(core: Core, db: &RedbBackend, address: &str) -> Result<Value> {
    let (prefix, bytes) =
        encoding::bech32::decode_and_convert(address).map_err(anyhow::Error::msg)?;
    ensure!(
        matches!(
            prefix.as_str(),
            crate::keys::ACC_PREFIX | crate::keys::VAL_PREFIX
        ) && (1..=255).contains(&bytes.len()),
        "invalid address"
    );
    let raw = db.get(b"head")?.context("missing sampled head")?;
    let header = HeaderWire::decode(raw.as_slice())?
        .header
        .context("missing sampled header")?;
    ensure!(header.height > 1, "no prior committed state");
    let key = app::utia_balance_key(&bytes);
    let height = header.height - 1;
    let result = core
        .unary(
            "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery",
            AbciQueryRequest {
                data: key.clone(),
                path: "store/bank/key".into(),
                height,
                prove: true,
            },
        )
        .await?;
    let amount = app::verify_bank_balance_response(&key, height, &header.app_hash, result)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({"denom":"utia","amount":amount}))
}

impl Core {
    pub(super) async fn unary<Req, Resp>(
        &self,
        path: &'static str,
        message: Req,
    ) -> Result<Resp, Box<tonic::Status>>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        let mut grpc = Grpc::new(self.channel.clone());
        grpc.ready()
            .await
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        let mut request = tonic::Request::new(message);
        if let Some(token) = &self.token {
            request.metadata_mut().insert("x-token", token.clone());
        }
        Ok(grpc
            .unary(
                request,
                PathAndQuery::from_static(path),
                ProstCodec::default(),
            )
            .await?
            .into_inner())
    }
}
