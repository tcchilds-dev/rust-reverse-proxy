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
    matches!(level.to_lowercase().as_str(), "debug" | "info" | "warn" | "error")
}

fn validate_backends_not_empty(routes: &[RouteConfig]) -> bool {
    routes
        .iter()
        .all(|r| !r.backends.is_empty() && r.backends.iter().all(|b| !b.is_empty()))
}

fn validate_backend_urls(routes: &[RouteConfig]) -> bool {
    routes
        .iter()
        .flat_map(|r| r.backends.iter())
        .all(|b| {
            Url::parse(b)
                .map(|u| matches!(u.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use crate::config::{
        CacheConfig, Config, HealthCheckConfig, LoggingConfig, RateLimitConfig, RouteConfig,
        ServerConfig,
    };

    use super::*;

    fn base_config(routes: Vec<RouteConfig>) -> Config {
        Config {
            server: ServerConfig {
                addr: "127.0.0.1:8080".parse().unwrap(),
                request_timeout_secs: 10,
                max_concurrent_requests: 100,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
            },
            routes,
            tls: None,
            rate_limiting: RateLimitConfig {
                requests_per_second: 100,
                burst_size: 200,
            },
            health_checks: HealthCheckConfig {
                path: "/healthz".to_string(),
                interval_secs: 10,
                timeout_secs: 3,
            },
            caching: CacheConfig {
                max_capacity: 100,
                default_ttl: 60,
            },
        }
    }

    fn valid_route() -> RouteConfig {
        RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec!["http://localhost:3000".to_string()],
        }
    }

    #[test]
    fn valid_config_passes() {
        let config = base_config(vec![valid_route()]);
        assert!(validate_config(config).is_ok());
    }

    #[test]
    fn path_prefix_must_start_with_slash() {
        let config = base_config(vec![RouteConfig {
            path_prefix: "api".to_string(),
            backends: vec!["http://localhost:3000".to_string()],
        }]);
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("start with '/'"));
    }

    #[test]
    fn duplicate_path_prefixes_rejected() {
        let config = base_config(vec![
            RouteConfig {
                path_prefix: "/api".to_string(),
                backends: vec!["http://localhost:3000".to_string()],
            },
            RouteConfig {
                path_prefix: "/api".to_string(),
                backends: vec!["http://localhost:3001".to_string()],
            },
        ]);
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn invalid_log_level_rejected() {
        let mut config = base_config(vec![valid_route()]);
        config.logging.level = "verbose".to_string();
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("Logging level"));
    }

    #[test]
    fn valid_log_levels_accepted() {
        for level in &["debug", "info", "warn", "error"] {
            let mut config = base_config(vec![valid_route()]);
            config.logging.level = level.to_string();
            assert!(validate_config(config).is_ok(), "level '{level}' should be valid");
        }
    }

    #[test]
    fn empty_backends_list_rejected() {
        let config = base_config(vec![RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec![],
        }]);
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("at least one backend"));
    }

    #[test]
    fn empty_backend_string_rejected() {
        let config = base_config(vec![RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec!["".to_string()],
        }]);
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("at least one backend"));
    }

    #[test]
    fn invalid_backend_url_rejected() {
        let config = base_config(vec![RouteConfig {
            path_prefix: "/api".to_string(),
            backends: vec!["not a url".to_string()],
        }]);
        let err = validate_config(config).unwrap_err();
        assert!(err.to_string().contains("valid URLs"));
    }

    #[test]
    fn multiple_valid_routes_accepted() {
        let config = base_config(vec![
            RouteConfig {
                path_prefix: "/api".to_string(),
                backends: vec!["http://localhost:3000".to_string()],
            },
            RouteConfig {
                path_prefix: "/static".to_string(),
                backends: vec![
                    "http://localhost:3001".to_string(),
                    "http://localhost:3002".to_string(),
                ],
            },
        ]);
        assert!(validate_config(config).is_ok());
    }
}
