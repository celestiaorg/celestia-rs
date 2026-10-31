//! Serve the reflection router in isolation for transport diagnostics.
//! Production listeners use the same router through api-server and rpc.

use axum::body::Body;
use axum::http::Request;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = match std::env::args().nth(1).as_deref() {
        Some("app") => grpc_reflection::Profile::App,
        Some("core") => grpc_reflection::Profile::Core,
        _ => return Err("usage: serve <app|core> [127.0.0.1:20090]".into()),
    };
    let address = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:20090".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    let router = grpc_reflection::router(profile).fallback(|request: Request<Body>| async move {
        let service = request
            .uri()
            .path()
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("");
        tonic::Status::unimplemented(format!("unknown service {service}"))
            .into_http()
            .map(Body::new)
    });
    axum::serve(listener, router).await?;
    Ok(())
}
