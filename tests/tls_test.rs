mod common;

use common::{
    generate_test_certs, parse_echo, proxy_get, proxy_get_https, proxy_request_https,
    spawn_echo_backend, spawn_tls_proxy,
};
use proxy::config::RouteConfig;
use reqwest::{Method, StatusCode};

async fn setup() -> (u16, u16) {
    let backend_port = spawn_echo_backend("backend").await;
    let certs = generate_test_certs();
    spawn_tls_proxy(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{backend_port}")],
        }],
        &certs,
    )
    .await
}

#[tokio::test]
async fn https_proxies_request_to_backend() {
    let (_http_port, https_port) = setup().await;

    let (status, body) = proxy_get_https(https_port, "/hello").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["backend"], "backend");
    assert_eq!(echo["path"], "/hello");
}

#[tokio::test]
async fn https_sets_x_forwarded_proto_to_https() {
    let (_http_port, https_port) = setup().await;

    let (status, _, body) =
        proxy_request_https(https_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["headers"]["x-forwarded-proto"], "https");
}

#[tokio::test]
async fn http_still_sets_x_forwarded_proto_to_http() {
    let (http_port, _https_port) = setup().await;

    let (status, body) = proxy_get(http_port, "/test").await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["headers"]["x-forwarded-proto"], "http");
}

#[tokio::test]
async fn https_forwards_method_and_body() {
    let (_http_port, https_port) = setup().await;

    let (status, _, body) = proxy_request_https(
        https_port,
        Method::POST,
        "/data",
        vec![("content-type", "text/plain")],
        Some("tls payload".to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["method"], "POST");
    assert_eq!(echo["body"], "tls payload");
}

#[tokio::test]
async fn https_preserves_query_string() {
    let (_http_port, https_port) = setup().await;

    let (status, body) = proxy_get_https(https_port, "/search?q=rust&page=1").await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["query"], "q=rust&page=1");
}

#[tokio::test]
async fn https_injects_x_forwarded_for() {
    let (_http_port, https_port) = setup().await;

    let (status, _, body) =
        proxy_request_https(https_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xff = echo["headers"]["x-forwarded-for"].as_str().unwrap();
    assert!(xff.contains("127.0.0.1"));
}

#[tokio::test]
async fn https_unknown_route_returns_404() {
    let backend_port = spawn_echo_backend("backend").await;
    let certs = generate_test_certs();
    let (_http_port, https_port) = spawn_tls_proxy(
        vec![RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec![format!("http://127.0.0.1:{backend_port}")],
        }],
        &certs,
    )
    .await;

    let (status, _) = proxy_get_https(https_port, "/unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
