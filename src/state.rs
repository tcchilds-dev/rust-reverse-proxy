//! Shared application state threaded through Axum handlers via [`State`](axum::extract::State).

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use axum::http::HeaderName;
use moka::future::Cache;

use crate::{
    balancer::{LoadBalancer, two_random_choices::TwoRandomChoicesBalancer},
    cache::{CachedResponse, ResponseExpiry},
    config::Config,
};

/// Axum extension that tells the proxy handler which protocol the listener used,
/// so it can set `X-Forwarded-Proto` correctly.
#[derive(Clone)]
pub struct Scheme(pub &'static str);

/// A route maps a URL path prefix to a set of backends behind a load balancer.
pub struct Route {
    pub path_prefix: String,
    pub balancer: Arc<dyn LoadBalancer>,
}

#[derive(Clone)]
pub struct AppState {
    pub client: reqwest::Client,
    pub routes: Arc<Vec<Route>>,
    pub cache: Cache<String, Arc<CachedResponse>>,
    pub default_ttl: Duration,
    pub max_response_body_bytes: usize,
    /// Header names to remove from client requests before forwarding upstream.
    pub strip_request_headers: Vec<HeaderName>,
    /// Header names to remove from upstream responses before sending to clients.
    pub strip_response_headers: Vec<HeaderName>,
}

impl AppState {
    pub fn from_config(config: Config) -> Result<Self> {
        // Set the idle-connection timeout: None means "no timeout" (connections
        // stay open until the backend closes them), otherwise cap at the configured value.
        let pool_idle_timeout = if config.connection_pool.idle_timeout_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(config.connection_pool.idle_timeout_secs))
        };
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(config.connection_pool.max_idle_per_host)
            .pool_idle_timeout(pool_idle_timeout)
            .build()?;

        let health_check_config = config.health_checks.clone();

        let routes: Vec<Route> = config
            .routes
            .into_iter()
            .map(|r| Route {
                path_prefix: r.path_prefix,
                balancer: Arc::new(TwoRandomChoicesBalancer::new(
                    r.backends,
                    &health_check_config,
                )),
            })
            .collect();

        let cache = Cache::builder()
            .max_capacity(config.caching.max_capacity)
            .expire_after(ResponseExpiry {
                default_ttl: Duration::from_secs(config.caching.default_ttl),
            })
            .build();

        let strip_request_headers = config
            .sensitive_headers
            .strip_from_request
            .iter()
            .filter_map(|s| s.parse::<HeaderName>().ok())
            .collect();

        let strip_response_headers = config
            .sensitive_headers
            .strip_from_response
            .iter()
            .filter_map(|s| s.parse::<HeaderName>().ok())
            .collect();

        Ok(Self {
            client,
            routes: Arc::new(routes),
            cache,
            default_ttl: Duration::from_secs(config.caching.default_ttl),
            max_response_body_bytes: config.server.max_response_body_bytes,
            strip_request_headers,
            strip_response_headers,
        })
    }
}
