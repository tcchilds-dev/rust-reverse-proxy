use std::{path::Path, time::Duration};

use anyhow::Result;
use axum::Router;
use proxy::{config::Config, proxy::proxy_handler, state::AppState};
use reqwest::StatusCode;
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tracing::Level;

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

    let router = Router::new()
        .fallback(proxy_handler)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::GATEWAY_TIMEOUT,
                    timeout,
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent_requests)),
        );

    let listener = tokio::net::TcpListener::bind(config.server.addr).await?;

    axum::serve(listener, router).await?;

    Ok(())
}
