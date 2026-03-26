//! Per-IP rate limiting implemented as a Tower [`Layer`]/[`Service`].
//!
//! Uses the [`governor`] crate's token-bucket algorithm keyed by client IP.
//! Requests that exceed the limit receive a `429 Too Many Requests` response
//! with a `Retry-After` header.

use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response},
};
use futures::future::BoxFuture;
use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock},
    state::keyed::DefaultKeyedStateStore,
};
use reqwest::StatusCode;
use tower::{Layer, Service};

type KeyedLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: Arc<KeyedLimiter>,
}

impl RateLimitLayer {
    pub fn new(per_second: u32, burst_size: u32) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(per_second).expect("rate must be non-zero"))
            .allow_burst(NonZeroU32::new(burst_size).expect("burst must be non-zero"));

        Self {
            limiter: Arc::new(RateLimiter::keyed(quota)),
        }
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: Arc::clone(&self.limiter),
        }
    }
}

#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: Arc<KeyedLimiter>,
}

impl<S, ReqBody> Service<Request<ReqBody>> for RateLimitService<S>
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
        // Falls back to 0.0.0.0 if ConnectInfo is missing (e.g. in unit tests
        // without `into_make_service_with_connect_info`).
        let ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .unwrap_or(IpAddr::from([0, 0, 0, 0]));

        match self.limiter.check_key(&ip) {
            Ok(_) => {
                let future = self.inner.call(req);
                Box::pin(future)
            }
            Err(rejected) => {
                let wait = rejected.wait_time_from(DefaultClock::default().now());
                let retry_after = wait.as_secs().max(1).to_string();

                let response = Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .header("Retry-After", retry_after)
                    .body(Body::from("rate limit exceeded"))
                    .unwrap();

                Box::pin(async move { Ok(response) })
            }
        }
    }
}
