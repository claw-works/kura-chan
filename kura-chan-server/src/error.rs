use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("harness error: {0}")]
    Harness(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Auth(_) => StatusCode::UNAUTHORIZED,
            AppError::Protocol(_) => StatusCode::BAD_REQUEST,
            AppError::Session(_) => StatusCode::CONFLICT,
            AppError::Harness(_) => StatusCode::BAD_GATEWAY,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
