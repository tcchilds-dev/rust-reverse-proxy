//! Entry point for the reverse proxy.
//!
//! Sets up the middleware stack, binds HTTP (and optionally HTTPS) listeners,
//! and dispatches all non-health-check traffic through [`proxy_handler`].

use std::{net::SocketAddr, path::Path, time::Duration};

use anyhow::Result;
use axum::{Extension, Router, routing::get};
use axum_server::tls_rustls::RustlsConfig;
use metrics_exporter_prometheus::PrometheusBuilder;
use proxy::{
    config::Config,
    metrics::MetricsLayer,
    proxy::proxy_handler,
    rate_limiter::RateLimitLayer,
    state::{AppState, Scheme},
};
use reqwest::StatusCode;
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tracing::{Level, info};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_file(Path::new("config.toml"))?;

    tracing_subscriber::fmt()
        .with_max_level(match config.logging.level.to_lowercase() {
            val if val == "debug" => Level::DEBUG,
            val if val == "info" => Level::INFO,
            val if val == "warn" => Level::WARN,
            val if val == "error" => Level::ERROR,
            _ => Level::INFO,
        })
        .init();

    let state = AppState::from_config(config.clone())?;

    let timeout = Duration::from_secs(config.server.request_timeout_secs);

    let max_concurrent_requests = config.server.max_concurrent_requests;

    let rate_limit_layer = RateLimitLayer::new(
        config.rate_limiting.requests_per_second,
        config.rate_limiting.burst_size,
    );

    // Install the global metrics recorder. Histogram buckets are tuned for
    // typical proxy latencies (sub-millisecond to a few seconds).
    let recorder_handle = PrometheusBuilder::new()
        .set_buckets(&[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5])
        .expect("valid buckets")
        .install_recorder()
        .expect("failed to install recorder");

    // Layers execute bottom-up: concurrency limit → timeout → trace → metrics → rate limit → handler.
    let proxy_route = Router::new()
        .route("/healthz", get(healthz))
        .fallback(proxy_handler)
        .with_state(state)
        .layer(rate_limit_layer)
        .layer(
            ServiceBuilder::new()
                .layer(MetricsLayer)
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent_requests)),
        );

    // Serve Prometheus metrics on a separate router so they bypass the proxy
    // middleware stack (rate limiting, timeouts, etc.).
    let metrics_route = Router::new().route(
        "/metrics",
        get(move || async move { recorder_handle.render() }),
    );

    let router = metrics_route.merge(proxy_route);

    // The Scheme extension lets the proxy handler set X-Forwarded-Proto correctly
    // depending on which listener accepted the connection.
    let http_router = router.clone().layer(Extension(Scheme("http")));
    let http_listener = tokio::net::TcpListener::bind(config.server.addr).await?;
    info!("HTTP listening on {}", config.server.addr);

    match config.tls {
        Some(tls_config) => {
            let rustls_config =
                RustlsConfig::from_pem_file(&tls_config.cert_path, &tls_config.key_path).await?;
            let https_router = router.layer(Extension(Scheme("https")));

            info!("HTTPS server listening on {}", tls_config.addr);

            let http_server = axum::serve(
                http_listener,
                http_router.into_make_service_with_connect_info::<SocketAddr>(),
            );
            let https_server = axum_server::bind_rustls(tls_config.addr, rustls_config)
                .serve(https_router.into_make_service_with_connect_info::<SocketAddr>());

            tokio::select! {
                result = http_server.into_future() => result?,
                result = https_server => result?,
            }
        }
        None => {
            info!("TLS not configured, running HTTP only");

            axum::serve(
                http_listener,
                http_router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
        }
    }

    Ok(())
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}
