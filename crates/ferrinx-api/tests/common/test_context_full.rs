use std::collections::HashSet;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferrinx_api::middleware::rate_limit::RateLimiter;
use ferrinx_api::routes::{create_router, AppState};
use ferrinx_common::constants::{
    REDIS_STREAM_KEY_HIGH, REDIS_STREAM_KEY_LOW, REDIS_STREAM_KEY_NORMAL,
};
use ferrinx_common::{Config, RedisClient, RedisPoolConfig, TaskStatus, UserRole};
use ferrinx_core::{InferenceEngine, LocalStorage, ModelLoader, ModelStorage};
use ferrinx_db::DbContext;
use ferrinx_worker::consumer::TaskConsumer;
use ferrinx_worker::model_reporter::ModelReporter;
use ferrinx_worker::processor::TaskProcessor;
use ferrinx_worker::redis::RedisClient as WorkerRedisClient;
use tempfile::NamedTempFile;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::test_db::TestDb;

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

pub fn models_dir() -> PathBuf {
    fixtures_dir().join("models")
}

pub fn hanzi_tiny_model_path() -> String {
    models_dir()
        .join("hanzi_tiny.onnx")
        .to_string_lossy()
        .to_string()
}

pub struct TestContextFull {
    pub db: Arc<DbContext>,
    pub redis: Arc<RedisClient>,
    pub config: Arc<Config>,
    pub storage_path: tempfile::TempDir,
    pub cancel_token: CancellationToken,
    _temp_db: TestDb,
}

impl TestContextFull {
    pub async fn new() -> Option<Self> {
        let redis = create_redis_client().await?;
        let temp_db = TestDb::new().await;
        let storage_path = tempfile::tempdir().expect("Failed to create temp dir");

        if let Err(e) = redis.initialize_consumer_groups().await {
            eprintln!("Failed to initialize consumer groups: {}", e);
        }

        let mut config = Config::default_dev();
        config.server.port = 0;
        config.storage.path = Some(storage_path.path().to_str().unwrap().to_string());
        config.database.url = format!("sqlite://{}", temp_db.temp_file_path());
        let config = Arc::new(config);

        let cancel_token = CancellationToken::new();

        Some(Self {
            db: temp_db.db.clone(),
            redis,
            config,
            storage_path,
            cancel_token,
            _temp_db: temp_db,
        })
    }

    pub async fn create_user(&self, username: &str, role: UserRole) -> Uuid {
        let password_hash = ferrinx_common::hash_password("password").expect("Failed to hash");
        let user = ferrinx_common::User {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash,
            role,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db.users.save(&user).await.expect("Failed to create user");
        user.id
    }

    pub async fn create_api_key(&self, user_id: Uuid, name: &str) -> (Uuid, String) {
        let raw_key = format!("sk_test_{}", Uuid::new_v4());
        let key_hash = ferrinx_common::hash_api_key(&raw_key);

        let api_key = ferrinx_common::ApiKeyRecord {
            id: Uuid::new_v4(),
            user_id,
            name: name.to_string(),
            key_hash,
            permissions: ferrinx_common::Permissions::user_default(),
            is_active: true,
            is_temporary: false,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_used_at: None,
        };

        self.db.api_keys.save(&api_key).await.expect("Failed to create API key");
        (api_key.id, raw_key)
    }

    pub async fn create_model(&self, name: &str, version: &str) -> ferrinx_common::ModelInfo {
        let source_path = PathBuf::from(hanzi_tiny_model_path());
        let storage_models_dir = self.storage_path.path().join("models");
        std::fs::create_dir_all(&storage_models_dir).expect("Failed to create models dir");
        let dest_path = storage_models_dir.join(format!("{}_{}.onnx", name, version));
        std::fs::copy(&source_path, &dest_path).expect("Failed to copy model file");

        let model = ferrinx_common::ModelInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            version: version.to_string(),
            file_path: dest_path.to_string_lossy().to_string(),
            file_size: Some(dest_path.metadata().map(|m| m.len() as i64).unwrap_or(0)),
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!({"import/Placeholder:0": [1, 1, 64, 64]})),
            output_shapes: Some(serde_json::json!({"import/add:0": [1, 10]})),
            metadata: Some(serde_json::json!({"framework": "onnx", "task": "classification"})),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db.models.save(&model).await.expect("Failed to create model");
        model
    }

    pub fn create_router(&self) -> Router {
        let storage: Arc<dyn ModelStorage> = Arc::new(
            LocalStorage::new(self.storage_path.path().to_str().unwrap())
                .expect("Failed to create storage"),
        );
        let loader = Arc::new(ModelLoader::new(storage.clone()));
        let rate_limiter = Arc::new(RateLimiter::new(1000, 60));
        
        let engine = match InferenceEngine::new(&self.config.onnx) {
            Ok(e) => Arc::new(e),
            Err(e) => {
                eprintln!("Failed to create InferenceEngine: {:?}", e);
                panic!("Failed to create InferenceEngine: {:?}", e);
            }
        };

        let state = AppState {
            config: self.config.clone(),
            db: self.db.clone(),
            redis: Some(self.redis.clone()),
            engine,
            loader,
            storage,
            rate_limiter,
            cancel_token: self.cancel_token.clone(),
            start_time: std::time::Instant::now(),
        };

        create_router(state)
    }

    pub async fn start_api_server(&self) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let app = self.create_router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        (addr, handle)
    }

    pub async fn start_worker(&self) -> tokio::task::JoinHandle<()> {
        let consumer_name = format!("test-worker-{}", Uuid::new_v4());
        let cancel_token = self.cancel_token.clone();
        let redis_url = std::env::var("TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let db = self.db.clone();
        let config = self.config.clone();
        let storage_path = self.storage_path.path().to_str().unwrap().to_string();

        let handle = tokio::spawn(async move {
            let redis = match ferrinx_worker::redis::create_redis_client(&redis_url).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to create worker redis client: {}", e);
                    return;
                }
            };

            let storage: Arc<dyn ModelStorage> =
                Arc::new(LocalStorage::new(&storage_path).expect("Failed to create storage"));

            let cached_models: Arc<std::sync::RwLock<HashSet<Uuid>>> =
                Arc::new(std::sync::RwLock::new(HashSet::new()));

            let cached_models_clone = cached_models.clone();
            let on_load = Some(Arc::new(move |model_id: Uuid| {
                if let Ok(mut set) = cached_models_clone.write() {
                    set.insert(model_id);
                }
            }) as ferrinx_core::CacheLoadCallback);

            let cached_models_clone = cached_models.clone();
            let on_evict = Some(Arc::new(move |model_id: Uuid| {
                if let Ok(mut set) = cached_models_clone.write() {
                    set.remove(&model_id);
                }
            }) as ferrinx_core::CacheEvictCallback);

            let engine = Arc::new(
                InferenceEngine::new(&config.onnx)
                    .expect("Failed to create engine")
                    .with_callbacks(on_evict, on_load),
            );

            let streams = vec![
                REDIS_STREAM_KEY_HIGH.to_string(),
                REDIS_STREAM_KEY_NORMAL.to_string(),
                REDIS_STREAM_KEY_LOW.to_string(),
            ];

            let consumer = Arc::new(
                TaskConsumer::new(
                    redis.clone(),
                    consumer_name.clone(),
                    "ferrinx-workers".to_string(),
                    streams,
                )
                .with_claim_idle_ms(5000),
            );

            let processor = Arc::new(TaskProcessor::new(
                db.clone(),
                redis.clone(),
                engine.clone(),
                3,
                1000,
            ));

            let model_reporter = Arc::new(
                ModelReporter::new(
                    consumer_name.clone(),
                    redis.clone(),
                    storage.clone(),
                    db.clone(),
                    5,
                )
                .with_cached_models(cached_models),
            );

            let reporter_token = cancel_token.child_token();
            tokio::spawn(async move {
                model_reporter.run(reporter_token).await;
            });

            let current_tasks = Arc::new(AtomicUsize::new(0));

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        break;
                    }

                    result = consumer.poll_task() => {
                        match result {
                            Ok(Some(task_message)) => {
                                let processor = processor.clone();
                                let consumer = consumer.clone();
                                let current_tasks = current_tasks.clone();

                                current_tasks.fetch_add(1, Ordering::Relaxed);

                                tokio::spawn(async move {
                                    let result = tokio::time::timeout(
                                        Duration::from_secs(60),
                                        processor.process(task_message.clone()),
                                    )
                                    .await;

                                    match result {
                                        Ok(Ok(())) => {
                                            let _ = consumer
                                                .ack_task(&task_message.stream, &task_message.entry_id)
                                                .await;
                                        }
                                        Ok(Err(e)) => {
                                            eprintln!("Task processing failed: {}", e);
                                        }
                                        Err(_) => {
                                            eprintln!("Task processing timed out");
                                        }
                                    }

                                    current_tasks.fetch_sub(1, Ordering::Relaxed);
                                });
                            }
                            Ok(None) => {}
                            Err(e) => {
                                eprintln!("Error polling task: {}", e);
                            }
                        }
                    }
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        handle
    }
}

pub async fn create_redis_client() -> Option<Arc<RedisClient>> {
    let redis_url = std::env::var("TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let config = RedisPoolConfig {
        url: redis_url,
        pool_size: 5,
        connection_timeout: Duration::from_secs(5),
        api_key_cache_ttl: 3600,
        result_cache_ttl: 86400,
        task_timeout_ms: 300000,
    };

    match RedisClient::new(config).await {
        Ok(client) => {
            match tokio::time::timeout(Duration::from_secs(5), client.health_check()).await {
                Ok(Ok(())) => Some(Arc::new(client)),
                _ => None,
            }
        }
        Err(_) => None,
    }
}

pub fn create_temp_config(api_base: &str, api_key: Option<&str>) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    let api_url = if api_base.ends_with("/api/v1") {
        api_base.to_string()
    } else {
        format!("{}/api/v1", api_base)
    };
    let content = if let Some(key) = api_key {
        format!(
            r#"
api_url = "{}"
api_key = "{}"
timeout = 30
output_format = "json"
"#,
            api_url, key
        )
    } else {
        format!(
            r#"
api_url = "{}"
timeout = 30
output_format = "json"
"#,
            api_url
        )
    };
    file.write_all(content.as_bytes()).unwrap();
    file
}
