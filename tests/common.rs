//! Shared test helpers for integration tests.
//!
//! Each helper spins up real HTTP servers on OS-assigned ports (`127.0.0.1:0`),
//! so tests are fully parallel and never collide on ports. The echo backend
//! mirrors every request as JSON, making it easy to assert on what the proxy
//! actually forwarded.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::{Extension, Router, routing::get};
use axum_server::tls_rustls::{RustlsConfig, from_tcp_rustls};
use http_body_util::BodyExt;
use proxy::config::{
    CacheConfig, Config, HealthCheckConfig, LoggingConfig, RateLimitConfig, RouteConfig,
    ServerConfig, TlsConfig,
};
use proxy::proxy::proxy_handler;
use proxy::rate_limiter::RateLimitLayer;
use proxy::state::{AppState, Scheme};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Spawns an HTTP server that echoes every request back as JSON containing the
/// backend name, method, path, query, headers, and body. Returns the listening port.
pub async fn spawn_echo_backend(name: &str) -> u16 {
    let name = name.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let router = Router::new().fallback(move |req: Request<Body>| {
        let name = name.clone();
        async move {
            let method = req.method().to_string();
            let path = req.uri().path().to_string();
            let query = req.uri().query().unwrap_or("").to_string();

            let headers: HashMap<String, String> = req
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
                .collect();

            let body_bytes = BodyExt::collect(req.into_body())
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&body_bytes).to_string();

            axum::Json(json!({
                "backend": name,
                "method": method,
                "path": path,
                "query": query,
                "headers": headers,
                "body": body,
            }))
        }
    });

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    port
}

/// Spawns a proxy with the production middleware stack, default config values,
/// and a 300s health check interval (effectively disabled for short tests).
/// Returns the listening port.
pub async fn spawn_proxy(routes: Vec<RouteConfig>) -> u16 {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 10,
            max_concurrent_requests: 100,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: None,
        rate_limiting: RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        },
        health_checks: HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 300,
            timeout_secs: 3,
        },
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        )
        .layer(Extension(Scheme("http")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    port
}

pub async fn spawn_proxy_with_rate_limit(
    routes: Vec<RouteConfig>,
    requests_per_second: u32,
    burst_size: u32,
) -> u16 {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 10,
            max_concurrent_requests: 100,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: None,
        rate_limiting: RateLimitConfig {
            requests_per_second,
            burst_size,
        },
        health_checks: HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 300,
            timeout_secs: 3,
        },
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        )
        .layer(Extension(Scheme("http")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    port
}

pub async fn spawn_proxy_with_health_check(
    routes: Vec<RouteConfig>,
    health_check: HealthCheckConfig,
) -> u16 {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 10,
            max_concurrent_requests: 100,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: None,
        rate_limiting: RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        },
        health_checks: health_check,
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        )
        .layer(Extension(Scheme("http")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    port
}

pub async fn proxy_get(proxy_port: u16, path: &str) -> (StatusCode, String) {
    let url = format!("http://127.0.0.1:{proxy_port}{path}");
    let resp = reqwest::get(&url).await.unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    (status, body)
}

pub async fn proxy_request(
    proxy_port: u16,
    method: Method,
    path: &str,
    headers: Vec<(&str, &str)>,
    body: Option<String>,
) -> (StatusCode, reqwest::header::HeaderMap, String) {
    let url = format!("http://127.0.0.1:{proxy_port}{path}");
    let client = reqwest::Client::new();
    let mut builder = client.request(method, &url);

    for (k, v) in headers {
        builder = builder.header(k, v);
    }

    if let Some(body) = body {
        builder = builder.body(body);
    }

    let resp = builder.send().await.unwrap();
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let body = resp.text().await.unwrap();
    (status, resp_headers, body)
}

pub fn parse_echo(body: &str) -> Value {
    serde_json::from_str(body).unwrap()
}

/// Holds paths to a self-signed TLS cert/key pair. The `_dir` field keeps the
/// temp directory alive for the lifetime of this struct.
pub struct TlsCertPair {
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// Generates a self-signed TLS certificate valid for `localhost` and `127.0.0.1`.
pub fn generate_test_certs() -> TlsCertPair {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");

    let mut cert_file = std::fs::File::create(&cert_path).unwrap();
    cert_file.write_all(cert.cert.pem().as_bytes()).unwrap();

    let mut key_file = std::fs::File::create(&key_path).unwrap();
    key_file
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    TlsCertPair {
        cert_path,
        key_path,
        _dir: dir,
    }
}

/// Spawns a proxy with both HTTP and HTTPS listeners. Returns `(http_port, https_port)`.
pub async fn spawn_tls_proxy(routes: Vec<RouteConfig>, certs: &TlsCertPair) -> (u16, u16) {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 10,
            max_concurrent_requests: 100,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: Some(TlsConfig {
            cert_path: certs.cert_path.to_str().unwrap().to_string(),
            key_path: certs.key_path.to_str().unwrap().to_string(),
            addr: "127.0.0.1:0".parse().unwrap(),
        }),
        rate_limiting: RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        },
        health_checks: HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 300,
            timeout_secs: 3,
        },
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        );

    let http_router = router.clone().layer(Extension(Scheme("http")));
    let https_router = router.layer(Extension(Scheme("https")));

    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            http_listener,
            http_router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let tls_config = config.tls.unwrap();
    let rustls_config = RustlsConfig::from_pem_file(&tls_config.cert_path, &tls_config.key_path)
        .await
        .unwrap();

    // Bind at port 0 to get an OS-assigned port, then convert to a std listener
    // so axum_server can take ownership. This avoids the TOCTOU race of
    // binding, dropping, and re-binding that the old bind_rustls(addr) approach required.
    let https_tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let https_port = https_tcp.local_addr().unwrap().port();
    let https_std = https_tcp.into_std().unwrap();

    tokio::spawn(async move {
        from_tcp_rustls(https_std, rustls_config)
            .unwrap()
            .serve(https_router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (http_port, https_port)
}

/// Returns a reqwest client that accepts the self-signed test certs.
pub fn https_client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
}

pub async fn proxy_get_https(proxy_port: u16, path: &str) -> (StatusCode, String) {
    let url = format!("https://127.0.0.1:{proxy_port}{path}");
    let client = https_client();
    let resp = client.get(&url).send().await.unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    (status, body)
}

pub async fn proxy_request_https(
    proxy_port: u16,
    method: Method,
    path: &str,
    headers: Vec<(&str, &str)>,
    body: Option<String>,
) -> (StatusCode, reqwest::header::HeaderMap, String) {
    let url = format!("https://127.0.0.1:{proxy_port}{path}");
    let client = https_client();
    let mut builder = client.request(method, &url);

    for (k, v) in headers {
        builder = builder.header(k, v);
    }

    if let Some(body) = body {
        builder = builder.body(body);
    }

    let resp = builder.send().await.unwrap();
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let body = resp.text().await.unwrap();
    (status, resp_headers, body)
}

/// Spawns a backend that counts how many times it has been called and optionally sets a
/// `Cache-Control` header on every response. Returns (port, call_count).
pub async fn spawn_counting_backend(
    cache_control: Option<&'static str>,
) -> (u16, Arc<AtomicU32>) {
    let call_count = Arc::new(AtomicU32::new(0));
    let call_count_clone = call_count.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let router = Router::new().fallback(move |req: Request<Body>| {
            // Don't count health check requests from the balancer.
            let is_health_check = req.uri().path() == "/healthz";
            let count = if !is_health_check {
                call_count_clone.fetch_add(1, Ordering::SeqCst)
            } else {
                call_count_clone.load(Ordering::SeqCst)
            };
            async move {
                let mut builder =
                    axum::response::Response::builder().status(StatusCode::OK);
                if let Some(cc) = cache_control {
                    builder = builder.header("cache-control", cc);
                }
                builder
                    .body(Body::from(format!(r#"{{"count":{count}}}"#)))
                    .unwrap()
            }
        });
        axum::serve(listener, router).await.unwrap();
    });

    (port, call_count)
}

/// Spawns a backend that sleeps for `delay` before responding.
pub async fn spawn_slow_backend(delay: Duration) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let router = Router::new().fallback(move |_req: Request<Body>| async move {
            tokio::time::sleep(delay).await;
            axum::Json(json!({"backend": "slow"}))
        });
        axum::serve(listener, router).await.unwrap();
    });

    port
}

/// Spawns a proxy with a custom request timeout (in seconds).
pub async fn spawn_proxy_with_timeout(routes: Vec<RouteConfig>, timeout_secs: u64) -> u16 {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: timeout_secs,
            max_concurrent_requests: 100,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: None,
        rate_limiting: RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        },
        health_checks: HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 300,
            timeout_secs: 3,
        },
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        )
        .layer(Extension(Scheme("http")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    port
}

/// Spawns a proxy with a custom concurrency limit.
pub async fn spawn_proxy_with_concurrency_limit(
    routes: Vec<RouteConfig>,
    max_concurrent: usize,
) -> u16 {
    let config = Config {
        server: ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 30,
            max_concurrent_requests: max_concurrent,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        routes,
        tls: None,
        rate_limiting: RateLimitConfig {
            requests_per_second: 1000,
            burst_size: 2000,
        },
        health_checks: HealthCheckConfig {
            path: "/healthz".to_string(),
            interval_secs: 300,
            timeout_secs: 3,
        },
        caching: CacheConfig {
            max_capacity: 100,
            default_ttl: 60,
        },
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    Duration::from_secs(timeout.as_secs()),
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent)),
        )
        .layer(Extension(Scheme("http")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    port
}
