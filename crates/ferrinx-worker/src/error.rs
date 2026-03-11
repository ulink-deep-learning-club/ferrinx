use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Model not valid: {0}")]
    ModelNotValid(String),

    #[error("Invalid task message")]
    InvalidTaskMessage,

    #[error("Redis error: {0}")]
    RedisError(String),

    #[error("Database error: {0}")]
    DbError(#[from] ferrinx_db::DbError),

    #[error("Core error: {0}")]
    CoreError(#[from] ferrinx_core::CoreError),

    #[error("Storage error: {0}")]
    StorageError(#[from] ferrinx_core::StorageError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Task timeout")]
    TaskTimeout,

    #[error("Worker shutdown")]
    Shutdown,

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type Result<T> = std::result::Result<T, WorkerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_not_found_display() {
        let id = Uuid::nil();
        let error = WorkerError::TaskNotFound(id);
        assert_eq!(format!("{}", error), format!("Task not found: {}", id));
    }

    #[test]
    fn test_model_not_found_display() {
        let error = WorkerError::ModelNotFound("test-model".to_string());
        assert_eq!(format!("{}", error), "Model not found: test-model");
    }

    #[test]
    fn test_model_not_valid_display() {
        let error = WorkerError::ModelNotValid("Invalid ONNX".to_string());
        assert_eq!(format!("{}", error), "Model not valid: Invalid ONNX");
    }

    #[test]
    fn test_invalid_task_message_display() {
        let error = WorkerError::InvalidTaskMessage;
        assert_eq!(format!("{}", error), "Invalid task message");
    }

    #[test]
    fn test_redis_error_display() {
        let error = WorkerError::RedisError("Connection refused".to_string());
        assert_eq!(format!("{}", error), "Redis error: Connection refused");
    }

    #[test]
    fn test_task_timeout_display() {
        let error = WorkerError::TaskTimeout;
        assert_eq!(format!("{}", error), "Task timeout");
    }

    #[test]
    fn test_shutdown_display() {
        let error = WorkerError::Shutdown;
        assert_eq!(format!("{}", error), "Worker shutdown");
    }

    #[test]
    fn test_config_error_display() {
        let error = WorkerError::ConfigError("Invalid URL".to_string());
        assert_eq!(format!("{}", error), "Configuration error: Invalid URL");
    }

    #[test]
    fn test_json_error_conversion() {
        let json_result: std::result::Result<serde_json::Value, _> =
            serde_json::from_str("invalid");
        assert!(json_result.is_err());
        let worker_error: WorkerError = json_result.unwrap_err().into();
        assert!(matches!(worker_error, WorkerError::JsonError(_)));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let worker_error: WorkerError = io_error.into();
        assert!(matches!(worker_error, WorkerError::IoError(_)));
    }

    #[test]
    fn test_core_error_conversion() {
        let core_error = ferrinx_core::CoreError::InferenceTimeout;
        let worker_error: WorkerError = core_error.into();
        assert!(matches!(worker_error, WorkerError::CoreError(_)));
    }

    #[test]
    fn test_storage_error_conversion() {
        let storage_error = ferrinx_core::StorageError::FileNotFound("test.onnx".to_string());
        let worker_error: WorkerError = storage_error.into();
        assert!(matches!(worker_error, WorkerError::StorageError(_)));
    }

    #[test]
    fn test_error_debug() {
        let error = WorkerError::TaskTimeout;
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("TaskTimeout"));
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
            Err(WorkerError::Shutdown)
        }
        assert!(returns_error().is_err());
    }
}
