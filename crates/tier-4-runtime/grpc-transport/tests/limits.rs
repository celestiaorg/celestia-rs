//! Exercise framed requests and streaming statuses through actual HTTP/2 sockets.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use grpc_transport::{Limits, RawCodec};
use std::future::{Ready, ready};
use tonic::Status;

type Stream = std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, Status>> + Send>>;

struct Echo;
impl tonic::server::UnaryService<Vec<u8>> for Echo {
    type Response = Vec<u8>;
    type Future = Ready<Result<tonic::Response<Vec<u8>>, Status>>;
    fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
        let mut response = tonic::Response::new(request.into_inner());
        response
            .metadata_mut()
            .insert("x-test-header", "retained".parse().unwrap());
        ready(Ok(response))
    }
}
struct Parts;
impl tonic::server::ServerStreamingService<Vec<u8>> for Parts {
    type Response = Vec<u8>;
    type ResponseStream = Stream;
    type Future = Ready<Result<tonic::Response<Stream>, Status>>;
    fn call(&mut self, _: tonic::Request<Vec<u8>>) -> Self::Future {
        // A small frame must reach the client before the oversized one fails.
        let stream: Stream = Box::pin(futures_util::stream::iter([
            Ok(vec![0; 32]),
            Ok(vec![0; 33]),
        ]));
        ready(Ok(tonic::Response::new(stream)))
    }
}
async fn handle(State(limits): State<Limits>, req: Request<Body>) -> Response<Body> {
    let streaming = req.uri().path().ends_with("/Parts");
    let mut grpc = limits.grpc();
    if streaming {
        grpc_transport::response(grpc.server_streaming(Parts, req).await)
    } else {
        grpc_transport::response(grpc.unary(Echo, req).await)
    }
}
async fn client(
    limits: Limits,
) -> (
    tonic::client::Grpc<tonic::transport::Channel>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().fallback(handle).with_state(limits);
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    (tonic::client::Grpc::new(channel), task)
}
#[tokio::test]
async fn request_boundary_and_capacity_after_failure() {
    let limits = Limits {
        max_receive: 64,
        max_send: 128,
    };
    let (mut client, server) = client(limits).await;
    for len in [63, 64, 65, 1] {
        client.ready().await.unwrap();
        let result = client
            .unary(
                tonic::Request::new(vec![7; len]),
                "/test.Echo/Unary".parse().unwrap(),
                RawCodec::default(),
            )
            .await;
        if len <= 64 {
            let response = result.unwrap();
            assert_eq!(response.into_inner(), vec![7; len]);
        } else {
            let status = result.unwrap_err();
            assert_eq!(status.code(), tonic::Code::ResourceExhausted);
            assert_eq!(
                status.message(),
                "grpc: received message larger than max (65 vs. 64)"
            );
        }
    }
    server.abort();
}
#[tokio::test]
async fn response_boundary_and_metadata() {
    let (mut client, server) = client(Limits {
        max_receive: 64,
        max_send: 32,
    })
    .await;
    for len in [31, 32, 33, 1] {
        client.ready().await.unwrap();
        let result = client
            .unary(
                tonic::Request::new(vec![7; len]),
                "/test.Echo/Unary".parse().unwrap(),
                RawCodec::default(),
            )
            .await;
        if len <= 32 {
            let response = result.unwrap();
            assert_eq!(
                response.metadata().get("x-test-header").unwrap(),
                "retained"
            );
            assert_eq!(response.into_inner(), vec![7; len]);
        } else {
            let status = result.unwrap_err();
            assert_eq!(status.code(), tonic::Code::ResourceExhausted);
            assert_eq!(
                status.message(),
                "grpc: trying to send message larger than max (33 vs. 32)"
            );
        }
    }
    server.abort();
}
#[tokio::test]
async fn streaming_limit_is_per_message_and_preserves_prior_frames() {
    let (mut client, server) = client(Limits {
        max_receive: 64,
        max_send: 32,
    })
    .await;
    client.ready().await.unwrap();
    let response = client
        .server_streaming(
            tonic::Request::new(Vec::new()),
            "/test.Echo/Parts".parse().unwrap(),
            RawCodec::default(),
        )
        .await
        .unwrap();
    let mut stream = response.into_inner();
    assert_eq!(stream.message().await.unwrap(), Some(vec![0; 32]));
    let status = stream.message().await.unwrap_err();
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    assert_eq!(
        status.message(),
        "grpc: trying to send message larger than max (33 vs. 32)"
    );
    server.abort();
}
#[tokio::test]
async fn declared_oversize_rejected_without_waiting_for_or_allocating_payload() {
    use tower::ServiceExt as _;
    let limits = Limits {
        max_receive: 64,
        max_send: 32,
    };
    let router = axum::Router::new().fallback(handle).with_state(limits);
    let response = router
        .oneshot(
            Request::post("/test.Echo/Unary")
                .header("content-type", "application/grpc")
                .body(Body::from(vec![0, 255, 255, 255, 255]))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = Status::from_header_map(response.headers()).unwrap();
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    assert_eq!(
        status.message(),
        "grpc: received message larger than max (4294967295 vs. 64)"
    );
    // The request contains only a header. Accepting the declaration first would
    // attempt a multi-gigabyte reserve or report truncated data instead.
}
