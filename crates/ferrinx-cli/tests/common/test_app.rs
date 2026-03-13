use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferrinx_api::middleware::rate_limit::RateLimiter;
use ferrinx_api::routes::{create_router, AppState};
use ferrinx_common::{
    ApiKeyRecord, DatabaseBackend, DatabaseConfig, InferenceTask, ModelInfo, Permissions, User,
    UserRole,
};
use ferrinx_core::{InferenceEngine, LocalStorage, ModelLoader};
use ferrinx_db::DbContext;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

pub fn models_dir() -> std::path::PathBuf {
    fixtures_dir().join("models")
}

pub fn hanzi_tiny_model_path() -> String {
    models_dir().join("hanzi_tiny.onnx").to_string_lossy().to_string()
}

pub struct TestApp {
    pub db: TestDb,
    pub config: Arc<ferrinx_common::Config>,
    pub storage_path: tempfile::TempDir,
    cancel_token: CancellationToken,
}

impl TestApp {
    pub async fn new() -> Self {
        let db = TestDb::new().await;
        let storage_path = tempfile::tempdir().expect("Failed to create temp dir");

        let mut config = ferrinx_common::Config::default_dev();
        config.server.port = 0;
        config.storage.path = Some(storage_path.path().to_str().unwrap().to_string());
        config.database.url = format!("sqlite://{}", db.temp_file_path());
        let config = Arc::new(config);

        let cancel_token = CancellationToken::new();

        Self {
            db,
            config,
            storage_path,
            cancel_token,
        }
    }

    pub fn create_router(&self) -> Router {
        let storage = Arc::new(
            LocalStorage::new(self.storage_path.path().to_str().unwrap())
                .expect("Failed to create storage"),
        );
        let loader = Arc::new(ModelLoader::new(storage.clone()));
        let rate_limiter = Arc::new(RateLimiter::new(1000, 60));
        let engine = Arc::new(InferenceEngine::new(&self.config.onnx).expect("Failed to create engine"));

        let state = AppState {
            config: self.config.clone(),
            db: self.db.db.clone(),
            redis: None,
            engine,
            loader,
            storage,
            rate_limiter,
            cancel_token: self.cancel_token.clone(),
            start_time: std::time::Instant::now(),
        };

        create_router(state)
    }

    /// Start the server in a dedicated thread and block until it's fully ready.
    /// Returns the server address and a thread handle for graceful shutdown.
    pub fn start_server_blocking(&self) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let app = self.create_router();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<SocketAddr>();
        
        let cancel_token = self.cancel_token.clone();

        let handle = std::thread::spawn(move || {
            let runtime = Runtime::new().expect("Failed to create Tokio runtime");
            
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind(&addr).await
                    .expect("Failed to bind to address");
                let local_addr = listener.local_addr()
                    .expect("Failed to get local address");
                
                // Notify the main thread that the server is starting
                let _ = tx.send(local_addr);

                let server = axum::serve(listener, app);
                
                // Run the server with graceful shutdown
                tokio::select! {
                    result = server => {
                        if let Err(e) = result {
                            eprintln!("Server error: {}", e);
                        }
                    }
                    _ = cancel_token.cancelled() => {
                        println!("Server shutting down gracefully");
                    }
                }
            });
        });

        // Wait for the server to be ready (blocking)
        let server_addr = rx.recv()
            .expect("Server thread failed to start");

        // Give a small delay for the server to actually start accepting connections
        std::thread::sleep(Duration::from_millis(100));

        (server_addr, handle)
    }

    /// Start server in a way compatible with tokio test runtime
    pub async fn start_server(&self) -> (SocketAddr, std::thread::JoinHandle<()>) {
        // Just call the blocking version since we need synchronous server startup
        self.start_server_blocking()
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct TestDb {
    pub db: Arc<DbContext>,
    _temp_file: NamedTempFile,
}

impl TestDb {
    pub async fn new() -> Self {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let path = temp_file.path().to_str().expect("Invalid path");

        let config = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            url: format!("sqlite://{}", path),
            max_connections: 5,
            run_migrations: true,
        };

        let db = DbContext::new(&config)
            .await
            .expect("Failed to create test database");

        Self {
            db: Arc::new(db),
            _temp_file: temp_file,
        }
    }

    pub fn temp_file_path(&self) -> String {
        self._temp_file.path().to_str().expect("Invalid path").to_string()
    }

    pub async fn create_user(&self, username: &str, role: UserRole) -> User {
        let user = User {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash: hash_password("password"),
            role,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        self.db.users.save(&user).await.expect("Failed to create user");
        user
    }

    pub async fn create_api_key(&self, user: &User, name: &str, admin: bool) -> (Uuid, String) {
        let key_id = Uuid::new_v4();
        let raw_key = generate_raw_key();
        let key_hash = ferrinx_common::hash_key(&raw_key);

        let api_key = ApiKeyRecord {
            id: key_id,
            user_id: user.id,
            key_hash,
            name: name.to_string(),
            permissions: if admin {
                Permissions::admin_default()
            } else {
                Permissions::user_default()
            },
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db
            .api_keys
            .save(&api_key)
            .await
            .expect("Failed to create API key");

        (key_id, raw_key)
    }

    pub async fn create_model(&self, name: &str, version: &str, storage_path: Option<&std::path::Path>) -> ModelInfo {
        let source_path = std::path::PathBuf::from(hanzi_tiny_model_path());
        
        // Copy model file to test storage if provided
        let model_path = if let Some(storage) = storage_path {
            let storage_models_dir = storage.join("models");
            std::fs::create_dir_all(&storage_models_dir).expect("Failed to create models dir");
            let dest_path = storage_models_dir.join(format!("{}_{}.onnx", name, version));
            std::fs::copy(&source_path, &dest_path).expect("Failed to copy model file");
            dest_path
        } else {
            source_path
        };

        let file_size = std::fs::metadata(&model_path)
            .map(|m| m.len() as i64)
            .ok();

        let metadata = Some(serde_json::json!({
            "inputs": {
                "preprocess": [
                    {"type": "resize", "size": [64, 64]},
                    {"type": "normalize", "mean": 0.5, "std": 0.5}
                ]
            },
            "outputs": {
                "postprocess": [
                    {"type": "argmax"}
                ]
            }
        }));

        let model = ModelInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            version: version.to_string(),
            file_path: model_path.to_string_lossy().to_string(),
            file_size,
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!([
                {"name": "input", "shape": [1, 1, 64, 64], "element_type": "float32"}
            ])),
            output_shapes: Some(serde_json::json!([
                {"name": "output", "shape": [1, 994], "element_type": "float32"}
            ])),
            metadata,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db
            .models
            .save(&model)
            .await
            .expect("Failed to create model");
        model
    }

    pub async fn create_task(
        &self,
        model: &ModelInfo,
        user: &User,
        api_key_id: &Uuid,
    ) -> InferenceTask {
        let task = InferenceTask {
            id: Uuid::new_v4(),
            model_id: model.id,
            user_id: user.id,
            api_key_id: *api_key_id,
            status: ferrinx_common::TaskStatus::Pending,
            inputs: serde_json::json!({"input": [1.0, 2.0, 3.0]}),
            outputs: None,
            error_message: None,
            priority: 5,
            retry_count: 0,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
        };

        self.db
            .tasks
            .save(&task)
            .await
            .expect("Failed to create task");
        task
    }

    pub async fn health_check(&self) -> bool {
        self.db.health_check().await.is_ok()
    }
}

fn hash_password(password: &str) -> String {
    ferrinx_common::hash_password(password).expect("Failed to hash password")
}

fn generate_raw_key() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    hex::encode(random_bytes)
}