use crate::{ApiKeyRecord, InferenceTask, ModelInfo, User};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyDetail {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub is_temporary: bool,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

impl From<ApiKeyRecord> for ApiKeyDetail {
    fn from(key: ApiKeyRecord) -> Self {
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

#[derive(Debug, Serialize, Deserialize)]
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

impl From<InferenceTask> for TaskDetail {
    fn from(task: InferenceTask) -> Self {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct UserDetail {
    pub id: String,
    pub username: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: String,
}

impl From<User> for UserDetail {
    fn from(user: User) -> Self {
        Self {
            id: user.id.to_string(),
            username: user.username,
            role: format!("{:?}", user.role).to_lowercase(),
            is_active: user.is_active,
            created_at: user.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    pub input_shapes: Option<serde_json::Value>,
    pub output_shapes: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub is_valid: bool,
    pub validation_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ModelInfo> for ModelDetail {
    fn from(model: ModelInfo) -> Self {
        let is_valid = model.is_valid();
        let validation_error = model.validation_error();
        Self {
            id: model.id.to_string(),
            name: model.name,
            version: model.version,
            file_path: model.file_path,
            file_size: model.file_size,
            input_shapes: model.input_shapes,
            output_shapes: model.output_shapes,
            metadata: model.metadata,
            is_valid,
            validation_error,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}
