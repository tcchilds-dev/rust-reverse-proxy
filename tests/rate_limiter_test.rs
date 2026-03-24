mod common;

use common::{
    parse_echo, proxy_get, proxy_request, spawn_echo_backend, spawn_proxy,
    spawn_proxy_with_rate_limit,
};
use proxy::config::RouteConfig;
use reqwest::{Method, StatusCode};

#[tokio::test]
async fn requests_within_burst_are_allowed() {
    let port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy_with_rate_limit(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{port}")],
        }],
        1,
        5,
    )
    .await;

    for _ in 0..5 {
        let (status, body) = proxy_get(proxy_port, "/test").await;
        assert_eq!(status, StatusCode::OK);
        let echo = parse_echo(&body);
        assert_eq!(echo["backend"], "backend");
    }
}

#[tokio::test]
async fn returns_429_when_rate_limit_exceeded() {
    let port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy_with_rate_limit(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{port}")],
        }],
        1,
        1,
    )
    .await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_response_includes_retry_after_header() {
    let port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy_with_rate_limit(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{port}")],
        }],
        1,
        1,
    )
    .await;

    proxy_get(proxy_port, "/test").await;

    let (status, headers, _) = proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        headers.contains_key("retry-after"),
        "expected Retry-After header on 429 response"
    );
}

#[tokio::test]
async fn rate_limit_recovers_after_waiting() {
    let port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy_with_rate_limit(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{port}")],
        }],
        1,
        1,
    )
    .await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn normal_traffic_unaffected_by_high_limit() {
    let port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await;

    for _ in 0..20 {
        let (status, _) = proxy_get(proxy_port, "/test").await;
        assert_eq!(status, StatusCode::OK);
    }
}
