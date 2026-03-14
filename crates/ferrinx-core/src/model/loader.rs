use std::sync::Arc;
use std::time::Duration;

use ort::session::Session;
use ort::value::ValueType;

use crate::error::{CoreError, Result};
use crate::storage::ModelStorage;
use ferrinx_common::{ModelMetadata, TensorInfo};

pub struct ModelLoader {
    storage: Arc<dyn ModelStorage>,
}

impl ModelLoader {
    pub fn new(storage: Arc<dyn ModelStorage>) -> Self {
        Self { storage }
    }

    pub async fn load_model_data(&self, path: &str) -> Result<Vec<u8>> {
        self.storage.load(path).await.map_err(CoreError::from)
    }

    pub async fn validate_model(&self, data: &[u8]) -> Result<ModelMetadata> {
        self.check_onnx_magic(data)?;
        let metadata = self.extract_metadata(data).await?;
        Ok(metadata)
    }

    fn check_onnx_magic(&self, data: &[u8]) -> Result<()> {
        if data.len() < 4 {
            return Err(CoreError::InvalidModelFormat("File too small".to_string()));
        }

        if data[0] != 0x08 && data[0] != 0x0a {
            return Err(CoreError::InvalidModelFormat(
                "Invalid ONNX file header".to_string(),
            ));
        }

        Ok(())
    }

    async fn extract_metadata(&self, data: &[u8]) -> Result<ModelMetadata> {
        let temp_file = tempfile::NamedTempFile::new()?;
        tokio::fs::write(&temp_file, data).await?;

        let temp_path = temp_file.path().to_path_buf();
        let session = tokio::task::spawn_blocking(move || {
            Session::builder()
                .map_err(|e| CoreError::SessionCreationFailed(e.to_string()))?
                .commit_from_file(&temp_path)
                .map_err(|e| CoreError::ModelParseFailed(e.to_string()))
        })
        .await
        .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))??;

        let mut inputs = Vec::new();
        for input in session.inputs().iter() {
            let (shape, element_type) = match input.dtype() {
                ValueType::Tensor { ty, shape, .. } => {
                    let shape_vec: Vec<i64> = shape.iter().copied().collect();
                    (shape_vec, format!("{:?}", ty))
                }
                _ => (vec![], "unknown".to_string()),
            };
            inputs.push(TensorInfo {
                name: input.name().to_string(),
                shape,
                element_type,
            });
        }

        let mut outputs = Vec::new();
        for output in session.outputs().iter() {
            let (shape, element_type) = match output.dtype() {
                ValueType::Tensor { ty, shape, .. } => {
                    let shape_vec: Vec<i64> = shape.iter().copied().collect();
                    (shape_vec, format!("{:?}", ty))
                }
                _ => (vec![], "unknown".to_string()),
            };
            outputs.push(TensorInfo {
                name: output.name().to_string(),
                shape,
                element_type,
            });
        }

        Ok(ModelMetadata {
            inputs,
            outputs,
            opset_version: None,
            producer_name: None,
        })
    }

    pub async fn validate_executable(&self, data: &[u8], timeout: Duration) -> Result<()> {
        let temp_file = tempfile::NamedTempFile::new()?;
        tokio::fs::write(&temp_file, data).await?;

        let temp_path = temp_file.path().to_path_buf();
        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                Session::builder().and_then(|mut b| b.commit_from_file(&temp_path))
            }),
        )
        .await
        .map_err(|_| CoreError::ValidationTimeout)?
        .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))?;

        result.map_err(|e| CoreError::SessionCreationFailed(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalStorage;

    fn create_test_storage() -> Arc<dyn ModelStorage> {
        Arc::new(LocalStorage::new("./test_models").unwrap())
    }

    #[test]
    fn test_check_onnx_magic_valid_first_byte() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x08, 0x00, 0x00, 0x00];
        assert!(loader.check_onnx_magic(&data).is_ok());
    }

    #[test]
    fn test_check_onnx_magic_valid_second_byte() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x0a, 0x00, 0x00, 0x00];
        assert!(loader.check_onnx_magic(&data).is_ok());
    }

    #[test]
    fn test_check_onnx_magic_too_small() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x08, 0x00];
        assert!(loader.check_onnx_magic(&data).is_err());
    }

    #[test]
    fn test_check_onnx_magic_invalid() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x00, 0x00, 0x00, 0x00];
        assert!(loader.check_onnx_magic(&data).is_err());
    }

    #[test]
    fn test_check_onnx_magic_empty() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![];
        assert!(loader.check_onnx_magic(&data).is_err());
    }

    #[tokio::test]
    async fn test_validate_model_too_small() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x00];
        let result = loader.validate_model(&data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_model_invalid_header() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x00, 0x00, 0x00, 0x00, 0x00];
        let result = loader.validate_model(&data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_executable_timeout() {
        let loader = ModelLoader::new(create_test_storage());
        let data = vec![0x08, 0x00, 0x00, 0x00, 0x00];
        let result = loader
            .validate_executable(&data, Duration::from_millis(1))
            .await;
        assert!(result.is_err());
    }
}
