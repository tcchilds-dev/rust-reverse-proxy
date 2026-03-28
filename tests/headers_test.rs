//! Tests for header manipulation: hop-by-hop stripping, Host rewriting,
//! and X-Forwarded-{For,Host,Proto} injection.

mod common;

use common::{
    parse_echo, proxy_request, spawn_backend_with_response_header, spawn_echo_backend, spawn_proxy,
    spawn_proxy_with_sensitive_headers,
};
use proxy::config::{RouteConfig, SensitiveHeadersConfig};
use reqwest::{Method, StatusCode};

/// Returns `(proxy_port, backend_port)` — backend port is needed to assert
/// that the Host header was rewritten to the backend's address.
async fn setup() -> (u16, u16) {
    let backend_port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{backend_port}")],
    }])
    .await;
    (proxy_port, backend_port)
}

#[tokio::test]
async fn strips_hop_by_hop_headers() {
    let (proxy_port, _) = setup().await;

    let (status, _, body) = proxy_request(
        proxy_port,
        Method::GET,
        "/test",
        vec![
            ("connection", "keep-alive"),
            ("transfer-encoding", "chunked"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let headers = &echo["headers"];
    assert!(headers.get("connection").is_none());
    assert!(headers.get("transfer-encoding").is_none());
}

#[tokio::test]
async fn injects_x_forwarded_for() {
    let (proxy_port, _) = setup().await;

    let (status, _, body) = proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xff = echo["headers"]["x-forwarded-for"].as_str().unwrap();
    assert!(xff.contains("127.0.0.1"));
}

#[tokio::test]
async fn client_supplied_x_forwarded_for_is_replaced() {
    let (proxy_port, _) = setup().await;

    // A client can supply a fake upstream IP to spoof their origin. The proxy
    // must discard any client-supplied X-Forwarded-For and replace it with the
    // actual direct connection IP so backends cannot be deceived.
    let (status, _, body) = proxy_request(
        proxy_port,
        Method::GET,
        "/test",
        vec![("x-forwarded-for", "10.0.0.1")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xff = echo["headers"]["x-forwarded-for"].as_str().unwrap();
    assert_eq!(xff, "127.0.0.1", "proxy must replace spoofed X-Forwarded-For");
    assert!(!xff.contains("10.0.0.1"), "spoofed IP must not appear in the header");
}

#[tokio::test]
async fn injects_x_forwarded_proto() {
    let (proxy_port, _) = setup().await;

    let (status, _, body) = proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["headers"]["x-forwarded-proto"], "http");
}

#[tokio::test]
async fn rewrites_host_header_to_backend() {
    let (proxy_port, backend_port) = setup().await;

    let (status, _, body) = proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let host = echo["headers"]["host"].as_str().unwrap();
    assert!(
        host.contains(&backend_port.to_string()),
        "host header should contain backend port, got: {host}"
    );
}

#[tokio::test]
async fn injects_x_forwarded_host() {
    let (proxy_port, _) = setup().await;

    let (status, _, body) = proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xfh = echo["headers"]["x-forwarded-host"].as_str().unwrap();
    assert!(
        xfh.contains(&proxy_port.to_string()),
        "x-forwarded-host should contain original host, got: {xfh}"
    );
}

#[tokio::test]
async fn generates_request_id_when_absent() {
    let (proxy_port, _) = setup().await;

    let (status, resp_headers, body) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    // The proxy must generate an X-Request-ID and echo it back to the client.
    let resp_id = resp_headers
        .get("x-request-id")
        .expect("proxy must add X-Request-ID to response")
        .to_str()
        .unwrap();
    assert!(!resp_id.is_empty());

    // The same ID must be forwarded to the upstream backend.
    let echo = parse_echo(&body);
    let upstream_id = echo["headers"]["x-request-id"].as_str().unwrap();
    assert_eq!(resp_id, upstream_id, "response and upstream X-Request-ID must match");
}

#[tokio::test]
async fn strips_configured_request_header_before_forwarding() {
    let backend_port = spawn_echo_backend("backend").await;
    let proxy_port = spawn_proxy_with_sensitive_headers(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{backend_port}")],
        }],
        SensitiveHeadersConfig {
            strip_from_request: vec!["x-internal-auth".to_string()],
            strip_from_response: vec![],
        },
    )
    .await;

    // Client sends a header it shouldn't be able to forge.
    let (status, _, body) = proxy_request(
        proxy_port,
        Method::GET,
        "/test",
        vec![("x-internal-auth", "super-secret")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert!(
        echo["headers"].get("x-internal-auth").is_none(),
        "stripped header must not reach the backend"
    );
}

#[tokio::test]
async fn strips_configured_response_header_before_returning_to_client() {
    // Backend sets a header that reveals server internals.
    let backend_port = spawn_backend_with_response_header("x-powered-by", "SecretFramework/1.0").await;
    let proxy_port = spawn_proxy_with_sensitive_headers(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{backend_port}")],
        }],
        SensitiveHeadersConfig {
            strip_from_request: vec![],
            strip_from_response: vec!["x-powered-by".to_string()],
        },
    )
    .await;

    let (status, resp_headers, _) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp_headers.get("x-powered-by").is_none(),
        "stripped response header must not reach the client"
    );
}

#[tokio::test]
async fn propagates_existing_request_id() {
    let (proxy_port, _) = setup().await;

    let client_id = "test-correlation-id-abc123";
    let (status, resp_headers, body) = proxy_request(
        proxy_port,
        Method::GET,
        "/test",
        vec![("x-request-id", client_id)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The client-supplied ID must be echoed back in the response.
    let resp_id = resp_headers
        .get("x-request-id")
        .expect("proxy must echo X-Request-ID in response")
        .to_str()
        .unwrap();
    assert_eq!(resp_id, client_id);

    // The same ID must be forwarded to the upstream backend.
    let echo = parse_echo(&body);
    assert_eq!(echo["headers"]["x-request-id"].as_str().unwrap(), client_id);
}
