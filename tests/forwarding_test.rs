mod common;

use common::{parse_echo, proxy_get, proxy_request, spawn_echo_backend, spawn_proxy};
use proxy::config::RouteConfig;
use reqwest::{Method, StatusCode};

async fn setup() -> u16 {
    let port = spawn_echo_backend("backend").await;
    spawn_proxy(vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await
}

#[tokio::test]
async fn forwards_get_method() {
    let proxy_port = setup().await;

    let (status, body) = proxy_get(proxy_port, "/hello").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["method"], "GET");
}

#[tokio::test]
async fn forwards_post_method() {
    let proxy_port = setup().await;

    let (status, _, body) =
        proxy_request(proxy_port, Method::POST, "/data", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["method"], "POST");
}

#[tokio::test]
async fn forwards_put_method() {
    let proxy_port = setup().await;

    let (status, _, body) =
        proxy_request(proxy_port, Method::PUT, "/data", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["method"], "PUT");
}

#[tokio::test]
async fn forwards_delete_method() {
    let proxy_port = setup().await;

    let (status, _, body) =
        proxy_request(proxy_port, Method::DELETE, "/data", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["method"], "DELETE");
}

#[tokio::test]
async fn preserves_path() {
    let proxy_port = setup().await;

    let (_, body) = proxy_get(proxy_port, "/api/v1/users/123").await;
    let echo = parse_echo(&body);
    assert_eq!(echo["path"], "/api/v1/users/123");
}

#[tokio::test]
async fn preserves_query_string() {
    let proxy_port = setup().await;

    let (_, body) = proxy_get(proxy_port, "/search?q=hello&page=2").await;
    let echo = parse_echo(&body);
    assert_eq!(echo["query"], "q=hello&page=2");
}

#[tokio::test]
async fn forwards_request_body() {
    let proxy_port = setup().await;

    let (status, _, body) = proxy_request(
        proxy_port,
        Method::POST,
        "/data",
        vec![("content-type", "text/plain")],
        Some("hello world".to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["body"], "hello world");
}
