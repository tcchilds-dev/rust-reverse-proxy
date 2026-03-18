mod common;

use common::{parse_echo, proxy_get, spawn_echo_backend, spawn_proxy};
use proxy::config::RouteConfig;
use reqwest::StatusCode;

#[tokio::test]
async fn routes_to_correct_backend_by_prefix() {
    let api_port = spawn_echo_backend("api-backend").await;
    let static_port = spawn_echo_backend("static-backend").await;

    let proxy_port = spawn_proxy(vec![
        RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec![format!("http://127.0.0.1:{api_port}")],
        },
        RouteConfig {
            path_prefix: "/static".to_string(),
            backends: vec![format!("http://127.0.0.1:{static_port}")],
        },
    ])
    .await;

    let (status, body) = proxy_get(proxy_port, "/api/foo").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["backend"], "api-backend");

    let (status, body) = proxy_get(proxy_port, "/static/style.css").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["backend"], "static-backend");
}

#[tokio::test]
async fn unknown_path_returns_404() {
    let port = spawn_echo_backend("backend").await;

    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/api".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await;

    let (status, _) = proxy_get(proxy_port, "/unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn exact_prefix_matches() {
    let port = spawn_echo_backend("backend").await;

    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/api".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await;

    let (status, body) = proxy_get(proxy_port, "/api").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["backend"], "backend");
}

#[tokio::test]
async fn deep_subpath_forwarded() {
    let port = spawn_echo_backend("backend").await;

    let proxy_port = spawn_proxy(vec![RouteConfig {
        path_prefix: "/api".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }])
    .await;

    let (status, body) = proxy_get(proxy_port, "/api/v1/users/123").await;
    assert_eq!(status, StatusCode::OK);
    let echo = parse_echo(&body);
    assert_eq!(echo["path"], "/api/v1/users/123");
}
