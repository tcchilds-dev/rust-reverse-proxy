#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::{Extension, Router, routing::get};
use axum_server::tls_rustls::RustlsConfig;
use http_body_util::BodyExt;
use proxy::config::{Config, LoggingConfig, RouteConfig, ServerConfig, TlsConfig};
use proxy::proxy::proxy_handler;
use proxy::state::{AppState, Scheme};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

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
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
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

pub struct TlsCertPair {
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

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
    };

    let state = AppState::from_config(config.clone()).unwrap();

    let timeout = Duration::from_secs(config.server.request_timeout_secs);
    let max_concurrent = config.server.max_concurrent_requests;

    let router = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .fallback(proxy_handler)
        .with_state(state)
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

    let tmp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let https_port = tmp_listener.local_addr().unwrap().port();
    drop(tmp_listener);

    let https_addr: SocketAddr = format!("127.0.0.1:{https_port}").parse().unwrap();

    tokio::spawn(async move {
        axum_server::bind_rustls(https_addr, rustls_config)
            .serve(https_router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (http_port, https_port)
}

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
