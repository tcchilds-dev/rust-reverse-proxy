use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;

pub struct CachedResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub ttl: Duration,
}

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

pub fn should_cache(method: &Method, status: StatusCode, headers: &HeaderMap) -> bool {
    is_cacheable_method(method) && is_cacheable_status(status) && is_cacheable_by_headers(headers)
}

fn is_cacheable_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
}

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

    // excluding for simplicity's sake
    if directives.contains(&"no-cache") {
        return false;
    }

    true
}

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

pub fn extract_ttl(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_max_age)
}
