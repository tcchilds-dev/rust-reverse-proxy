//! Core proxy handler: route matching, upstream forwarding, caching, and metrics.

pub mod headers;

use std::{net::SocketAddr, sync::Arc, time::Instant};

use anyhow::Result;
use axum::{
    Extension,
    extract::{ConnectInfo, Request, State},
    http::HeaderValue,
    response::Response,
};
use metrics::{counter, histogram};
use reqwest::Url;

use crate::{
    access_log::UpstreamInfo,
    cache::{CachedResponse, extract_ttl, should_cache},
    error::ProxyError,
    metrics::RouteInfo,
    proxy::headers::{handle_request_headers, handle_response_headers},
    state::{AppState, Route, Scheme},
};

/// Returns the first route whose `path_prefix` matches the start of `path`.
///
/// Routes are checked in config order, so more-specific prefixes should appear
/// first in the configuration file (there is no longest-prefix-match).
fn find_matching_route<'a>(path: &str, routes: &'a [Route]) -> Option<&'a Route> {
    routes.iter().find(|r| path.starts_with(&r.path_prefix))
}

fn build_url(backend: &str, path: &str, query: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(backend)?;
    url.set_path(path);
    if let Some(query) = query {
        url.set_query(Some(query));
    }

    Ok(url)
}

fn build_response_from_cache(cached_response: &Arc<CachedResponse>) -> Response {
    let mut response_builder = axum::response::Response::builder().status(cached_response.status);

    for (name, value) in &cached_response.headers {
        response_builder = response_builder.header(name, value);
    }

    response_builder
        .body(axum::body::Body::from(cached_response.body.clone()))
        .expect("Response builder should not fail with valid status and headers.")
}

// skip_all: recording `request` would log full headers, including
// Authorization and Cookie values, at debug level.
#[tracing::instrument(
    skip_all,
    fields(method = %request.method(), uri = %request.uri(), client = %client_addr)
)]
pub async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(Scheme(scheme)): Extension<Scheme>,
    request: Request,
) -> Result<Response, ProxyError> {
    // Authenticated requests always bypass the cache to avoid serving
    // one user's personalized response to another. Cookies count as
    // authentication: session cookies personalize responses just like
    // Authorization headers do.
    let is_authenticated = request.headers().contains_key("authorization")
        || request.headers().contains_key("cookie");
    // Include the method in the cache key so HEAD and GET responses are stored
    // separately, and non-GET/HEAD requests never match a cached entry.
    let cache_key = format!("{}:{}", request.method(), request.uri());

    // If the client supplied X-Request-ID, propagate it for end-to-end correlation.
    // Otherwise generate a fresh 128-bit random hex ID so every request is traceable.
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));

    if !is_authenticated && let Some(cached) = state.cache.get(&cache_key).await {
        let mut response = handle_response_headers(build_response_from_cache(&cached));
        for name in &state.strip_response_headers {
            response.headers_mut().remove(name);
        }
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&request_id).expect("request ID is always a valid header value"),
        );
        return Ok(response);
    }

    let (mut parts, body) = request.into_parts();
    let path = parts.uri.path();
    let query = parts.uri.query();
    let method = parts.method;

    // Bridge axum's body type into reqwest's body type via a byte stream.
    let body = reqwest::Body::wrap_stream(body.into_data_stream());

    let Some(route) = find_matching_route(path, &state.routes) else {
        return Err(ProxyError::NoRouteFound(path.to_string()));
    };

    let Some(guard) = route.balancer.pick() else {
        return Err(ProxyError::NoHealthyBackend);
    };

    // Strip configured sensitive headers before forwarding so clients cannot
    // inject headers that backends might trust (e.g. x-real-ip, x-internal-auth).
    for name in &state.strip_request_headers {
        parts.headers.remove(name);
    }

    let headers = handle_request_headers(parts.headers, &guard.url, client_addr.ip(), scheme, &request_id);

    let url = build_url(&guard.url, path, query).expect("Valid path and backend should not fail.");

    tracing::debug!(%url, "forwarding request");

    let upstream_timer_start = Instant::now();

    let result = state
        .client
        .request(method.clone(), url)
        .headers(headers)
        .body(body)
        .send()
        .await;

    if let Err(ref e) = result {
        let kind = if e.is_timeout() {
            "timeout"
        } else {
            "connection"
        };
        counter!("proxy_upstream_errors_total", "upstream" => guard.url.clone(), "kind" => kind)
            .increment(1);
    }

    let result = result.map_err(ProxyError::UpstreamUnreachable)?;

    let upstream_timer_elapsed = upstream_timer_start.elapsed().as_secs_f64();

    histogram!("proxy_upstream_response_seconds", "upstream" => guard.url.clone())
        .record(upstream_timer_elapsed);

    let status_class = match result.status().as_u16() {
        200..=299 => "2xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    };

    counter!("proxy_upstream_requests_total", "upstream" => guard.url.clone(), "status" => status_class)
        .increment(1);

    tracing::debug!(status = %result.status(), "upstream response");

    let status = result.status();
    // If the upstream declares a Content-Length over the cache limit, skip the
    // caching path entirely so the body streams through without being buffered.
    let declared_too_large = result
        .content_length()
        .is_some_and(|len| len as usize > state.max_response_body_bytes);
    let should_cache_this =
        !is_authenticated && !declared_too_large && should_cache(&method, status, result.headers());

    let mut response_builder = axum::response::Response::builder().status(status);
    for (name, value) in result.headers() {
        response_builder = response_builder.header(name, value);
    }

    // When caching, we must buffer the entire body to store it. Otherwise,
    // stream the response directly to avoid holding large bodies in memory.
    let response = if should_cache_this {
        let headers_for_cache = result.headers().clone();
        let ttl = extract_ttl(&headers_for_cache).unwrap_or(state.default_ttl);
        let body_bytes = result
            .bytes()
            .await
            .map_err(ProxyError::UpstreamUnreachable)?;

        // The buffered body may exceed the limit even without a Content-Length
        // header; in that case it is still returned to the client, just not cached.
        if body_bytes.len() <= state.max_response_body_bytes {
            state
                .cache
                .insert(
                    cache_key,
                    Arc::new(CachedResponse {
                        status,
                        headers: headers_for_cache,
                        body: body_bytes.clone(),
                        ttl,
                    }),
                )
                .await;
        }

        response_builder
            .body(axum::body::Body::from(body_bytes))
            .expect("Response builder should not fail with valid status and headers.")
    } else {
        response_builder
            .body(axum::body::Body::from_stream(result.bytes_stream()))
            .expect("Response builder should not fail with valid status and headers.")
    };

    let mut response = handle_response_headers(response);
    for name in &state.strip_response_headers {
        response.headers_mut().remove(name);
    }
    response.extensions_mut().insert(UpstreamInfo(guard.url.clone()));
    response.extensions_mut().insert(RouteInfo(route.path_prefix.clone()));
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("request ID is always a valid header value"),
    );
    Ok(response)
}
