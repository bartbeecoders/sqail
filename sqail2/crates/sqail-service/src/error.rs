//! API errors rendered as `application/problem+json`.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqail_proto::Problem;

use crate::engine::DbError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub detail: Option<String>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, detail: impl Into<Option<String>>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid bearer token is required".to_string(),
        )
    }

    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", detail.into())
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", detail.into())
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", detail.into())
    }

    pub fn too_many_requests() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "slow down".to_string(),
        )
    }

    /// Log the real cause; tell the client nothing about internals.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(format!("{e:#}"))
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        let status = match &e {
            DbError::Database { .. } | DbError::Invalid(_) | DbError::Unsupported(_) => {
                StatusCode::BAD_REQUEST
            }
            DbError::Connect(_) => StatusCode::BAD_GATEWAY,
            DbError::Cancelled | DbError::ClientGone => StatusCode::CONFLICT,
            DbError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            DbError::Internal(_) => return Self::internal(e),
        };
        Self::new(status, e.code(), e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Problem {
            title: self
                .status
                .canonical_reason()
                .unwrap_or("Error")
                .to_string(),
            status: self.status.as_u16(),
            code: self.code.to_string(),
            detail: self.detail,
        };
        let mut res = (self.status, axum::Json(body)).into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        res
    }
}
