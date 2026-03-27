//! Tests for the cached-response body size limit.
//!
//! The limit is only enforced on the buffered (caching) path. Non-cached
//! (streaming) responses are passed through regardless of size.

mod common;

use common::spawn_proxy_with_body_limits;
use proxy::config::RouteConfig;
use reqwest::StatusCode;
use axum::{Router, body::Body, response::Response};

fn route(backend_port: u16) -> Vec<RouteConfig> {
    vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{backend_port}")],
    }]
}

/// Spawns a backend that returns a cacheable response (Cache-Control: max-age=60)
/// with a body of exactly `size` bytes.
async fn spawn_cacheable_backend(size: usize) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let router = Router::new().fallback(move || async move {
            Response::builder()
                .status(200)
                .header("cache-control", "max-age=60")
                .body(Body::from(vec![b'x'; size]))
                .unwrap()
        });
        axum::serve(listener, router).await.unwrap();
    });
    port
}

#[tokio::test]
async fn cacheable_response_within_limit_is_returned() {
    let backend_port = spawn_cacheable_backend(50).await;
    let proxy_port = spawn_proxy_with_body_limits(route(backend_port), 100).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len(), 50);
}

#[tokio::test]
async fn cacheable_response_exceeding_limit_returns_502() {
    let backend_port = spawn_cacheable_backend(200).await;
    // Limit is 100 bytes; backend will send 200.
    let proxy_port = spawn_proxy_with_body_limits(route(backend_port), 100).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}
