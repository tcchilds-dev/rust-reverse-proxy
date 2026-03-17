use axum::response::{IntoResponse, Response};
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("No route found for path: {0}")]
    NoRouteFound(String),
    #[error("No healthy backend available.")]
    NoHealthyBackend,
    #[error("Upstream unreachable: {0}")]
    UpstreamUnreachable(#[from] reqwest::Error),
    #[error("Request timed out.")]
    Timeout,
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = match &self {
            ProxyError::NoRouteFound(_) => StatusCode::NOT_FOUND,
            ProxyError::NoHealthyBackend => StatusCode::BAD_GATEWAY,
            ProxyError::UpstreamUnreachable(_) => StatusCode::BAD_GATEWAY,
            ProxyError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ProxyError::InvalidConfig(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (status, self.to_string()).into_response()
    }
}
