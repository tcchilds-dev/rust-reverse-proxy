//! Tower middleware that emits one structured log line per request.
//!
//! Logs method, path, status code, latency, client IP, and upstream backend.
//! Sits at an outer layer in the middleware stack so it captures every
//! request, including those rejected by rate limiting or concurrency limits
//! before they reach the proxy handler.
//!
//! The upstream backend URL is communicated from [`proxy_handler`] via the
//! [`UpstreamInfo`] response extension. Requests that never reach the handler
//! (or that were served from cache) log upstream as `"-"`.

use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response},
};
use futures::future::BoxFuture;
use tower::{Layer, Service};

/// Response extension set by [`proxy_handler`](crate::proxy::proxy_handler)
/// to record which upstream backend handled the request.
#[derive(Clone)]
pub struct UpstreamInfo(pub String);

/// Tower [`Layer`] that wraps every request with access logging.
#[derive(Clone)]
pub struct AccessLogLayer;

impl<S> Layer<S> for AccessLogLayer {
    type Service = AccessLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AccessLogService { inner }
    }
}

#[derive(Clone)]
pub struct AccessLogService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for AccessLogService<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let query = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();

        let client_ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .unwrap_or(IpAddr::from([0, 0, 0, 0]));

        // Standard Tower clone-and-swap pattern for 'static futures.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let start = Instant::now();
            let result = inner.call(req).await;
            let latency_ms = start.elapsed().as_millis();

            let (status, upstream) = match &result {
                Ok(response) => {
                    let status = response.status().as_u16().to_string();
                    let upstream = response
                        .extensions()
                        .get::<UpstreamInfo>()
                        .map(|u| u.0.clone())
                        .unwrap_or_else(|| "-".to_string());
                    (status, upstream)
                }
                Err(_) => ("error".to_string(), "-".to_string()),
            };

            tracing::info!(
                target: "access_log",
                %method,
                path = %format!("{path}{query}"),
                %status,
                %latency_ms,
                %client_ip,
                %upstream,
            );

            result
        })
    }
}
