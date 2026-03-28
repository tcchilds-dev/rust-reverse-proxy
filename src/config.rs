//! TOML-based configuration with validation.

pub mod validation;

use std::{net::SocketAddr, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::validation::validate_config;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    pub routes: Vec<RouteConfig>,
    pub tls: Option<TlsConfig>,
    pub rate_limiting: RateLimitConfig,
    pub health_checks: HealthCheckConfig,
    pub caching: CacheConfig,
    /// Omitting this section is equivalent to empty lists (nothing is stripped).
    #[serde(default)]
    pub sensitive_headers: SensitiveHeadersConfig,
    /// Omitting this section uses built-in defaults (32 idle connections/host, 90s timeout).
    #[serde(default)]
    pub connection_pool: ConnectionPoolConfig,
}

/// Tuning knobs for the reqwest connection pool used to forward requests upstream.
///
/// Connections to backends are kept alive and reused across requests. Tuning
/// these values lets you trade off memory/file-descriptor usage against the
/// latency cost of establishing new TCP connections.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionPoolConfig {
    /// Maximum number of idle connections to keep open per backend host.
    /// Lowering this reduces file-descriptor usage; raising it improves
    /// throughput under bursty traffic by avoiding repeated TCP handshakes.
    /// Default: 32.
    #[serde(default = "default_pool_max_idle_per_host")]
    pub max_idle_per_host: usize,
    /// Seconds an idle connection is kept before being closed.
    /// Set to 0 to disable the idle timeout (connections stay open until
    /// the backend closes them).
    /// Default: 90.
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: default_pool_max_idle_per_host(),
            idle_timeout_secs: default_pool_idle_timeout_secs(),
        }
    }
}

/// Headers to strip on ingress (client → upstream) and egress (upstream → client).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SensitiveHeadersConfig {
    /// Strip these headers from incoming client requests before forwarding upstream.
    /// Use this to prevent clients from forging headers your backends trust
    /// (e.g. `x-real-ip`, `x-internal-auth`).
    #[serde(default)]
    pub strip_from_request: Vec<String>,
    /// Strip these headers from upstream responses before sending to clients.
    /// Use this to avoid leaking server implementation details
    /// (e.g. `server`, `x-powered-by`).
    #[serde(default)]
    pub strip_from_response: Vec<String>,
}

fn default_max_response_body_bytes() -> usize {
    100 * 1024 * 1024 // 100 MiB
}

fn default_pool_max_idle_per_host() -> usize {
    32
}

fn default_pool_idle_timeout_secs() -> u64 {
    90
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub request_timeout_secs: u64,
    pub max_concurrent_requests: usize,
    /// Maximum bytes the proxy will buffer when caching a response body.
    /// Non-cached (streaming) responses are not limited.
    /// Defaults to 100 MiB if omitted from config.
    #[serde(default = "default_max_response_body_bytes")]
    pub max_response_body_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub path_prefix: String,
    pub backends: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub requests_per_second: u32,
    pub burst_size: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthCheckConfig {
    pub path: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub max_capacity: u64,
    pub default_ttl: u64,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).context("could not read file to string")?;
        let converted = toml::from_str(&contents).context("failed to convert from string")?;
        let config = validate_config(converted)?;

        Ok(config)
    }
}
