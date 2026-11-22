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
    fibre: std::sync::Arc<celestia_fibre::FibreClient>,
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
    let fibre = {
        let endpoint = celestia_fibre::Endpoint::new(url).timeout(Duration::from_secs(10));
        let endpoint = match &token {
            Some(value) => endpoint.metadata("x-token", value.to_str()?),
            None => endpoint,
        };
        std::sync::Arc::new(celestia_fibre::FibreClient::from_endpoint(
            endpoint,
            celestia_fibre::FibreClientConfig::new(chain)?,
        )?)
    };
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
    let mut grpc = Grpc::new(core.channel);
    grpc.ready().await.context("core endpoint unavailable")?;
    let mut req = tonic::Request::new(AbciQueryRequest {
        data: key.clone(),
        path: "store/bank/key".into(),
        height,
        prove: true,
    });
    if let Some(token) = core.token {
        req.metadata_mut().insert("x-token", token);
    }
    let result = grpc
        .unary(
            req,
            PathAndQuery::from_static("/cosmos.base.tendermint.v1beta1.Service/ABCIQuery"),
            ProstCodec::default(),
        )
        .await?
        .into_inner();
    let amount = app::verify_bank_balance_response(&key, height, &header.app_hash, result)
        .map_err(anyhow::Error::msg)?;
    Ok(json!({"denom":"utia","amount":amount}))
}

#[cfg(feature = "da-fibre")]
pub(super) async fn download(core: Core, encoded: &str) -> Result<Value> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let id = celestia_fibre::BlobID::from_bytes(&STANDARD.decode(encoded)?)?;
    let blob = tokio::time::timeout(
        Duration::from_secs(90),
        core.fibre
            .download(&id, celestia_fibre::DownloadOptions::default()),
    )
    .await??;
    Ok(json!({"data":STANDARD.encode(blob.data())}))
}
