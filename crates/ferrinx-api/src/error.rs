use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

use crate::dto::{ApiResponse, ErrorCode};

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Missing API key")]
    MissingApiKey,

    #[error("Invalid API key format")]
    InvalidApiKeyFormat,

    #[error("Permission denied")]
    PermissionDenied,

    #[error("Model not found")]
    ModelNotFound,

    #[error("Model not valid")]
    ModelNotValid,

    #[error("Task not found")]
    TaskNotFound,

    #[error("Task not cancellable")]
    TaskNotCancellable,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Redis unavailable")]
    RedisUnavailable,

    #[error("User not found")]
    UserNotFound,

    #[error("User already exists")]
    UserAlreadyExists,

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("API key limit exceeded")]
    ApiKeyLimitExceeded,

    #[error("Database error: {0}")]
    DatabaseError(#[from] ferrinx_db::DbError),

    #[error("Core error: {0}")]
    CoreError(#[from] ferrinx_core::CoreError),

    #[error("Storage error: {0}")]
    StorageError(#[from] ferrinx_core::StorageError),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal server error")]
    InternalError,

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("UUID parse error: {0}")]
    UuidError(#[from] uuid::Error),

    #[error("Redis error: {0}")]
    RedisError(#[from] ferrinx_common::RedisError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::InvalidApiKey
            | ApiError::MissingApiKey
            | ApiError::InvalidApiKeyFormat
            | ApiError::InvalidCredentials => (StatusCode::UNAUTHORIZED, ErrorCode::InvalidApiKey),
            ApiError::PermissionDenied => (StatusCode::FORBIDDEN, ErrorCode::PermissionDenied),
            ApiError::ModelNotFound => (StatusCode::NOT_FOUND, ErrorCode::ModelNotFound),
            ApiError::TaskNotFound => (StatusCode::NOT_FOUND, ErrorCode::TaskNotFound),
            ApiError::UserNotFound => (StatusCode::NOT_FOUND, ErrorCode::UserNotFound),
            ApiError::ModelNotValid
            | ApiError::TaskNotCancellable
            | ApiError::BadRequest(_)
            | ApiError::UserAlreadyExists
            | ApiError::ApiKeyLimitExceeded => (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
            ApiError::RateLimitExceeded => {
                (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimitExceeded)
            }
            ApiError::RedisUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ServiceUnavailable,
            ),
            ApiError::DatabaseError(_)
            | ApiError::CoreError(_)
            | ApiError::StorageError(_)
            | ApiError::InternalError
            | ApiError::JsonError(_)
            | ApiError::UuidError(_)
            | ApiError::RedisError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
            }
        };

        let body = ApiResponse::<()>::error(code, self.to_string());

        (status, Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
