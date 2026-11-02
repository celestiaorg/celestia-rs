//! SDK errors send height metadata before their status trailers in grpc-go.
#![allow(clippy::unwrap_used)]
use http_body_util::BodyExt as _;

#[tokio::test]
async fn sdk_error_preserves_initial_height_and_sends_status_in_trailers() {
    let mut status = tonic::Status::invalid_argument("invalid account");
    status
        .metadata_mut()
        .insert("x-cosmos-block-height", "19".parse().unwrap());
    let response = grpc_transport::response(status.into_http());
    let response = grpc_transport::query_error_trailers(response, "x-cosmos-block-height");
    assert_eq!(response.headers()["x-cosmos-block-height"], "19");
    assert!(!response.headers().contains_key("grpc-status"));
    let body = response.into_body().collect().await.unwrap();
    let trailers = body.trailers().unwrap();
    assert_eq!(trailers["grpc-status"], "3");
    assert_eq!(trailers["grpc-message"], "invalid%20account");
    assert!(!trailers.contains_key("x-cosmos-block-height"));
    assert!(body.to_bytes().is_empty());
}

#[tokio::test]
async fn pre_dispatch_error_remains_trailers_only_without_a_height() {
    let response =
        grpc_transport::response(tonic::Status::unimplemented("unknown service").into_http());
    let response = grpc_transport::query_error_trailers(response, "x-cosmos-block-height");
    assert_eq!(response.headers()["grpc-status"], "12");
    assert!(!response.headers().contains_key("x-cosmos-block-height"));
    let body = response.into_body().collect().await.unwrap();
    assert!(body.trailers().is_none());
    assert!(body.to_bytes().is_empty());
}
