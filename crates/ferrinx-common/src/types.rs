use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Cached,
    Available,
}

impl ModelState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelState::Cached => "cached",
            ModelState::Available => "available",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cached" => Some(ModelState::Cached),
            "available" => Some(ModelState::Available),
            _ => None,
        }
    }

    pub fn priority_score(&self) -> i64 {
        match self {
            ModelState::Cached => 0,
            ModelState::Available => 1,
        }
    }
}

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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ModelInfo {
    pub fn unique_key(&self) -> String {
        format!("{}:{}", self.name, self.version)
    }

    pub fn is_valid(&self) -> bool {
        self.metadata.is_some() && self.input_shapes.is_some()
    }

    pub fn has_config(&self) -> bool {
        self.metadata.is_some()
    }

    pub fn validation_error(&self) -> Option<String> {
        if self.input_shapes.is_none() {
            return Some("Model failed validation".to_string());
        }
        if self.metadata.is_none() {
            return Some("Missing preprocessing config".to_string());
        }
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn test_model_state_as_str() {
        assert_eq!(ModelState::Cached.as_str(), "cached");
        assert_eq!(ModelState::Available.as_str(), "available");
    }

    #[test]
    fn test_model_state_from_str() {
        assert_eq!(ModelState::from_str("cached"), Some(ModelState::Cached));
        assert_eq!(
            ModelState::from_str("available"),
            Some(ModelState::Available)
        );
        assert_eq!(ModelState::from_str("invalid"), None);
    }

    #[test]
    fn test_model_state_priority_score() {
        assert_eq!(ModelState::Cached.priority_score(), 0);
        assert_eq!(ModelState::Available.priority_score(), 1);
    }

    #[test]
    fn test_permissions_user_default() {
        let perms = Permissions::user_default();
        assert!(perms.can_read_models());
        assert!(!perms.can_write_models());
        assert!(!perms.can_delete_models());
        assert!(perms.can_execute_inference());
        assert!(!perms.admin);
    }

    #[test]
    fn test_permissions_admin_default() {
        let perms = Permissions::admin_default();
        assert!(perms.can_read_models());
        assert!(perms.can_write_models());
        assert!(perms.can_delete_models());
        assert!(perms.can_execute_inference());
        assert!(perms.admin);
    }

    #[test]
    fn test_permissions_can_methods_with_admin() {
        let mut perms = Permissions::default();
        perms.admin = true;
        assert!(perms.can_read_models());
        assert!(perms.can_write_models());
        assert!(perms.can_delete_models());
        assert!(perms.can_execute_inference());
    }

    #[test]
    fn test_permissions_can_methods_without_admin() {
        let mut perms = Permissions::default();
        perms.models = vec!["read".to_string()];
        perms.inference = vec!["execute".to_string()];
        assert!(perms.can_read_models());
        assert!(!perms.can_write_models());
        assert!(!perms.can_delete_models());
        assert!(perms.can_execute_inference());
    }

    #[test]
    fn test_api_key_record_is_expired_no_expiry() {
        let key = ApiKeyRecord {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            key_hash: "hash".to_string(),
            name: "test".to_string(),
            permissions: Permissions::default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!key.is_expired());
    }

    #[test]
    fn test_api_key_record_is_expired_future() {
        let key = ApiKeyRecord {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            key_hash: "hash".to_string(),
            name: "test".to_string(),
            permissions: Permissions::default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!key.is_expired());
    }

    #[test]
    fn test_api_key_record_is_expired_past() {
        let key = ApiKeyRecord {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            key_hash: "hash".to_string(),
            name: "test".to_string(),
            permissions: Permissions::default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: Some(Utc::now() - Duration::hours(1)),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(key.is_expired());
    }

    #[test]
    fn test_api_key_record_is_valid() {
        let mut key = ApiKeyRecord {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            key_hash: "hash".to_string(),
            name: "test".to_string(),
            permissions: Permissions::default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(key.is_valid());

        key.is_active = false;
        assert!(!key.is_valid());

        key.is_active = true;
        key.expires_at = Some(Utc::now() - Duration::hours(1));
        assert!(!key.is_valid());
    }

    #[test]
    fn test_task_status_as_str() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::Running.as_str(), "running");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");
        assert_eq!(TaskStatus::Failed.as_str(), "failed");
        assert_eq!(TaskStatus::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn test_task_status_from_str() {
        assert_eq!(TaskStatus::from_str("pending"), Some(TaskStatus::Pending));
        assert_eq!(TaskStatus::from_str("running"), Some(TaskStatus::Running));
        assert_eq!(
            TaskStatus::from_str("completed"),
            Some(TaskStatus::Completed)
        );
        assert_eq!(TaskStatus::from_str("failed"), Some(TaskStatus::Failed));
        assert_eq!(
            TaskStatus::from_str("cancelled"),
            Some(TaskStatus::Cancelled)
        );
        assert_eq!(TaskStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_task_status_is_terminal() {
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn test_task_priority_as_i32() {
        assert_eq!(TaskPriority::Low.as_i32(), 1);
        assert_eq!(TaskPriority::Normal.as_i32(), 5);
        assert_eq!(TaskPriority::High.as_i32(), 10);
    }

    #[test]
    fn test_task_priority_from_i32() {
        assert_eq!(TaskPriority::from_i32(1), TaskPriority::Low);
        assert_eq!(TaskPriority::from_i32(2), TaskPriority::Low);
        assert_eq!(TaskPriority::from_i32(3), TaskPriority::Low);
        assert_eq!(TaskPriority::from_i32(4), TaskPriority::Normal);
        assert_eq!(TaskPriority::from_i32(5), TaskPriority::Normal);
        assert_eq!(TaskPriority::from_i32(7), TaskPriority::Normal);
        assert_eq!(TaskPriority::from_i32(8), TaskPriority::High);
        assert_eq!(TaskPriority::from_i32(10), TaskPriority::High);
        assert_eq!(TaskPriority::from_i32(100), TaskPriority::High);
    }

    #[test]
    fn test_task_priority_default() {
        assert_eq!(TaskPriority::default(), TaskPriority::Normal);
    }

    #[test]
    fn test_inference_task_priority_enum() {
        let mut task = InferenceTask {
            id: Uuid::nil(),
            model_id: Uuid::nil(),
            user_id: Uuid::nil(),
            api_key_id: Uuid::nil(),
            status: TaskStatus::Pending,
            inputs: serde_json::json!({}),
            outputs: None,
            error_message: None,
            priority: 5,
            retry_count: 0,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        assert_eq!(task.priority_enum(), TaskPriority::Normal);

        task.priority = 1;
        assert_eq!(task.priority_enum(), TaskPriority::Low);

        task.priority = 10;
        assert_eq!(task.priority_enum(), TaskPriority::High);
    }

    #[test]
    fn test_inference_task_latency_ms() {
        let mut task = InferenceTask {
            id: Uuid::nil(),
            model_id: Uuid::nil(),
            user_id: Uuid::nil(),
            api_key_id: Uuid::nil(),
            status: TaskStatus::Pending,
            inputs: serde_json::json!({}),
            outputs: None,
            error_message: None,
            priority: 5,
            retry_count: 0,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        assert!(task.latency_ms().is_none());

        task.started_at = Some(Utc::now());
        assert!(task.latency_ms().is_none());

        task.completed_at = Some(Utc::now() + Duration::milliseconds(100));
        let latency = task.latency_ms().unwrap();
        assert!(latency >= 100 && latency <= 110);
    }

    #[test]
    fn test_api_key_info_from_record() {
        let record = ApiKeyRecord {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            key_hash: "hash".to_string(),
            name: "test".to_string(),
            permissions: Permissions::user_default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: ApiKeyInfo = record.into();
        assert_eq!(info.id, Uuid::nil());
        assert_eq!(info.user_id, Uuid::nil());
        assert_eq!(info.name, "test");
        assert!(info.is_active);
        assert!(!info.is_temporary);
    }

    #[test]
    fn test_api_key_info_is_valid() {
        let mut info = ApiKeyInfo {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            permissions: Permissions::default(),
            is_active: true,
            is_temporary: false,
            expires_at: None,
        };
        assert!(info.is_valid());

        info.is_active = false;
        assert!(!info.is_valid());

        info.is_active = true;
        info.expires_at = Some(Utc::now() - Duration::hours(1));
        assert!(!info.is_valid());
    }

    #[test]
    fn test_model_info_unique_key() {
        let model = ModelInfo {
            id: Uuid::nil(),
            name: "resnet".to_string(),
            version: "1.0".to_string(),
            file_path: "/path/to/model.onnx".to_string(),
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: None,
            output_shapes: None,
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(model.unique_key(), "resnet:1.0");
    }

    #[test]
    fn test_model_info_is_valid() {
        let mut model = ModelInfo {
            id: Uuid::nil(),
            name: "resnet".to_string(),
            version: "1.0".to_string(),
            file_path: "/path/to/model.onnx".to_string(),
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!([])),
            output_shapes: None,
            metadata: Some(serde_json::json!({})),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(model.is_valid());

        model.metadata = None;
        assert!(!model.is_valid());

        model.metadata = Some(serde_json::json!({}));
        model.input_shapes = None;
        assert!(!model.is_valid());
    }

    #[test]
    fn test_model_info_validation_error() {
        let mut model = ModelInfo {
            id: Uuid::nil(),
            name: "resnet".to_string(),
            version: "1.0".to_string(),
            file_path: "/path/to/model.onnx".to_string(),
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!([])),
            output_shapes: None,
            metadata: Some(serde_json::json!({})),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(model.validation_error().is_none());

        model.metadata = None;
        assert_eq!(
            model.validation_error(),
            Some("Missing preprocessing config".to_string())
        );

        model.input_shapes = None;
        assert_eq!(
            model.validation_error(),
            Some("Model failed validation".to_string())
        );

        model.metadata = Some(serde_json::json!({}));
        assert_eq!(
            model.validation_error(),
            Some("Model failed validation".to_string())
        );
    }

    #[test]
    fn test_model_info_has_config() {
        let mut model = ModelInfo {
            id: Uuid::nil(),
            name: "resnet".to_string(),
            version: "1.0".to_string(),
            file_path: "/path/to/model.onnx".to_string(),
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: None,
            output_shapes: None,
            metadata: Some(serde_json::json!({})),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(model.has_config());

        model.metadata = None;
        assert!(!model.has_config());
    }
}
