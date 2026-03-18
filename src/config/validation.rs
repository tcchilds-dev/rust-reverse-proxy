use std::collections::HashSet;

use anyhow::{Result, bail};
use reqwest::Url;

use crate::config::{Config, RouteConfig};

pub fn validate_config(config: Config) -> Result<Config> {
    let prefix_result = validate_path_prefix(&config.routes);
    if !prefix_result {
        bail!("All paths must start with '/'.")
    }

    let dupe_result = validate_no_duplicate_paths(&config.routes);
    if !dupe_result {
        bail!("No duplicate paths allowed.")
    }

    let level_result = validate_logger_level(&config.logging.level);
    if !level_result {
        bail!("Logging level must be one of: debug, info, warn, error.")
    }

    let empty_result = validate_backends_not_empty(&config.routes);
    if !empty_result {
        bail!("All paths must have at least one backend.")
    }

    let urls_result = validate_backend_urls(&config.routes);
    if !urls_result {
        bail!("All backends must be valid URLs.")
    }

    Ok(config)
}

fn validate_path_prefix(routes: &[RouteConfig]) -> bool {
    routes.iter().all(|r| r.path_prefix.starts_with("/"))
}

fn validate_no_duplicate_paths(routes: &[RouteConfig]) -> bool {
    let mut set = HashSet::new();

    routes.iter().all(|r| set.insert(&r.path_prefix))
}

fn validate_logger_level(level: &str) -> bool {
    matches!(level, "debug" | "info" | "warn" | "error")
}

fn validate_backends_not_empty(routes: &[RouteConfig]) -> bool {
    routes
        .iter()
        .flat_map(|r| r.backends.iter())
        .all(|b| !b.is_empty())
}

fn validate_backend_urls(routes: &[RouteConfig]) -> bool {
    routes
        .iter()
        .flat_map(|r| r.backends.iter())
        .all(|b| Url::parse(b).is_ok())
}
