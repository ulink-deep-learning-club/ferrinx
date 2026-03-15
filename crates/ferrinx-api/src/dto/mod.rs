use serde::{Deserialize, Serialize};

pub use ferrinx_common::{ApiKeyDetail, ModelDetail, TaskDetail, UserDetail};

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiErrorBody>,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorCode {
    InvalidApiKey,
    PermissionDenied,
    ModelNotFound,
    TaskNotFound,
    UserNotFound,
    InvalidInput,
    RateLimitExceeded,
    ServiceUnavailable,
    NoWorkerAvailable,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::InvalidApiKey => "INVALID_API_KEY",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::ModelNotFound => "MODEL_NOT_FOUND",
            ErrorCode::TaskNotFound => "TASK_NOT_FOUND",
            ErrorCode::UserNotFound => "USER_NOT_FOUND",
            ErrorCode::InvalidInput => "INVALID_INPUT",
            ErrorCode::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            ErrorCode::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            ErrorCode::NoWorkerAvailable => "NO_WORKER_AVAILABLE",
            ErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            request_id: generate_request_id(),
            data: Some(data),
            error: None,
        }
    }

    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            request_id: generate_request_id(),
            data: None,
            error: Some(ApiErrorBody {
                code: code.as_str().to_string(),
                message: message.into(),
            }),
        }
    }
}

fn generate_request_id() -> String {
    format!("req-{}", uuid::Uuid::new_v4())
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub database: bool,
    pub redis: bool,
    pub engine: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub api_key: String,
    pub user_id: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BootstrapResponse {
    pub user_id: String,
    pub username: String,
    pub password: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub permissions: Option<ferrinx_common::Permissions>,
    #[serde(default)]
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub key_id: String,
    pub key: String,
    pub name: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub permissions: Option<ferrinx_common::Permissions>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
}
