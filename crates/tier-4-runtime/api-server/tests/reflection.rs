//! Reflection is registered on the production SDK router, outside the query
//! interceptor; it does not parse heights or advertise the core-only version.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use grpc_reflection::proto;
use prost::Message;
use std::sync::Arc;

#[tokio::test]
async fn sdk_router_registers_only_the_sdk_reflection_version() {
    let app = Arc::new(app::AbciApp::new(app::App::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(api_server::serve_grpc_on(listener, app));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = tonic::client::Grpc::new(channel);
    let request = proto::Request {
        host: "provider.example".into(),
        operation: Some(proto::request::Operation::ListServices(String::new())),
    }
    .encode_to_vec();
    client.ready().await.unwrap();
    let mut input = tonic::Request::new(futures_util::stream::iter([request.clone()]));
    input.metadata_mut().insert(
        api_server::BLOCK_HEIGHT_HEADER,
        "not-a-height".parse().unwrap(),
    );
    let response = client
        .streaming(
            input,
            grpc_reflection::V1ALPHA.parse().unwrap(),
            api_server::RawCodec,
        )
        .await
        .unwrap();
    assert!(
        response
            .metadata()
            .get(api_server::BLOCK_HEIGHT_HEADER)
            .is_none()
    );
    let mut stream = response.into_inner();
    let raw = stream.message().await.unwrap().unwrap();
    let output = proto::Response::decode(raw.as_slice()).unwrap();
    assert_eq!(output.valid_host, "provider.example");
    let Some(proto::response::Result::ListServicesResponse(services)) = output.result else {
        panic!("missing service list")
    };
    assert!(
        services
            .service
            .iter()
            .any(|s| s.name == "tendermint.rpc.grpc.BlockAPI")
    );
    assert!(
        !services
            .service
            .iter()
            .any(|s| s.name == "tendermint.rpc.grpc.BroadcastAPI")
    );
    assert!(
        !services
            .service
            .iter()
            .any(|s| s.name == "grpc.reflection.v1.ServerReflection")
    );
    assert!(stream.message().await.unwrap().is_none());
    client.ready().await.unwrap();
    let result = client
        .streaming(
            tonic::Request::new(futures_util::stream::iter([request])),
            grpc_reflection::V1.parse().unwrap(),
            api_server::RawCodec,
        )
        .await;
    let status = match result {
        Err(status) => status,
        Ok(response) => response.into_inner().message().await.unwrap_err(),
    };
    assert_eq!(status.code(), tonic::Code::Unimplemented);
    assert_eq!(
        status.message(),
        "unknown service grpc.reflection.v1.ServerReflection"
    );
    task.abort();
}
