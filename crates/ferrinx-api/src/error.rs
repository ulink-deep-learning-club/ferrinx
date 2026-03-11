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

    #[error("No worker available for this model")]
    NoWorkerAvailable,

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
    RedisError(String),
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
            ApiError::NoWorkerAvailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NoWorkerAvailable,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_api_key_response() {
        let error = ApiError::InvalidApiKey;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_missing_api_key_response() {
        let error = ApiError::MissingApiKey;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_invalid_api_key_format_response() {
        let error = ApiError::InvalidApiKeyFormat;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_permission_denied_response() {
        let error = ApiError::PermissionDenied;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_model_not_found_response() {
        let error = ApiError::ModelNotFound;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_task_not_found_response() {
        let error = ApiError::TaskNotFound;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_user_not_found_response() {
        let error = ApiError::UserNotFound;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_model_not_valid_response() {
        let error = ApiError::ModelNotValid;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_task_not_cancellable_response() {
        let error = ApiError::TaskNotCancellable;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_user_already_exists_response() {
        let error = ApiError::UserAlreadyExists;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_api_key_limit_exceeded_response() {
        let error = ApiError::ApiKeyLimitExceeded;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_bad_request_response() {
        let error = ApiError::BadRequest("Invalid input".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_rate_limit_exceeded_response() {
        let error = ApiError::RateLimitExceeded;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_redis_unavailable_response() {
        let error = ApiError::RedisUnavailable;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_no_worker_available_response() {
        let error = ApiError::NoWorkerAvailable;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_internal_error_response() {
        let error = ApiError::InternalError;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_redis_error_response() {
        let error = ApiError::RedisError("Connection refused".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_error_display() {
        assert_eq!(format!("{}", ApiError::InvalidApiKey), "Invalid API key");
        assert_eq!(format!("{}", ApiError::MissingApiKey), "Missing API key");
        assert_eq!(
            format!("{}", ApiError::PermissionDenied),
            "Permission denied"
        );
        assert_eq!(format!("{}", ApiError::ModelNotFound), "Model not found");
        assert_eq!(format!("{}", ApiError::TaskNotFound), "Task not found");
        assert_eq!(format!("{}", ApiError::UserNotFound), "User not found");
        assert_eq!(
            format!("{}", ApiError::InvalidCredentials),
            "Invalid credentials"
        );
    }

    #[test]
    fn test_bad_request_custom_message() {
        let error = ApiError::BadRequest("Custom error message".to_string());
        assert_eq!(format!("{}", error), "Bad request: Custom error message");
    }

    #[test]
    fn test_uuid_error_conversion() {
        let uuid_result = uuid::Uuid::parse_str("invalid-uuid");
        assert!(uuid_result.is_err());
        let api_error: ApiError = uuid_result.unwrap_err().into();
        assert!(matches!(api_error, ApiError::UuidError(_)));
    }

    #[test]
    fn test_json_error_conversion() {
        let json_result: std::result::Result<serde_json::Value, _> =
            serde_json::from_str("invalid json");
        assert!(json_result.is_err());
        let api_error: ApiError = json_result.unwrap_err().into();
        assert!(matches!(api_error, ApiError::JsonError(_)));
    }
}
