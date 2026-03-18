use std::net::IpAddr;

use axum::{
    http::{HeaderMap, HeaderValue},
    response::Response,
};

pub fn handle_request_headers(
    mut headers: HeaderMap,
    backend: &str,
    client_ip: IpAddr,
    scheme: &str,
) -> HeaderMap {
    let forwarded_host = headers.get("host").cloned();

    headers.insert(
        "host",
        HeaderValue::from_str(backend).expect("Backend should be validated."),
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
