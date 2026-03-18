use std::{collections::HashMap, net::SocketAddr};

use axum::{Json, Router, body::Body, extract::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};

pub async fn run(name: &str, port: u16) {
    let name = name.to_string();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Backend {name} listening on {addr}");

    let router = Router::new().fallback(move |req: Request<Body>| {
        let name = name.clone();
        async move { echo_handler(name, req).await }
    });

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}

async fn echo_handler(name: String, req: Request<Body>) -> Json<Value> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();

    let body_bytes = BodyExt::collect(req.into_body())
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    Json(json!({
        "backend": name,
        "method": method,
        "path": path,
        "query": query,
        "headers": headers,
        "body": body,
    }))
}
