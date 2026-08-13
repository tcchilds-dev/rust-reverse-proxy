//! Tests for the cached-response body size limit.
//!
//! The limit is only enforced on the buffered (caching) path: oversized
//! responses are still returned to the client in full, they just aren't
//! cached. Non-cached (streaming) responses are passed through regardless
//! of size.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::{Router, body::Body, response::Response};
use common::spawn_proxy_with_body_limits;
use proxy::config::RouteConfig;
use reqwest::StatusCode;

fn route(backend_port: u16) -> Vec<RouteConfig> {
    vec![RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{backend_port}")],
    }]
}

/// Spawns a backend that returns a cacheable response (Cache-Control: max-age=60)
/// with a body of exactly `size` bytes, counting how many requests it serves.
/// With `chunked` set, the body is streamed without a Content-Length header so
/// the proxy cannot detect the size up front.
async fn spawn_cacheable_backend(size: usize, chunked: bool) -> (u16, Arc<AtomicU32>) {
    let call_count = Arc::new(AtomicU32::new(0));
    let call_count_clone = call_count.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let router = Router::new().fallback(move |req: axum::extract::Request| {
            // Don't count health check requests from the balancer.
            if req.uri().path() != "/healthz" {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
            }
            async move {
                let body = if chunked {
                    let chunks = vec![Ok::<_, std::io::Error>(vec![b'x'; size])];
                    Body::from_stream(futures::stream::iter(chunks))
                } else {
                    Body::from(vec![b'x'; size])
                };
                Response::builder()
                    .status(200)
                    .header("cache-control", "max-age=60")
                    .body(body)
                    .unwrap()
            }
        });
        axum::serve(listener, router).await.unwrap();
    });
    (port, call_count)
}

#[tokio::test]
async fn cacheable_response_within_limit_is_returned_and_cached() {
    let (backend_port, call_count) = spawn_cacheable_backend(50, false).await;
    let proxy_port = spawn_proxy_with_body_limits(route(backend_port), 100).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len(), 50);

    // Second request should be served from cache.
    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "response within limit was not cached"
    );
}

#[tokio::test]
async fn cacheable_response_exceeding_limit_is_returned_but_not_cached() {
    // Backend sends 200 bytes with a Content-Length header; limit is 100 bytes,
    // so the proxy should skip buffering and stream the response through.
    let (backend_port, call_count) = spawn_cacheable_backend(200, false).await;
    let proxy_port = spawn_proxy_with_body_limits(route(backend_port), 100).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len(), 200);

    // The oversized response must not have been cached.
    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "oversized response was incorrectly cached"
    );
}

#[tokio::test]
async fn chunked_response_exceeding_limit_is_returned_but_not_cached() {
    // No Content-Length header: the proxy only discovers the size after
    // buffering, and must still return the full body without caching it.
    let (backend_port, call_count) = spawn_cacheable_backend(200, true).await;
    let proxy_port = spawn_proxy_with_body_limits(route(backend_port), 100).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len(), 200);

    let resp = reqwest::get(format!("http://127.0.0.1:{proxy_port}/"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "oversized chunked response was incorrectly cached"
    );
}
