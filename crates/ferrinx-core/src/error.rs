use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Model load failed: {0}")]
    ModelLoadFailed(String),

    #[error("Invalid model format: {0}")]
    InvalidModelFormat(String),

    #[error("Model parse failed: {0}")]
    ModelParseFailed(String),

    #[error("Session creation failed: {0}")]
    SessionCreationFailed(String),

    #[error("Inference failed: {0}")]
    InferenceFailed(String),

    #[error("Inference timeout")]
    InferenceTimeout,

    #[error("Concurrency limit reached")]
    ConcurrencyLimitReached,

    #[error("Input not found: {0}")]
    InputNotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Unsupported tensor type")]
    UnsupportedTensorType,

    #[error("Unsupported input type")]
    UnsupportedInputType,

    #[error("Execution provider error: {0}")]
    ExecutionProviderError(String),

    #[error("Validation timeout")]
    ValidationTimeout,

    #[error("Blocking task failed: {0}")]
    BlockingTaskFailed(String),

    #[error("ONNX Runtime library not found: {0}")]
    OnnxRuntimeLibraryNotFound(String),

    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("ONNX error: {0}")]
    OrtError(#[from] ort::Error),

    #[error("Ndarray error: {0}")]
    NdarrayError(String),
}

impl From<ndarray::ShapeError> for CoreError {
    fn from(err: ndarray::ShapeError) -> Self {
        CoreError::NdarrayError(err.to_string())
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("File not found: {0}")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_not_found_display() {
        let error = CoreError::ModelNotFound("test-model".to_string());
        assert_eq!(format!("{}", error), "Model not found: test-model");
    }

    #[test]
    fn test_model_load_failed_display() {
        let error = CoreError::ModelLoadFailed("Invalid format".to_string());
        assert_eq!(format!("{}", error), "Model load failed: Invalid format");
    }

    #[test]
    fn test_invalid_model_format_display() {
        let error = CoreError::InvalidModelFormat("Missing layer".to_string());
        assert_eq!(format!("{}", error), "Invalid model format: Missing layer");
    }

    #[test]
    fn test_model_parse_failed_display() {
        let error = CoreError::ModelParseFailed("Syntax error".to_string());
        assert_eq!(format!("{}", error), "Model parse failed: Syntax error");
    }

    #[test]
    fn test_session_creation_failed_display() {
        let error = CoreError::SessionCreationFailed("Out of memory".to_string());
        assert_eq!(
            format!("{}", error),
            "Session creation failed: Out of memory"
        );
    }

    #[test]
    fn test_inference_failed_display() {
        let error = CoreError::InferenceFailed("Shape mismatch".to_string());
        assert_eq!(format!("{}", error), "Inference failed: Shape mismatch");
    }

    #[test]
    fn test_inference_timeout_display() {
        let error = CoreError::InferenceTimeout;
        assert_eq!(format!("{}", error), "Inference timeout");
    }

    #[test]
    fn test_concurrency_limit_reached_display() {
        let error = CoreError::ConcurrencyLimitReached;
        assert_eq!(format!("{}", error), "Concurrency limit reached");
    }

    #[test]
    fn test_input_not_found_display() {
        let error = CoreError::InputNotFound("input.1".to_string());
        assert_eq!(format!("{}", error), "Input not found: input.1");
    }

    #[test]
    fn test_invalid_input_display() {
        let error = CoreError::InvalidInput("Wrong shape".to_string());
        assert_eq!(format!("{}", error), "Invalid input: Wrong shape");
    }

    #[test]
    fn test_unsupported_tensor_type_display() {
        let error = CoreError::UnsupportedTensorType;
        assert_eq!(format!("{}", error), "Unsupported tensor type");
    }

    #[test]
    fn test_unsupported_input_type_display() {
        let error = CoreError::UnsupportedInputType;
        assert_eq!(format!("{}", error), "Unsupported input type");
    }

    #[test]
    fn test_execution_provider_error_display() {
        let error = CoreError::ExecutionProviderError("CUDA not available".to_string());
        assert_eq!(
            format!("{}", error),
            "Execution provider error: CUDA not available"
        );
    }

    #[test]
    fn test_validation_timeout_display() {
        let error = CoreError::ValidationTimeout;
        assert_eq!(format!("{}", error), "Validation timeout");
    }

    #[test]
    fn test_blocking_task_failed_display() {
        let error = CoreError::BlockingTaskFailed("Thread panic".to_string());
        assert_eq!(format!("{}", error), "Blocking task failed: Thread panic");
    }

    #[test]
    fn test_ndarray_error_display() {
        let error = CoreError::NdarrayError("Shape mismatch".to_string());
        assert_eq!(format!("{}", error), "Ndarray error: Shape mismatch");
    }

    #[test]
    fn test_storage_error_io() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let storage_error = StorageError::IoError(io_error);
        assert!(storage_error.to_string().contains("IO error"));
    }

    #[test]
    fn test_storage_error_invalid_path() {
        let error = StorageError::InvalidPath("../invalid".to_string());
        assert_eq!(format!("{}", error), "Invalid path: ../invalid");
    }

    #[test]
    fn test_storage_error_file_not_found() {
        let error = StorageError::FileNotFound("/models/missing.onnx".to_string());
        assert_eq!(format!("{}", error), "File not found: /models/missing.onnx");
    }

    #[test]
    fn test_storage_error_into_core_error() {
        let storage_error = StorageError::FileNotFound("test.onnx".to_string());
        let core_error: CoreError = storage_error.into();
        assert!(matches!(core_error, CoreError::StorageError(_)));
    }

    #[test]
    fn test_io_error_into_core_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let core_error: CoreError = io_error.into();
        assert!(matches!(core_error, CoreError::IoError(_)));
    }

    #[test]
    fn test_json_error_into_core_error() {
        let json_result: std::result::Result<serde_json::Value, _> =
            serde_json::from_str("invalid");
        let json_error = json_result.unwrap_err();
        let core_error: CoreError = json_error.into();
        assert!(matches!(core_error, CoreError::JsonError(_)));
    }

    #[test]
    fn test_ndarray_shape_error_from_shape_mismatch() {
        use ndarray::ErrorKind;
        use ndarray::ShapeError;
        let shape_err = ShapeError::from_kind(ErrorKind::IncompatibleShape);
        let core_err: CoreError = shape_err.into();
        assert!(matches!(core_err, CoreError::NdarrayError(_)));
    }

    #[test]
    fn test_error_debug() {
        let error = CoreError::ModelNotFound("test".to_string());
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("ModelNotFound"));
    }

    #[test]
    fn test_result_type() {
        fn returns_result() -> Result<String> {
            Ok("success".to_string())
        }
        assert!(returns_result().is_ok());
    }
    #[test]
    fn test_result_error() {
        fn returns_error() -> Result<String> {
            Err(CoreError::InferenceTimeout)
        }
        assert!(returns_error().is_err());
    }

    #[test]
    fn test_onnx_runtime_library_not_found_display() {
        let error = CoreError::OnnxRuntimeLibraryNotFound("/path/to/lib.so".to_string());
        assert_eq!(
            format!("{}", error),
            "ONNX Runtime library not found: /path/to/lib.so"
        );
    }
}
