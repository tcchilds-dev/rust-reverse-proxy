//! Response caching layer with per-entry TTL.
//!
//! Caching decisions follow a simplified HTTP caching model: only GET/HEAD
//! responses with cacheable status codes and permissive `Cache-Control` headers
//! are stored. TTL is derived from `max-age` when present, falling back to a
//! configurable default.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;

/// A fully buffered upstream response, stored in the Moka cache.
pub struct CachedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    /// Per-entry TTL extracted from `Cache-Control: max-age`, or the global default.
    pub ttl: Duration,
}

/// Moka [`Expiry`] implementation that gives each cache entry its own TTL
/// rather than using a single global expiration for all entries.
pub struct ResponseExpiry {
    pub default_ttl: Duration,
}

impl moka::Expiry<String, Arc<CachedResponse>> for ResponseExpiry {
    fn expire_after_create(
        &self,
        _key: &String,
        value: &Arc<CachedResponse>,
        _created_at: std::time::Instant,
    ) -> Option<Duration> {
        Some(value.ttl)
    }
}

/// Determines whether a response is eligible for caching based on method,
/// status code, and `Cache-Control` directives.
pub fn should_cache(method: &Method, status: StatusCode, headers: &HeaderMap) -> bool {
    is_cacheable_method(method) && is_cacheable_status(status) && is_cacheable_by_headers(headers)
}

fn is_cacheable_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
}

/// Cacheable status codes per RFC 7231 §6.1 (defaults that are cacheable
/// without explicit cache headers).
fn is_cacheable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::OK
            | StatusCode::NON_AUTHORITATIVE_INFORMATION
            | StatusCode::NO_CONTENT
            | StatusCode::PARTIAL_CONTENT
            | StatusCode::MULTIPLE_CHOICES
            | StatusCode::MOVED_PERMANENTLY
            | StatusCode::NOT_FOUND
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::GONE
            | StatusCode::URI_TOO_LONG
            | StatusCode::NOT_IMPLEMENTED
    )
}

fn is_cacheable_by_headers(headers: &HeaderMap) -> bool {
    let cache_control = headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(age) = parse_max_age(cache_control)
        && age == Duration::from_secs(0)
    {
        return false;
    }

    let directives: Vec<&str> = cache_control.split(',').map(|d| d.trim()).collect();

    if directives.contains(&"no-store") {
        return false;
    }

    if directives.contains(&"private") {
        return false;
    }

    // Excluded for simplicity's sake.
    if directives.contains(&"no-cache") {
        return false;
    }

    true
}

/// Extracts the `max-age` value (in seconds) from a `Cache-Control` header string.
pub fn parse_max_age(header_value: &str) -> Option<Duration> {
    header_value
        .split(',')
        .map(|d| d.trim())
        .find_map(|directive| {
            directive
                .strip_prefix("max-age=")
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs)
        })
}

/// Reads the TTL for a response from its `Cache-Control: max-age` directive.
/// Returns `None` if the header is absent or unparseable, letting the caller
/// fall back to a default TTL.
pub fn extract_ttl(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_max_age)
}
