# ferrinx-common 模块设计

## 1. 模块职责

`ferrinx-common` 是所有其他 crate 的基础依赖，提供：
- 统一的配置管理
- 公共类型定义
- 常量和错误码
- 工具函数

**关键特性**：
- 无重型依赖（不依赖 ort、sqlx、redis 等）
- 编译速度快
- 可被所有子模块复用

## 2. 核心结构设计

### 2.1 配置结构

```rust
// src/config.rs

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub rate_limit: RateLimitConfig,
    pub auth: AuthConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub storage: StorageConfig,
    pub onnx: OnnxConfig,
    pub logging: LoggingConfig,
    pub worker: WorkerConfig,
    pub cleanup: CleanupConfig,
    pub model_validation: ModelValidationConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub max_request_size_mb: usize,
    pub graceful_shutdown_timeout: u64,
    pub sync_inference_concurrency: usize,
    pub sync_inference_timeout: u64,
    pub api_version: String,
    pub include_version_header: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub algorithm: RateLimitAlgorithm,
    pub default_rpm: u32,
    pub burst: u32,
    pub sync_inference_rpm: u32,
    pub async_inference_rpm: u32,
    pub cleanup_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitAlgorithm {
    TokenBucket,
    SlidingWindow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub api_key_secret: String,
    pub api_key_prefix: String,
    pub max_keys_per_user: usize,
    pub temp_key_ttl_hours: u64,
    pub temp_key_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub backend: DatabaseBackend,
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Postgresql,
    Sqlite,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub url: String,
    pub pool_size: u32,
    pub stream_key: String,
    pub consumer_group: String,
    pub dead_letter_stream: String,
    pub result_cache_prefix: String,
    pub result_cache_ttl: u64,
    pub api_key_store: String,
    pub api_key_cache_ttl: u64,
    pub fallback_to_db: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub backend: StorageBackend,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub bucket: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Local,
    // S3 - 推迟到将来实现
}

#[derive(Debug, Clone, Deserialize)]
pub struct OnnxConfig {
    pub cache_size: usize,
    #[serde(default)]
    pub preload: Vec<String>,
    pub execution_provider: ExecutionProvider,
    pub gpu_device_id: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExecutionProvider {
    CPU,
    CUDA,
    TensorRT,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
    pub file: Option<PathBuf>,
    pub max_file_size_mb: u64,
    pub max_files: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub consumer_name: String,
    pub concurrency: usize,
    pub poll_interval_ms: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CleanupConfig {
    pub enabled: bool,
    pub completed_task_retention_days: u32,
    pub failed_task_retention_days: u32,
    pub cancelled_task_retention_days: u32,
    pub cleanup_interval_hours: u64,
    pub cleanup_batch_size: usize,
    pub temp_key_cleanup_interval_hours: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelValidationConfig {
    pub enabled: bool,
    pub validate_session: bool,
    pub validation_timeout_secs: u64,
    pub async_validation: bool,
}
```

### 2.2 公共类型定义

```rust
// src/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 用户角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
    Admin,
}

/// 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub role: UserRole,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 权限定义
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
            models: vec!["read".to_string(), "write".to_string(), "delete".to_string()],
            inference: vec!["execute".to_string()],
            api_keys: vec!["read".to_string(), "write".to_string(), "delete".to_string()],
            admin: true,
        }
    }
}

/// API Key 记录
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

/// 模型信息
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
    /// 生成唯一键
    pub fn unique_key(&self) -> String {
        format!("{}:{}", self.name, self.version)
    }
    
    /// 检查模型是否有效（需要同时有 config 和 input_shapes）
    pub fn is_valid(&self) -> bool {
        self.metadata.is_some() && self.input_shapes.is_some()
    }
    
    /// 检查是否有配置文件
    pub fn has_config(&self) -> bool {
        self.metadata.is_some()
    }
    
    /// 获取验证错误信息
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

/// 任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 任务优先级
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
}

/// 推理任务
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

/// 推理输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceInput {
    pub inputs: std::collections::HashMap<String, serde_json::Value>,
}

/// 推理输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceOutput {
    pub outputs: std::collections::HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
}

/// 推理结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub outputs: serde_json::Value,
    pub error_message: Option<String>,
}
```

### 2.3 错误码定义

```rust
// src/constants.rs

/// 错误码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // 认证相关 (401)
    InvalidApiKey,
    
    // 权限相关 (403)
    PermissionDenied,
    BootstrapDisabled,
    
    // 资源不存在 (404)
    ModelNotFound,
    TaskNotFound,
    UserNotFound,
    ApiKeyNotFound,
    
    // 请求错误 (400)
    InvalidInput,
    InvalidModelFormat,
    ModelAlreadyExists,
    
    // 服务错误 (500)
    InferenceFailed,
    InternalError,
    
    // 网关错误 (504)
    InferenceTimeout,
    
    // 服务不可用 (503)
    ServiceUnavailable,
    RedisUnavailable,
    
    // 限流 (429)
    RateLimitExceeded,
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
            ErrorCode::InferenceFailed => "INFERENCE_FAILED",
            ErrorCode::InternalError => "INTERNAL_ERROR",
            ErrorCode::InferenceTimeout => "INFERENCE_TIMEOUT",
            ErrorCode::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            ErrorCode::RedisUnavailable => "REDIS_UNAVAILABLE",
            ErrorCode::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
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
            | ErrorCode::ModelAlreadyExists => 400,
            ErrorCode::InferenceFailed | ErrorCode::InternalError => 500,
            ErrorCode::InferenceTimeout => 504,
            ErrorCode::ServiceUnavailable | ErrorCode::RedisUnavailable => 503,
            ErrorCode::RateLimitExceeded => 429,
        }
    }
}

/// 常量定义
pub mod constants {
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
}
```

### 2.4 工具函数

```rust
// src/utils.rs

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// SHA-256 哈希
pub fn sha256_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 生成随机 API Key
pub fn generate_api_key(prefix: &str) -> String {
    let random_bytes: String = (0..32)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    format!("{}_{}", prefix, random_bytes)
}

/// 生成 UUID
pub fn generate_uuid() -> Uuid {
    Uuid::new_v4()
}

/// 环境变量替换
pub fn expand_env_vars(input: &str) -> String {
    // 支持 ${VAR_NAME} 格式的环境变量替换
    shellexpand::env(input).unwrap_or_else(|_| input.into()).to_string()
}

/// 验证 API Key 格式
pub fn validate_api_key_format(key: &str, prefix: &str) -> bool {
    key.starts_with(prefix) && key.len() > prefix.len() + 1
}
```

## 3. 配置加载

```rust
// src/config.rs (续)

impl Config {
    /// 从文件加载配置
    pub fn from_file(path: &str) -> Result<Self, config::ConfigError> {
        let config = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("FERRINX").separator("__"))
            .build()?;
        
        let mut config: Config = config.try_deserialize()?;
        
        // 环境变量替换
        config.database.url = expand_env_vars(&config.database.url);
        config.redis.url = expand_env_vars(&config.redis.url);
        config.auth.api_key_secret = expand_env_vars(&config.auth.api_key_secret);
        
        Ok(config)
    }
    
    /// 默认配置（开发环境）
    pub fn default_dev() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                workers: 4,
                max_request_size_mb: 500,
                graceful_shutdown_timeout: 30,
                sync_inference_concurrency: 4,
                sync_inference_timeout: 30,
                api_version: "v1".to_string(),
                include_version_header: true,
            },
            // ... 其他默认配置
        }
    }
}
```

## 4. 依赖关系

```toml
# Cargo.toml

[package]
name = "ferrinx-common"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
sha2 = { workspace = true }
config = { workspace = true }
toml = { workspace = true }
thiserror = { workspace = true }
shellexpand = "3.1"
rand = "0.8"
```

## 5. 错误处理

```rust
// src/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommonError {
    #[error("Configuration error: {0}")]
    ConfigError(#[from] config::ConfigError),
    
    #[error("Invalid API key format")]
    InvalidApiKeyFormat,
    
    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),
}

pub type Result<T> = std::result::Result<T, CommonError>;
```

## 6. 模块组织

```rust
// src/lib.rs

pub mod config;
pub mod constants;
pub mod error;
pub mod types;
pub mod utils;

pub use config::*;
pub use constants::*;
pub use error::*;
pub use types::*;
pub use utils::*;
```

## 7. 使用示例

```rust
// 在其他 crate 中使用

use ferrinx_common::{Config, generate_api_key, sha256_hash, ErrorCode};

// 加载配置
let config = Config::from_file("config.toml")?;

// 生成 API Key
let api_key = generate_api_key("frx_sk");

// 计算哈希
let hash = sha256_hash(&api_key);

// 使用错误码
let error_code = ErrorCode::ModelNotFound;
println!("Error: {} (HTTP {})", error_code.as_str(), error_code.http_status());
```

## 8. 测试策略

### 8.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sha256_hash() {
        let input = "test_api_key";
        let hash = sha256_hash(input);
        assert_eq!(hash.len(), 64); // SHA-256 输出 64 个十六进制字符
    }
    
    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key("frx_sk");
        assert!(key.starts_with("frx_sk_"));
        assert!(key.len() > 10);
    }
    
    #[test]
    fn test_permissions_default() {
        let user_perms = Permissions::user_default();
        assert!(!user_perms.admin);
        assert!(user_perms.models.contains(&"read".to_string()));
        
        let admin_perms = Permissions::admin_default();
        assert!(admin_perms.admin);
    }
    
    #[test]
    fn test_task_priority_conversion() {
        let priority = TaskPriority::High;
        assert_eq!(priority.as_i32(), 10);
        
        let from_int = TaskPriority::from_i32(6);
        assert_eq!(from_int, TaskPriority::Normal);
    }
    
    #[test]
    fn test_error_code_http_status() {
        assert_eq!(ErrorCode::InvalidApiKey.http_status(), 401);
        assert_eq!(ErrorCode::ModelNotFound.http_status(), 404);
        assert_eq!(ErrorCode::InferenceTimeout.http_status(), 504);
    }
}
```

### 8.2 配置测试

```rust
#[cfg(test)]
mod config_tests {
    use super::*;
    
    #[test]
    fn test_load_config_from_toml() {
        let toml_content = r#"
            [server]
            host = "0.0.0.0"
            port = 8080
            workers = 4
            max_request_size_mb = 500
            graceful_shutdown_timeout = 30
            sync_inference_concurrency = 4
            sync_inference_timeout = 30
            api_version = "v1"
            include_version_header = true
            
            [rate_limit]
            enabled = true
            algorithm = "sliding_window"
            default_rpm = 60
            burst = 10
            sync_inference_rpm = 30
            async_inference_rpm = 100
            cleanup_interval_secs = 60
        "#;
        
        // 测试解析逻辑
        // 实际测试需要写入临时文件
    }
}
```

## 9. 设计要点

### 9.1 无重型依赖

- 不引入 `ort`、`sqlx`、`redis` 等重型依赖
- 编译速度快，便于 CI/CD
- 减少依赖冲突

### 9.2 类型安全

- 使用 `serde` 进行序列化/反序列化
- 枚举类型确保类型安全
- `newtype` 模式避免原始类型滥用

### 9.3 配置管理

- 支持文件配置和环境变量
- 环境变量优先级高于文件
- 支持 `${VAR}` 格式的环境变量替换

### 9.4 错误码规范

- 错误码统一管理
- HTTP 状态码自动映射
- 便于 API 响应统一处理

## 10. 后续优化

### 10.1 配置验证

```rust
impl Config {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        
        if self.server.port == 0 {
            errors.push("server.port cannot be 0".to_string());
        }
        
        if self.sync_inference_concurrency == 0 {
            errors.push("sync_inference_concurrency must be > 0".to_string());
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
```

### 10.2 配置热更新

```rust
// 未来支持配置热更新
pub struct ConfigWatcher {
    config: Arc<RwLock<Config>>,
}

impl ConfigWatcher {
    pub async fn watch(&self, path: &str) {
        // 监听文件变化
        // 重新加载配置
    }
}
```

### 10.3 国际化错误消息

```rust
impl ErrorCode {
    pub fn message(&self, lang: &str) -> String {
        // 根据 lang 返回不同语言的错误消息
        match lang {
            "zh" => self.message_zh(),
            _ => self.message_en(),
        }
    }
}
```
