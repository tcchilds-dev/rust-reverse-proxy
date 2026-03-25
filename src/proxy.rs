pub mod headers;

use std::{net::SocketAddr, time::Instant};

use anyhow::Result;
use axum::{
    Extension,
    extract::{ConnectInfo, Request, State},
    response::Response,
};
use metrics::{counter, histogram};
use reqwest::Url;

use crate::{
    error::ProxyError,
    proxy::headers::{handle_request_headers, handle_response_headers},
    state::{AppState, Route, Scheme},
};

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

#[tracing::instrument(skip(state))]
pub async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(Scheme(scheme)): Extension<Scheme>,
    request: Request,
) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    let query = parts.uri.query();
    let method = parts.method;

    let body = reqwest::Body::wrap_stream(body.into_data_stream());

    let Some(route) = find_matching_route(path, &state.routes) else {
        return Err(ProxyError::NoRouteFound(path.to_string()));
    };

    let Some(guard) = route.balancer.pick() else {
        return Err(ProxyError::NoHealthyBackend);
    };

    let headers = handle_request_headers(parts.headers, &guard.url, client_addr.ip(), scheme);

    let url = build_url(&guard.url, path, query).expect("Valid path and backend should not fail.");

    tracing::debug!(%url, "forwarding request");

    let upstream_timer_start = Instant::now();

    let result = state
        .client
        .request(method, url)
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

    let mut response_builder = axum::response::Response::builder().status(result.status());

    for (name, value) in result.headers() {
        response_builder = response_builder.header(name, value);
    }

    let body_stream = result.bytes_stream();

    let response = response_builder
        .body(axum::body::Body::from_stream(body_stream))
        .expect("Response builder should not fail with valid status and headers.");

    Ok(handle_response_headers(response))
}
