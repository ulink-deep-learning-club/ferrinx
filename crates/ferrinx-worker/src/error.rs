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
