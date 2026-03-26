//! Tests for the power-of-two-random-choices load balancer.
//!
//! These verify that traffic is distributed across backends and that the
//! algorithm degrades gracefully to a single backend.

mod common;

use std::collections::HashSet;

use common::{parse_echo, proxy_get, spawn_echo_backend, spawn_proxy};
use proxy::config::RouteConfig;
use reqwest::StatusCode;

#[tokio::test]
async fn single_backend_always_selected() {
    let port = spawn_echo_backend("only-one").await;

    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await;

    for _ in 0..10 {
        let (status, body) = proxy_get(proxy_port, "/test").await;
        assert_eq!(status, StatusCode::OK);
        let echo = parse_echo(&body);
        assert_eq!(echo["backend"], "only-one");
    }
}

#[tokio::test]
async fn multiple_backends_all_receive_traffic() {
    let port_a = spawn_echo_backend("backend-a").await;
    let port_b = spawn_echo_backend("backend-b").await;
    let port_c = spawn_echo_backend("backend-c").await;

    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![
            format!("http://127.0.0.1:{port_a}"),
            format!("http://127.0.0.1:{port_b}"),
            format!("http://127.0.0.1:{port_c}"),
        ],
    }])
    .await;

    let mut seen = HashSet::new();

    // Use a unique path per request so each one is a cache miss and actually
    // reaches a backend (caching on the same URL would send everything through
    // the first backend only).
    for i in 0..100 {
        let (status, body) = proxy_get(proxy_port, &format!("/test/{i}")).await;
        assert_eq!(status, StatusCode::OK);
        let echo = parse_echo(&body);
        seen.insert(echo["backend"].as_str().unwrap().to_string());
    }

    assert!(
        seen.contains("backend-a"),
        "backend-a never received traffic"
    );
    assert!(
        seen.contains("backend-b"),
        "backend-b never received traffic"
    );
    assert!(
        seen.contains("backend-c"),
        "backend-c never received traffic"
    );
}
