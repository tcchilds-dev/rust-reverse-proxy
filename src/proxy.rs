pub mod headers;

use anyhow::Result;
use axum::{
    extract::{Request, State},
    response::Response,
};
use reqwest::Url;

use crate::{
    error::ProxyError,
    proxy::headers::{handle_request_headers, handle_response_headers},
    state::{AppState, Route},
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

    let guard = route.balancer.pick();

    let headers = handle_request_headers(parts.headers, &guard.url);

    let url = build_url(&guard.url, path, query).expect("Valid path and backend should not fail.");

    tracing::debug!(%url, "forwarding request");

    let result = state
        .client
        .request(method, url)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(ProxyError::UpstreamUnreachable)?;

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
