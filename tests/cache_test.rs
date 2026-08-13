//! Tests for response caching.
//!
//! Each test uses `spawn_counting_backend` which tracks how many non-healthcheck
//! requests actually reach the backend. Asserting on the call count lets us
//! distinguish cache hits (count stays the same) from misses (count increments).

mod common;

use std::sync::atomic::Ordering;

use proxy::config::RouteConfig;
use reqwest::{Method, StatusCode};

fn route(port: u16) -> RouteConfig {
    RouteConfig {
        path_prefix: "/".to_string(),
        backends: vec![format!("http://127.0.0.1:{port}")],
    }
}

#[tokio::test]
async fn test_get_response_is_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(None).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    let (status, _, _) =
        common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Second GET to the same URL — should be served from cache.
    let (status, _, _) =
        common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "backend hit a second time — response was not cached"
    );
}

#[tokio::test]
async fn test_authenticated_requests_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(None).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    let auth = vec![("authorization", "Bearer token123")];

    common::proxy_request(proxy_port, Method::GET, "/test", auth.clone(), None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Authenticated requests must bypass the cache.
    common::proxy_request(proxy_port, Method::GET, "/test", auth, None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "authenticated request was incorrectly served from cache"
    );
}

#[tokio::test]
async fn test_cookie_requests_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(None).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    let cookie = vec![("cookie", "session=abc123")];

    common::proxy_request(proxy_port, Method::GET, "/test", cookie.clone(), None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Requests carrying session cookies must bypass the cache.
    common::proxy_request(proxy_port, Method::GET, "/test", cookie, None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "cookie-bearing request was incorrectly served from cache"
    );
}

#[tokio::test]
async fn test_post_responses_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(None).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(
        proxy_port,
        Method::POST,
        "/test",
        vec![],
        Some("body".to_string()),
    )
    .await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // POST responses must never be cached.
    common::proxy_request(
        proxy_port,
        Method::POST,
        "/test",
        vec![],
        Some("body".to_string()),
    )
    .await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "POST response was incorrectly cached"
    );
}

#[tokio::test]
async fn test_no_store_responses_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(Some("no-store")).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Cache-Control: no-store must prevent caching.
    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "response with Cache-Control: no-store was incorrectly cached"
    );
}

#[tokio::test]
async fn test_private_responses_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(Some("private")).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Cache-Control: private must prevent caching.
    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "response with Cache-Control: private was incorrectly cached"
    );
}

#[tokio::test]
async fn test_no_cache_responses_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(Some("no-cache")).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Cache-Control: no-cache must prevent caching.
    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "response with Cache-Control: no-cache was incorrectly cached"
    );
}

#[tokio::test]
async fn test_vary_responses_not_cached() {
    let (backend_port, call_count) =
        common::spawn_counting_backend_with_headers(vec![("vary", "accept-encoding")]).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // The cache key ignores content negotiation, so responses with Vary
    // must not be cached at all.
    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "response with a Vary header was incorrectly cached"
    );
}

#[tokio::test]
async fn test_max_age_zero_responses_not_cached() {
    let (backend_port, call_count) = common::spawn_counting_backend(Some("max-age=0")).await;
    let proxy_port = common::spawn_proxy(vec![route(backend_port)]).await;

    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Cache-Control: max-age=0 must prevent caching.
    common::proxy_request(proxy_port, Method::GET, "/test", vec![], None).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "response with Cache-Control: max-age=0 was incorrectly cached"
    );
}
