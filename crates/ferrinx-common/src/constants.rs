#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidApiKey,
    PermissionDenied,
    BootstrapDisabled,
    ModelNotFound,
    TaskNotFound,
    UserNotFound,
    ApiKeyNotFound,
    InvalidInput,
    InvalidModelFormat,
    ModelAlreadyExists,
    ModelNotValid,
    TaskNotCancellable,
    InferenceFailed,
    InternalError,
    InferenceTimeout,
    ServiceUnavailable,
    RedisUnavailable,
    RateLimitExceeded,
    NoWorkerAvailable,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::InvalidApiKey => "INVALID_API_KEY",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::BootstrapDisabled => "BOOTSTRAP_DISABLED",
            ErrorCode::ModelNotFound => "MODEL_NOT_FOUND",
            ErrorCode::TaskNotFound => "TASK_NOT_FOUND",
            ErrorCode::UserNotFound => "USER_NOT_FOUND",
            ErrorCode::ApiKeyNotFound => "API_KEY_NOT_FOUND",
            ErrorCode::InvalidInput => "INVALID_INPUT",
            ErrorCode::InvalidModelFormat => "INVALID_MODEL_FORMAT",
            ErrorCode::ModelAlreadyExists => "MODEL_ALREADY_EXISTS",
            ErrorCode::ModelNotValid => "MODEL_NOT_VALID",
            ErrorCode::TaskNotCancellable => "TASK_NOT_CANCELLABLE",
            ErrorCode::InferenceFailed => "INFERENCE_FAILED",
            ErrorCode::InternalError => "INTERNAL_ERROR",
            ErrorCode::InferenceTimeout => "INFERENCE_TIMEOUT",
            ErrorCode::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            ErrorCode::RedisUnavailable => "REDIS_UNAVAILABLE",
            ErrorCode::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            ErrorCode::NoWorkerAvailable => "NO_WORKER_AVAILABLE",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::InvalidApiKey => 401,
            ErrorCode::PermissionDenied | ErrorCode::BootstrapDisabled => 403,
            ErrorCode::ModelNotFound
            | ErrorCode::TaskNotFound
            | ErrorCode::UserNotFound
            | ErrorCode::ApiKeyNotFound => 404,
            ErrorCode::InvalidInput
            | ErrorCode::InvalidModelFormat
            | ErrorCode::ModelAlreadyExists
            | ErrorCode::ModelNotValid
            | ErrorCode::TaskNotCancellable => 400,
            ErrorCode::InferenceFailed | ErrorCode::InternalError => 500,
            ErrorCode::InferenceTimeout => 504,
            ErrorCode::ServiceUnavailable
            | ErrorCode::RedisUnavailable
            | ErrorCode::NoWorkerAvailable => 503,
            ErrorCode::RateLimitExceeded => 429,
        }
    }
}

pub const API_KEY_PREFIX: &str = "frx_sk";
pub const TEMP_KEY_PREFIX: &str = "frx_sk_temp";
pub const API_VERSION: &str = "v1";

pub const DEFAULT_SYNC_INFERENCE_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_SYNC_INFERENCE_CONCURRENCY: usize = 4;

pub const REDIS_STREAM_KEY_HIGH: &str = "ferrinx:tasks:high";
pub const REDIS_STREAM_KEY_NORMAL: &str = "ferrinx:tasks:normal";
pub const REDIS_STREAM_KEY_LOW: &str = "ferrinx:tasks:low";
pub const REDIS_DEAD_LETTER_STREAM: &str = "ferrinx:tasks:dead_letter";
pub const REDIS_CONSUMER_GROUP: &str = "ferrinx-workers";
pub const REDIS_RESULT_CACHE_PREFIX: &str = "ferrinx:results";
pub const REDIS_API_KEY_STORE: &str = "ferrinx:api_keys";

pub const REDIS_WORKER_MODELS_PREFIX: &str = "ferrinx:workers";
pub const REDIS_WORKER_HEARTBEAT_SUFFIX: &str = "heartbeat";
pub const REDIS_WORKER_MODELS_SUFFIX: &str = "models";
pub const REDIS_MODEL_WORKERS_PREFIX: &str = "ferrinx:models";
pub const REDIS_MODEL_WORKERS_SUFFIX: &str = "workers";

pub const REDIS_WORKER_HEARTBEAT_TTL_SECS: u64 = 60;
pub const REDIS_WORKER_MODELS_TTL_SECS: u64 = 30;
pub const WORKER_STATUS_REPORT_INTERVAL_SECS: u64 = 10;
