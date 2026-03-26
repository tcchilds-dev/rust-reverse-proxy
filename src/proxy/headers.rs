//! HTTP header manipulation for proxied requests and responses.
//!
//! Strips hop-by-hop headers (RFC 2616 §13.5.1) that must not be forwarded,
//! and injects `X-Forwarded-*` headers so backends can see the original client info.

use std::net::IpAddr;

use axum::{
    http::{HeaderMap, HeaderValue},
    response::Response,
};

/// Prepares request headers for forwarding to the upstream backend.
///
/// - Strips hop-by-hop headers that are meaningful only for a single connection.
/// - Sets `Host` to the backend address.
/// - Injects/appends `X-Forwarded-For`, `X-Forwarded-Host`, and `X-Forwarded-Proto`.
pub fn handle_request_headers(
    mut headers: HeaderMap,
    backend: &str,
    client_ip: IpAddr,
    scheme: &str,
) -> HeaderMap {
    let forwarded_host = headers.get("host").cloned();

    // Extract the authority (host:port) from the full backend URL.
    // The Host header must not include the scheme.
    let authority = reqwest::Url::parse(backend)
        .expect("Backend URL should be validated at config load")
        .authority()
        .to_string();
    headers.insert(
        "host",
        HeaderValue::from_str(&authority).expect("Authority should be valid header value"),
    );
    headers.remove("connection");
    headers.remove("keep-alive");
    headers.remove("proxy-authenticate");
    headers.remove("proxy-authorization");
    headers.remove("te");
    headers.remove("trailers");
    headers.remove("transfer-encoding");
    headers.remove("upgrade");

    if let Some(host) = forwarded_host {
        headers.insert("x-forwarded-host", host);
    };

    // Append the immediate client IP to the chain. When the proxy sits behind
    // another proxy, the existing value is preserved and extended.
    let forwarded_for = match headers.get("x-forwarded-for") {
        Some(existing) => format!(
            "{}, {client_ip}",
            existing
                .to_str()
                .expect("x-forwarded-for should be valid ASCII")
        ),
        None => client_ip.to_string(),
    };
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_str(&forwarded_for)
            .expect("existing x-forwarded-for header should be valid UTF-8"),
    );

    if let Ok(forwarded_proto) = HeaderValue::from_str(scheme) {
        headers.insert("x-forwarded-proto", forwarded_proto);
    }

    headers
}

/// Strips hop-by-hop headers from the upstream response before sending to the client.
pub fn handle_response_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();

    headers.remove("connection");
    headers.remove("keep-alive");
    headers.remove("proxy-authenticate");
    headers.remove("proxy-authorization");
    headers.remove("te");
    headers.remove("trailers");
    headers.remove("transfer-encoding");
    headers.remove("upgrade");

    response
}
