use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub inference: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub admin: bool,
}

impl Permissions {
    pub fn user_default() -> Self {
        Self {
            models: vec!["read".to_string()],
            inference: vec!["execute".to_string()],
            api_keys: vec!["read".to_string(), "write".to_string()],
            admin: false,
        }
    }

    pub fn admin_default() -> Self {
        Self {
            models: vec![
                "read".to_string(),
                "write".to_string(),
                "delete".to_string(),
            ],
            inference: vec!["execute".to_string()],
            api_keys: vec![
                "read".to_string(),
                "write".to_string(),
                "delete".to_string(),
            ],
            admin: true,
        }
    }

    pub fn can_read_models(&self) -> bool {
        self.models.contains(&"read".to_string()) || self.admin
    }

    pub fn can_write_models(&self) -> bool {
        self.models.contains(&"write".to_string()) || self.admin
    }

    pub fn can_delete_models(&self) -> bool {
        self.models.contains(&"delete".to_string()) || self.admin
    }

    pub fn can_execute_inference(&self) -> bool {
        self.inference.contains(&"execute".to_string()) || self.admin
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub key_hash: String,
    pub name: String,
    pub permissions: Permissions,
    pub is_active: bool,
    pub is_temporary: bool,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ApiKeyRecord {
    pub fn is_expired(&self) -> bool {
        self.expires_at.map_or(false, |exp| Utc::now() > exp)
    }

    pub fn is_valid(&self) -> bool {
        self.is_active && !self.is_expired()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    pub storage_backend: String,
    pub input_shapes: Option<serde_json::Value>,
    pub output_shapes: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub is_valid: bool,
    pub validation_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ModelInfo {
    pub fn unique_key(&self) -> String {
        format!("{}:{}", self.name, self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "running" => Some(TaskStatus::Running),
            "completed" => Some(TaskStatus::Completed),
            "failed" => Some(TaskStatus::Failed),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low = 1,
    Normal = 5,
    High = 10,
}

impl TaskPriority {
    pub fn as_i32(&self) -> i32 {
        *self as i32
    }

    pub fn from_i32(value: i32) -> Self {
        match value {
            1..=3 => TaskPriority::Low,
            4..=7 => TaskPriority::Normal,
            _ => TaskPriority::High,
        }
    }

    pub fn stream_key(&self) -> &'static str {
        match self {
            TaskPriority::High => crate::constants::REDIS_STREAM_KEY_HIGH,
            TaskPriority::Normal => crate::constants::REDIS_STREAM_KEY_NORMAL,
            TaskPriority::Low => crate::constants::REDIS_STREAM_KEY_LOW,
        }
    }
}

impl Default for TaskPriority {
    fn default() -> Self {
        TaskPriority::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceTask {
    pub id: Uuid,
    pub model_id: Uuid,
    pub user_id: Uuid,
    pub api_key_id: Uuid,
    pub status: TaskStatus,
    pub inputs: serde_json::Value,
    pub outputs: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub priority: i32,
    pub retry_count: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl InferenceTask {
    pub fn priority_enum(&self) -> TaskPriority {
        TaskPriority::from_i32(self.priority)
    }

    pub fn latency_ms(&self) -> Option<i64> {
        self.started_at.and_then(|start| {
            self.completed_at
                .map(|end| (end - start).num_milliseconds())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceInput {
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceOutput {
    pub outputs: HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub outputs: serde_json::Value,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub permissions: Permissions,
    pub is_active: bool,
    pub is_temporary: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<ApiKeyRecord> for ApiKeyInfo {
    fn from(record: ApiKeyRecord) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id,
            name: record.name,
            permissions: record.permissions,
            is_active: record.is_active,
            is_temporary: record.is_temporary,
            expires_at: record.expires_at,
        }
    }
}

impl ApiKeyInfo {
    pub fn is_expired(&self) -> bool {
        self.expires_at.map_or(false, |exp| Utc::now() > exp)
    }

    pub fn is_valid(&self) -> bool {
        self.is_active && !self.is_expired()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModelFilter {
    pub name: Option<String>,
    pub is_valid: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub user_id: Option<Uuid>,
    pub model_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct UserUpdates {
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<UserRole>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorInfo {
    pub name: String,
    pub shape: Vec<i64>,
    pub element_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub inputs: Vec<TensorInfo>,
    pub outputs: Vec<TensorInfo>,
    pub opset_version: Option<i64>,
    pub producer_name: Option<String>,
}
