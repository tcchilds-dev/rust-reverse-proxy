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
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub request_timeout_secs: u64,
    pub max_concurrent_requests: usize,
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

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).context("could not read file to string")?;
        let converted = toml::from_str(&contents).context("failed to convert from string")?;
        let config = validate_config(converted)?;

        Ok(config)
    }
}
