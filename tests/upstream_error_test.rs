mod common;

use std::time::Duration;

use common::{proxy_get, spawn_proxy_with_timeout, spawn_slow_backend};
use proxy::config::RouteConfig;
use reqwest::StatusCode;

/// Returns a port that has no server listening on it.
async fn dead_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn connection_refused_returns_502() {
    let dead = dead_port().await;

    // Health check interval is 300s so the backend stays "healthy" (default).
    // The actual request will fail with a connection error.
    let proxy_port = spawn_proxy_with_timeout(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{dead}")],
        }],
        10,
    )
    .await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn request_timeout_returns_504() {
    let slow_port = spawn_slow_backend(Duration::from_secs(10)).await;

    let proxy_port = spawn_proxy_with_timeout(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{slow_port}")],
        }],
        1, // 1 second timeout
    )
    .await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
}
