use serde::Deserialize;
use std::path::PathBuf;

use crate::utils::expand_env_vars_with_default;

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

fn default_api_key_secret() -> String {
    std::env::var("FERRINX_API_KEY_SECRET").unwrap_or_default()
}

fn default_database_url() -> String {
    std::env::var("FERRINX_DATABASE_URL").unwrap_or_default()
}

fn default_redis_url() -> String {
    std::env::var("FERRINX_REDIS_URL").unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_api_key_secret")]
    pub api_key_secret: String,
    pub api_key_prefix: String,
    pub max_keys_per_user: usize,
    pub temp_key_ttl_hours: u64,
    pub temp_key_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub backend: DatabaseBackend,
    #[serde(default = "default_database_url")]
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Postgresql,
    Sqlite,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    #[serde(default = "default_redis_url")]
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
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Local,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OnnxConfig {
    pub cache_size: usize,
    #[serde(default)]
    pub preload: Vec<String>,
    pub execution_provider: ExecutionProvider,
    pub gpu_device_id: u32,
    #[serde(default)]
    pub dynamic_lib_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExecutionProvider {
    CPU,
    CUDA,
    TensorRT,
    CoreML,
    ROCm,
    WebGPU,
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
    #[serde(default = "default_task_recovery_interval_secs")]
    pub task_recovery_interval_secs: u64,
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
    #[serde(default = "default_claim_idle_ms")]
    pub claim_idle_ms: i64,
}

fn default_task_recovery_interval_secs() -> u64 {
    300
}
fn default_health_check_interval_secs() -> u64 {
    30
}
fn default_claim_idle_ms() -> i64 {
    300_000
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

impl Config {
    pub fn from_file(path: &str) -> Result<Self, crate::error::CommonError> {
        let _ = dotenvy::dotenv();

        let config = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("FERRINX").separator("__"))
            .build()?;

        let mut config: Config = config.try_deserialize()?;

        config.database.url =
            expand_env_vars_with_default(&config.database.url, "sqlite://./data/ferrinx.db");
        config.redis.url = expand_env_vars_with_default(&config.redis.url, "");
        config.auth.api_key_secret =
            expand_env_vars_with_default(&config.auth.api_key_secret, "dev-secret-key");

        Ok(config)
    }

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
            rate_limit: RateLimitConfig {
                enabled: true,
                algorithm: RateLimitAlgorithm::SlidingWindow,
                default_rpm: 60,
                burst: 10,
                sync_inference_rpm: 30,
                async_inference_rpm: 100,
                cleanup_interval_secs: 60,
            },
            auth: AuthConfig {
                api_key_secret: "dev-secret-key".to_string(),
                api_key_prefix: "frx_sk".to_string(),
                max_keys_per_user: 10,
                temp_key_ttl_hours: 1,
                temp_key_prefix: "frx_sk_temp_".to_string(),
            },
            database: DatabaseConfig {
                backend: DatabaseBackend::Sqlite,
                url: "sqlite://./data/ferrinx.db".to_string(),
                max_connections: 5,
                run_migrations: true,
            },
            redis: RedisConfig {
                url: "redis://127.0.0.1:6379".to_string(),
                pool_size: 10,
                stream_key: "ferrinx:tasks:stream".to_string(),
                consumer_group: "ferrinx-workers".to_string(),
                dead_letter_stream: "ferrinx:tasks:dead_letter".to_string(),
                result_cache_prefix: "ferrinx:results".to_string(),
                result_cache_ttl: 86400,
                api_key_store: "ferrinx:api_keys".to_string(),
                api_key_cache_ttl: 3600,
                fallback_to_db: true,
            },
            storage: StorageConfig {
                backend: StorageBackend::Local,
                path: Some("./models".to_string()),
            },
            onnx: OnnxConfig {
                cache_size: 5,
                preload: vec![],
                execution_provider: ExecutionProvider::CPU,
                gpu_device_id: 0,
                dynamic_lib_path: None,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: LogFormat::Text,
                file: None,
                max_file_size_mb: 100,
                max_files: 10,
            },
            worker: WorkerConfig {
                consumer_name: "".to_string(),
                concurrency: 4,
                poll_interval_ms: 100,
                max_retries: 3,
                retry_delay_ms: 1000,
                task_recovery_interval_secs: 300,
                health_check_interval_secs: 30,
                claim_idle_ms: 300_000,
            },
            cleanup: CleanupConfig {
                enabled: true,
                completed_task_retention_days: 30,
                failed_task_retention_days: 7,
                cancelled_task_retention_days: 3,
                cleanup_interval_hours: 24,
                cleanup_batch_size: 1000,
                temp_key_cleanup_interval_hours: 1,
            },
            model_validation: ModelValidationConfig {
                enabled: true,
                validate_session: false,
                validation_timeout_secs: 30,
                async_validation: true,
            },
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.server.port == 0 {
            errors.push("server.port cannot be 0".to_string());
        }

        if self.server.sync_inference_concurrency == 0 {
            errors.push("sync_inference_concurrency must be > 0".to_string());
        }

        if self.database.max_connections == 0 {
            errors.push("database.max_connections must be > 0".to_string());
        }

        if self.onnx.cache_size == 0 {
            errors.push("onnx.cache_size must be > 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_dev_config() {
        let config = Config::default_dev();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_valid_config() {
        let config = Config::default_dev();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_port_zero() {
        let mut config = Config::default_dev();
        config.server.port = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("port")));
    }

    #[test]
    fn test_validate_zero_concurrency() {
        let mut config = Config::default_dev();
        config.server.sync_inference_concurrency = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("sync_inference_concurrency")));
    }

    #[test]
    fn test_validate_zero_db_connections() {
        let mut config = Config::default_dev();
        config.database.max_connections = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("max_connections")));
    }

    #[test]
    fn test_validate_zero_cache_size() {
        let mut config = Config::default_dev();
        config.onnx.cache_size = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("cache_size")));
    }

    #[test]
    fn test_validate_multiple_errors() {
        let mut config = Config::default_dev();
        config.server.port = 0;
        config.onnx.cache_size = 0;
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_execution_provider_cpu() {
        let config = Config::default_dev();
        assert!(matches!(
            config.onnx.execution_provider,
            ExecutionProvider::CPU
        ));
    }

    #[test]
    fn test_database_backend_sqlite() {
        let config = Config::default_dev();
        assert!(matches!(config.database.backend, DatabaseBackend::Sqlite));
    }

    #[test]
    fn test_storage_backend_local() {
        let config = Config::default_dev();
        assert!(matches!(config.storage.backend, StorageBackend::Local));
    }

    #[test]
    fn test_rate_limit_algorithm_default() {
        let config = Config::default_dev();
        assert!(matches!(
            config.rate_limit.algorithm,
            RateLimitAlgorithm::SlidingWindow
        ));
    }

    #[test]
    fn test_log_format_default() {
        let config = Config::default_dev();
        assert!(matches!(config.logging.format, LogFormat::Text));
    }

    #[test]
    fn test_worker_config_defaults() {
        let config = Config::default_dev();
        assert_eq!(config.worker.max_retries, 3);
        assert_eq!(config.worker.task_recovery_interval_secs, 300);
        assert_eq!(config.worker.health_check_interval_secs, 30);
        assert_eq!(config.worker.claim_idle_ms, 300_000);
    }

    #[test]
    fn test_cleanup_config() {
        let config = Config::default_dev();
        assert!(config.cleanup.enabled);
        assert_eq!(config.cleanup.completed_task_retention_days, 30);
        assert_eq!(config.cleanup.failed_task_retention_days, 7);
        assert_eq!(config.cleanup.cancelled_task_retention_days, 3);
    }

    #[test]
    fn test_model_validation_config() {
        let config = Config::default_dev();
        assert!(config.model_validation.enabled);
        assert!(!config.model_validation.validate_session);
        assert_eq!(config.model_validation.validation_timeout_secs, 30);
        assert!(config.model_validation.async_validation);
    }
}
