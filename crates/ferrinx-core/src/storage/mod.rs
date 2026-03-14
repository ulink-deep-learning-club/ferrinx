mod local;

pub use local::LocalStorage;

use crate::error::StorageError;
use async_trait::async_trait;

#[async_trait]
pub trait ModelStorage: Send + Sync {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError>;
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError>;
    async fn delete(&self, path: &str) -> Result<(), StorageError>;
    async fn exists(&self, path: &str) -> Result<bool, StorageError>;
    async fn size(&self, path: &str) -> Result<u64, StorageError>;
}
