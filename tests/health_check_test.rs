mod common;

use std::time::Duration;

use common::{proxy_get, spawn_echo_backend, spawn_proxy_with_health_check};
use proxy::config::{HealthCheckConfig, RouteConfig};
use reqwest::StatusCode;

// Returns a port that has no server listening on it.
async fn dead_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // immediately release; nothing will listen here
    port
}

#[tokio::test]
async fn unhealthy_backend_is_skipped() {
    let healthy_port = spawn_echo_backend("healthy").await;
    let dead = dead_port().await;

    let proxy_port = spawn_proxy_with_health_check(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![
                format!("http://127.0.0.1:{healthy_port}"),
                format!("http://127.0.0.1:{dead}"),
            ],
        }],
        HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 1,
            timeout_secs: 1,
        },
    )
    .await;

    // Wait long enough for 3 health check failures on the dead backend (3 * 1s + margin).
    tokio::time::sleep(Duration::from_secs(4)).await;

    for _ in 0..10 {
        let (status, body) = proxy_get(proxy_port, "/test").await;
        assert_eq!(status, StatusCode::OK);
        let echo: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(echo["backend"], "healthy", "dead backend should be skipped");
    }
}

#[tokio::test]
async fn all_unhealthy_returns_502() {
    let dead = dead_port().await;

    let proxy_port = spawn_proxy_with_health_check(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{dead}")],
        }],
        HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 1,
            timeout_secs: 1,
        },
    )
    .await;

    // Wait for 3 consecutive health check failures (3s + margin).
    tokio::time::sleep(Duration::from_secs(4)).await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn backend_recovers_after_health_check_passes() {
    let dead = dead_port().await;

    let proxy_port = spawn_proxy_with_health_check(
        vec![RouteConfig {
            path_prefix: "/".to_string(),
            backends: vec![format!("http://127.0.0.1:{dead}")],
        }],
        HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 1,
            timeout_secs: 1,
        },
    )
    .await;

    // Wait for backend to be marked unhealthy.
    tokio::time::sleep(Duration::from_secs(4)).await;

    let (status, _) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "should be 502 before recovery");

    // Start a real backend on the previously dead port.
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{dead}"))
        .await
        .unwrap();
    let echo_router = axum::Router::new().fallback(move |_req: axum::extract::Request| async {
        axum::Json(serde_json::json!({"backend": "recovered", "method": "", "path": "", "query": "", "headers": {}, "body": ""}))
    });
    tokio::spawn(async move {
        axum::serve(listener, echo_router).await.unwrap();
    });

    // Wait for 3 consecutive health check successes (3s + margin).
    tokio::time::sleep(Duration::from_secs(4)).await;

    let (status, body) = proxy_get(proxy_port, "/test").await;
    assert_eq!(status, StatusCode::OK, "should be 200 after recovery");
    let echo: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(echo["backend"], "recovered");
}
