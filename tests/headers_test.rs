mod common;

use common::{parse_echo, proxy_request, spawn_echo_backend, spawn_proxy};
use proxy::config::RouteConfig;
use reqwest::{Method, StatusCode};

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

    let (status, _, body) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xff = echo["headers"]["x-forwarded-for"].as_str().unwrap();
    assert!(xff.contains("127.0.0.1"));
}

#[tokio::test]
async fn appends_to_existing_x_forwarded_for() {
    let (proxy_port, _) = setup().await;

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
    assert!(xff.starts_with("10.0.0.1, "));
    assert!(xff.ends_with("127.0.0.1"));
}

#[tokio::test]
async fn injects_x_forwarded_proto() {
    let (proxy_port, _) = setup().await;

    let (status, _, body) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    assert_eq!(echo["headers"]["x-forwarded-proto"], "http");
}

#[tokio::test]
async fn rewrites_host_header_to_backend() {
    let (proxy_port, backend_port) = setup().await;

    let (status, _, body) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
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

    let (status, _, body) =
        proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let echo = parse_echo(&body);
    let xfh = echo["headers"]["x-forwarded-host"].as_str().unwrap();
    assert!(
        xfh.contains(&proxy_port.to_string()),
        "x-forwarded-host should contain original host, got: {xfh}"
    );
}
