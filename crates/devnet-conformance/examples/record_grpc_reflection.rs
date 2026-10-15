//! Records Go's `cosmos.base.reflection.v2alpha1.ReflectionService`
//! answers (served by the gRPC server only, not over ABCI).
//!
//! usage: `record_grpc_reflection <grpc-addr> <out.json>`
#![allow(clippy::print_stderr)]

use api_server::RawCodec;

const METHODS: &[&str] = &[
    "GetAuthnDescriptor",
    "GetChainDescriptor",
    "GetCodecDescriptor",
    "GetConfigurationDescriptor",
    "GetQueryServicesDescriptor",
    "GetTxDescriptor",
];

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, addr, out] = args.as_slice() else {
        anyhow::bail!("usage: record_grpc_reflection <grpc-addr> <out.json>");
    };
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
        .connect()
        .await?;
    let mut client = tonic::client::Grpc::new(channel);
    let mut cases = serde_json::Map::new();
    for m in METHODS {
        client.ready().await?;
        let path: http::uri::PathAndQuery =
            format!("/cosmos.base.reflection.v2alpha1.ReflectionService/{m}").parse()?;
        let r = client
            .unary(tonic::Request::new(Vec::new()), path, RawCodec)
            .await?;
        cases.insert((*m).to_owned(), hex::encode(r.get_ref()).into());
    }
    std::fs::write(out, serde_json::to_string_pretty(&cases)? + "\n")?;
    eprintln!("wrote {out}");
    Ok(())
}
