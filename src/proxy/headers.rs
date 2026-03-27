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
/// - Sets `X-Request-ID` to `request_id` (either propagated from the client or freshly generated).
pub fn handle_request_headers(
    mut headers: HeaderMap,
    backend: &str,
    client_ip: IpAddr,
    scheme: &str,
    request_id: &str,
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

    // Always replace any client-supplied X-Forwarded-For with the direct client
    // IP. Trusting and appending to a client-supplied header allows IP spoofing:
    // a client can send "X-Forwarded-For: 1.2.3.4" to make backends believe the
    // request originated elsewhere.
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_str(&client_ip.to_string())
            .expect("IP address is always a valid header value"),
    );

    if let Ok(forwarded_proto) = HeaderValue::from_str(scheme) {
        headers.insert("x-forwarded-proto", forwarded_proto);
    }

    headers.insert(
        "x-request-id",
        HeaderValue::from_str(request_id).expect("request ID is always a valid header value"),
    );

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
