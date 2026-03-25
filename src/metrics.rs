use std::{pin::Pin, time::Instant};

use axum::http::{Request, Response};
use metrics::{counter, gauge, histogram};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { inner }
    }
}

#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for MetricsService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Send + Clone + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let method = req.method().to_string();

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let start = Instant::now();

            gauge!("proxy_inflight_requests").increment(1.0);

            let result = inner.call(req).await;

            gauge!("proxy_inflight_requests").decrement(1.0);

            let elapsed = start.elapsed().as_secs_f64();

            let status = match &result {
                Ok(res) => res.status().as_u16().to_string(),
                Err(_) => "error".to_string(),
            };

            counter!(
                "proxy_requests_total",
                    "method" => method.clone(),
                    "status" => status.clone(),
            )
            .increment(1);

            histogram!("proxy_request_duration_seconds", "method" => method, "status" => status)
                .record(elapsed);

            result
        })
    }
}
