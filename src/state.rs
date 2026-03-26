//! Shared application state threaded through Axum handlers via [`State`](axum::extract::State).

use std::{sync::Arc, time::Duration};

use anyhow::Result;
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
}

impl AppState {
    pub fn from_config(config: Config) -> Result<Self> {
        let client = reqwest::Client::new();

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

        Ok(Self {
            client,
            routes: Arc::new(routes),
            cache,
            default_ttl: Duration::from_secs(config.caching.default_ttl),
        })
    }
}
