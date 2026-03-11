use async_trait::async_trait;
use ferrinx_common::*;

use crate::error::Result;

#[async_trait]
pub trait ModelRepository: Send + Sync {
    async fn save(&self, model: &ModelInfo) -> Result<()>;
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ModelInfo>>;
    async fn find_by_name_version(&self, name: &str, version: &str) -> Result<Option<ModelInfo>>;
    async fn list(&self, filter: &ModelFilter) -> Result<Vec<ModelInfo>>;
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool>;
    async fn update_validation_status(
        &self,
        id: &uuid::Uuid,
        is_valid: bool,
        error: Option<&str>,
    ) -> Result<()>;
    async fn exists(&self, name: &str, version: &str) -> Result<bool>;
}

#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn save(&self, task: &InferenceTask) -> Result<()>;
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<InferenceTask>>;
    async fn update_status(&self, id: &uuid::Uuid, status: TaskStatus) -> Result<()>;
    async fn set_result(
        &self,
        id: &uuid::Uuid,
        status: TaskStatus,
        outputs: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<()>;
    async fn list(&self, filter: &TaskFilter) -> Result<Vec<InferenceTask>>;
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool>;
    async fn delete_by_user(&self, user_id: &uuid::Uuid) -> Result<u64>;
    async fn delete_by_model(&self, model_id: &uuid::Uuid) -> Result<u64>;
    async fn cleanup_expired(&self, retention_days: u32, batch_size: usize) -> Result<u64>;
    async fn count_by_status(&self, status: TaskStatus) -> Result<i64>;
}

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn save(&self, key: &ApiKeyRecord) -> Result<()>;
    async fn find_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRecord>>;
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ApiKeyRecord>>;
    async fn find_by_user(&self, user_id: &uuid::Uuid) -> Result<Vec<ApiKeyRecord>>;
    async fn find_temporary_by_user(&self, user_id: &uuid::Uuid) -> Result<Vec<ApiKeyRecord>>;
    async fn update_last_used(&self, id: &uuid::Uuid) -> Result<()>;
    async fn deactivate(&self, id: &uuid::Uuid) -> Result<bool>;
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool>;
    async fn update_permissions(&self, id: &uuid::Uuid, permissions: &Permissions) -> Result<()>;
    async fn delete_by_user(&self, user_id: &uuid::Uuid) -> Result<u64>;
    async fn cleanup_expired_temp_keys(&self) -> Result<u64>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn save(&self, user: &User) -> Result<()>;
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<User>>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>>;
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool>;
    async fn list(&self, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<User>>;
    async fn count(&self) -> Result<u64>;
    async fn update(&self, id: &uuid::Uuid, updates: &UserUpdates) -> Result<()>;
    async fn exists(&self) -> Result<bool>;
}
