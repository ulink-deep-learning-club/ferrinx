use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::StorageError;
use super::ModelStorage;

pub struct LocalStorage {
    base_path: PathBuf,
}

impl LocalStorage {
    pub fn new(base_path: &str) -> Result<Self, StorageError> {
        let path = PathBuf::from(base_path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { base_path: path })
    }
}

#[async_trait]
impl ModelStorage for LocalStorage {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError> {
        let filename = format!("{}.onnx", model_id);
        let path = self.base_path.join(&filename);
        tokio::fs::write(&path, data).await?;
        Ok(path.to_string_lossy().to_string())
    }

    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        tokio::fs::read(path).await.map_err(StorageError::from)
    }

    async fn delete(&self, path: &str) -> Result<(), StorageError> {
        tokio::fs::remove_file(path).await.map_err(StorageError::from)
    }

    async fn exists(&self, path: &str) -> Result<bool, StorageError> {
        Ok(tokio::fs::metadata(path).await.is_ok())
    }

    async fn size(&self, path: &str) -> Result<u64, StorageError> {
        let metadata = tokio::fs::metadata(path).await?;
        Ok(metadata.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_storage() -> (TempDir, LocalStorage) {
        let temp_dir = TempDir::new().unwrap();
        let storage = LocalStorage::new(temp_dir.path().to_str().unwrap()).unwrap();
        (temp_dir, storage)
    }

    #[tokio::test]
    async fn test_local_storage_save_load() {
        let (_temp_dir, storage) = setup_test_storage();
        
        let data = vec![1, 2, 3, 4, 5];
        let path = storage.save("test-model", &data).await.unwrap();
        
        assert!(storage.exists(&path).await.unwrap());
        
        let loaded = storage.load(&path).await.unwrap();
        assert_eq!(loaded, data);
    }

    #[tokio::test]
    async fn test_local_storage_delete() {
        let (_temp_dir, storage) = setup_test_storage();
        
        let data = vec![1, 2, 3, 4, 5];
        let path = storage.save("test-model", &data).await.unwrap();
        
        assert!(storage.exists(&path).await.unwrap());
        
        storage.delete(&path).await.unwrap();
        
        assert!(!storage.exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn test_local_storage_size() {
        let (_temp_dir, storage) = setup_test_storage();
        
        let data = vec![1, 2, 3, 4, 5];
        let path = storage.save("test-model", &data).await.unwrap();
        
        let size = storage.size(&path).await.unwrap();
        assert_eq!(size, 5);
    }
}
