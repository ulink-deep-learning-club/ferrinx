use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

#[derive(Debug, Serialize)]
pub struct ModelDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    pub input_shapes: Option<serde_json::Value>,
    pub output_shapes: Option<serde_json::Value>,
    pub is_valid: bool,
    pub validation_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ferrinx_common::ModelInfo> for ModelDetail {
    fn from(model: ferrinx_common::ModelInfo) -> Self {
        Self {
            id: model.id.to_string(),
            name: model.name,
            version: model.version,
            file_path: model.file_path,
            file_size: model.file_size,
            input_shapes: model.input_shapes,
            output_shapes: model.output_shapes,
            is_valid: model.is_valid,
            validation_error: model.validation_error,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApiKeyDetail {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub is_temporary: bool,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct TaskDetail {
    pub task_id: String,
    pub model_id: String,
    pub status: String,
    pub outputs: Option<HashMap<String, serde_json::Value>>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub latency_ms: Option<i64>,
}

impl From<ferrinx_common::InferenceTask> for TaskDetail {
    fn from(task: ferrinx_common::InferenceTask) -> Self {
        let latency_ms = task.latency_ms();
        let outputs = task.outputs.and_then(|v| serde_json::from_value(v).ok());

        Self {
            task_id: task.id.to_string(),
            model_id: task.model_id.to_string(),
            status: task.status.as_str().to_string(),
            outputs,
            error_message: task.error_message,
            created_at: task.created_at.to_rfc3339(),
            completed_at: task.completed_at.map(|t| t.to_rfc3339()),
            latency_ms,
        }
    }
}

impl From<ferrinx_common::ApiKeyRecord> for ApiKeyDetail {
    fn from(key: ferrinx_common::ApiKeyRecord) -> Self {
        Self {
            id: key.id.to_string(),
            name: key.name,
            is_active: key.is_active,
            is_temporary: key.is_temporary,
            last_used_at: key.last_used_at.map(|t| t.to_rfc3339()),
            expires_at: key.expires_at.map(|t| t.to_rfc3339()),
            created_at: key.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UserDetail {
    pub id: String,
    pub username: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: String,
}

impl From<ferrinx_common::User> for UserDetail {
    fn from(user: ferrinx_common::User) -> Self {
        Self {
            id: user.id.to_string(),
            username: user.username,
            role: format!("{:?}", user.role).to_lowercase(),
            is_active: user.is_active,
            created_at: user.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub permissions: Option<ferrinx_common::Permissions>,
    #[serde(default)]
    pub expires_in_hours: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub key: String,
    pub name: String,
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
