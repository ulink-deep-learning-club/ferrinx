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

    #[error("S3 error: {0}")]
    S3Error(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("File not found: {0}")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
