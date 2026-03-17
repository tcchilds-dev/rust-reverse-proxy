use axum::{
    http::{HeaderMap, HeaderValue},
    response::Response,
};

pub fn handle_request_headers(mut headers: HeaderMap, backend: &str) -> HeaderMap {
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
        headers.insert("x-forwarded-for", host);
    };

    // TODO: inject forwarded_for and forwarded_proto

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
