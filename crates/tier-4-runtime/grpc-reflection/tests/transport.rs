//! Real HTTP/2 socket replay of fresh, release-pinned Go reflection streams.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::Read as _;

use axum::body::Body;
use axum::http::Request;
use grpc_reflection::{Profile, proto};
use prost::Message;
use serde_json::Value;
use tonic::Status;

fn recording(profile: Profile) -> Value {
    let bytes: &[u8] = match profile {
        Profile::App => {
            include_bytes!("../../../../conformance/grpc-reflection/v10.5.0/app.json.gz")
        }
        Profile::Core => {
            include_bytes!("../../../../conformance/grpc-reflection/v10.5.0/core.json.gz")
        }
    };
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    serde_json::from_slice(&raw).unwrap()
}

async fn server(profile: Profile) -> (tonic::transport::Channel, tokio::task::JoinHandle<()>) {
    server_with_limits(
        profile,
        match profile {
            Profile::App => grpc_transport::Limits::sdk(),
            Profile::Core => grpc_transport::Limits::core(),
        },
    )
    .await
}

async fn server_with_limits(
    profile: Profile,
    limits: grpc_transport::Limits,
) -> (tonic::transport::Channel, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = grpc_reflection::router_with_limits(profile, limits).fallback(
        |req: Request<Body>| async move {
            let service = req
                .uri()
                .path()
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or("");
            Status::unimplemented(format!("unknown service {service}"))
                .into_http()
                .map(Body::new)
        },
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    (channel, task)
}

fn status_message(message: &str) -> String {
    // Go's protobuf internal/errors package deliberately randomizes the space
    // following its proto: prefix between ASCII space and NBSP per binary.
    message.replace("proto:\u{a0}", "proto: ")
}

fn canonical(mut response: proto::Response) -> proto::Response {
    if let Some(proto::response::Result::AllExtensionNumbersResponse(numbers)) =
        response.result.as_mut()
    {
        // gogoreflection ranges over an extension map. Its order is neither
        // stable within one process nor part of protobuf's extension semantics.
        numbers.extension_number.sort_unstable();
    }
    if let Some(proto::response::Result::ErrorResponse(error)) = response.result.as_mut() {
        error.error_message = status_message(&error.error_message);
    }
    response
}

async fn replay(profile: Profile) {
    let recording = recording(profile);
    replay_recording(
        profile,
        recording,
        match profile {
            Profile::App => grpc_transport::Limits::sdk(),
            Profile::Core => grpc_transport::Limits::core(),
        },
    )
    .await;
}

async fn replay_recording(profile: Profile, recording: Value, limits: grpc_transport::Limits) {
    let (channel, task) = server_with_limits(profile, limits).await;
    let mut client = tonic::client::Grpc::new(channel);
    for session in recording["sessions"].as_array().unwrap() {
        let exchanges = session["exchanges"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let requests: Vec<Vec<u8>> = exchanges
            .iter()
            .map(|ex| hex::decode(ex["request"].as_str().unwrap()).unwrap())
            .collect();
        client.ready().await.unwrap();
        let path = session["method"].as_str().unwrap().parse().unwrap();
        let result = client
            .streaming(
                tonic::Request::new(futures_util::stream::iter(requests)),
                path,
                grpc_transport::RawCodec::new(grpc_transport::Limits::sdk()),
            )
            .await;
        let expected_code = i32::try_from(session["code"].as_i64().unwrap_or_default()).unwrap();
        let expected_message = session["message"].as_str().unwrap_or_default();
        match result {
            Err(status) => {
                assert!(
                    exchanges.iter().all(|ex| ex.get("response").is_none()),
                    "early error: {session}"
                );
                assert_eq!(status.code() as i32, expected_code, "{session}");
                assert_eq!(
                    status_message(status.message()),
                    status_message(expected_message),
                    "{session}"
                );
            }
            Ok(response) => {
                assert_eq!(
                    response
                        .metadata()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok()),
                    Some("application/grpc")
                );
                assert!(response.metadata().get("x-cosmos-block-height").is_none());
                let mut stream = response.into_inner();
                let mut terminal = None;
                for exchange in exchanges {
                    if let Some(expected) = exchange["response"].as_str() {
                        let actual = stream.message().await.unwrap().expect("response frame");
                        let expected = hex::decode(expected).unwrap();
                        let a = canonical(proto::Response::decode(actual.as_slice()).unwrap());
                        let e = canonical(proto::Response::decode(expected.as_slice()).unwrap());
                        assert_eq!(a, e, "request {}", exchange["request"]);
                        // The nested original request must retain unknown fields.
                        let a = Original::decode(actual.as_slice()).unwrap();
                        let e = Original::decode(expected.as_slice()).unwrap();
                        assert_eq!(a, e);
                    } else {
                        terminal = Some(stream.message().await);
                        break;
                    }
                }
                let terminal = match terminal {
                    Some(terminal) => terminal,
                    None => stream.message().await,
                };
                match terminal {
                    Ok(None) => assert_eq!(expected_code, 0, "{session}"),
                    Err(status) => {
                        assert_eq!(status.code() as i32, expected_code, "{session}");
                        assert_eq!(
                            status_message(status.message()),
                            status_message(expected_message),
                            "{session}"
                        );
                    }
                    Ok(Some(_)) => {
                        panic!("unexpected response after final recorded frame: {session}")
                    }
                }
                if expected_code == 0 {
                    let trailers = stream.trailers().await.unwrap();
                    assert!(
                        trailers
                            .as_ref()
                            .is_none_or(|m| m.get("x-cosmos-block-height").is_none())
                    );
                }
            }
        }
    }
    task.abort();
}

#[derive(Clone, PartialEq, prost::Message)]
struct Original {
    #[prost(bytes = "vec", optional, tag = "2")]
    bytes: Option<Vec<u8>>,
}

#[tokio::test]
async fn sdk_v1alpha_matches_go_socket_streams() {
    replay(Profile::App).await;
}

#[tokio::test]
async fn core_v1_and_v1alpha_match_go_socket_streams() {
    replay(Profile::Core).await;
}

#[tokio::test]
async fn cancelled_streams_leave_the_connection_usable() {
    let (channel, task) = server(Profile::Core).await;
    let mut client = tonic::client::Grpc::new(channel);
    let request = proto::Request {
        host: String::new(),
        operation: Some(proto::request::Operation::ListServices(String::new())),
    }
    .encode_to_vec();
    for _ in 0..32 {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.send(request.clone()).await.unwrap();
        let input = futures_util::stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|message| (message, receiver))
        });
        client.ready().await.unwrap();
        let response = client
            .streaming(
                tonic::Request::new(input),
                grpc_reflection::V1.parse().unwrap(),
                grpc_transport::RawCodec::new(grpc_transport::Limits::sdk()),
            )
            .await
            .unwrap();
        let mut stream = response.into_inner();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.message())
                .await
                .unwrap()
                .unwrap()
                .is_some()
        );
        drop(stream);
        drop(sender);
    }
    // The same HTTP/2 connection remains usable after repeated cancellation.
    // There is no spawned task or outbound queue in the reflection service;
    // dropping a stream drops its decoder and finite descriptor-sent set.
    client.ready().await.unwrap();
    let response = client
        .streaming(
            tonic::Request::new(futures_util::stream::empty::<Vec<u8>>()),
            grpc_reflection::V1.parse().unwrap(),
            grpc_transport::RawCodec::new(grpc_transport::Limits::sdk()),
        )
        .await
        .unwrap();
    assert!(response.into_inner().message().await.unwrap().is_none());
    task.abort();
}

#[test]
fn descriptor_dependencies_are_current_release_bytes() {
    for profile in [Profile::App, Profile::Core] {
        let recording = recording(profile);
        assert!(recording["files"].as_object().unwrap().len() > 20);
        let mut sent = std::collections::BTreeSet::new();
        for (name, raw) in recording["files"].as_object().unwrap() {
            let response = profile
                .registry()
                .answer(
                    proto::Request {
                        host: String::new(),
                        operation: Some(proto::request::Operation::FileByFilename(name.clone())),
                    },
                    &mut sent,
                )
                .unwrap();
            let Some(proto::response::Result::FileDescriptorResponse(files)) = response.result
            else {
                panic!("missing captured file {name}")
            };
            assert_eq!(
                files.file_descriptor_proto[0],
                hex::decode(raw.as_str().unwrap()).unwrap()
            );
        }
    }
}

#[test]
fn evidence_and_embedded_registry_hashes_match_manifest() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let meta: Value = serde_json::from_slice(
        &std::fs::read(root.join("conformance/grpc-reflection/v10.5.0/meta.json")).unwrap(),
    )
    .unwrap();
    let hashes = meta["sha256"].as_object().unwrap();
    assert_eq!(hashes.len(), 19);
    for (name, expected) in hashes {
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            hex::encode(hash::sha256(&bytes)),
            expected.as_str().unwrap(),
            "stale evidence hash for {name}"
        );
    }
}

#[tokio::test]
async fn sdk_custom_limits_match_go_stream_errors() {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(
        include_bytes!("../../../../conformance/grpc-reflection/v10.5.0/app-limits.json.gz")
            .as_slice(),
    )
    .read_to_end(&mut raw)
    .unwrap();
    let capture = serde_json::from_slice(&raw).unwrap();
    replay_recording(
        Profile::App,
        capture,
        grpc_transport::Limits {
            max_receive: 64,
            max_send: 128,
        },
    )
    .await;
}

#[tokio::test]
async fn unresolvable_core_extension_is_a_bounded_error() {
    let (channel, task) = server(Profile::Core).await;
    let mut client = tonic::client::Grpc::new(channel);
    let crash = recording(Profile::Core); // registration still advertises extension65003.
    assert!(
        crash["extensions"]["google.protobuf.FieldOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_i64() == Some(65003))
    );
    let extension = proto::Request {
        host: String::new(),
        operation: Some(proto::request::Operation::FileContainingExtension(
            proto::ExtensionRequest {
                containing_type: "google.protobuf.FieldOptions".into(),
                extension_number: 65003,
            },
        )),
    }
    .encode_to_vec();
    let list = proto::Request {
        host: String::new(),
        operation: Some(proto::request::Operation::ListServices(String::new())),
    }
    .encode_to_vec();
    client.ready().await.unwrap();
    let response = client
        .streaming(
            tonic::Request::new(futures_util::stream::iter([extension, list])),
            grpc_reflection::V1ALPHA.parse().unwrap(),
            grpc_transport::RawCodec::new(grpc_transport::Limits::core()),
        )
        .await
        .unwrap();
    let mut stream = response.into_inner();
    let raw = stream.message().await.unwrap().unwrap();
    let response = proto::Response::decode(raw.as_slice()).unwrap();
    let Some(proto::response::Result::ErrorResponse(error)) = response.result else {
        panic!("unresolvable extension accepted")
    };
    assert_eq!(error.error_code, 5);
    assert_eq!(error.error_message, "proto: not found");
    assert!(stream.message().await.unwrap().is_some());
    assert!(stream.message().await.unwrap().is_none());
    task.abort();
}
