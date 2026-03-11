use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use ferrinx_api::middleware::rate_limit::RateLimiter;
use ferrinx_api::routes::{create_router, AppState};
use ferrinx_common::Config;
use ferrinx_core::{InferenceEngine, LocalStorage, ModelLoader};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::common::TestDb;

pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/common")
}

pub fn models_dir() -> std::path::PathBuf {
    fixtures_dir().join("models")
}

pub fn lenet_model_path() -> String {
    models_dir().join("lenet.onnx").to_string_lossy().to_string()
}

pub struct TestApp {
    pub db: TestDb,
    pub config: Arc<Config>,
    pub storage_path: TempDir,
    cancel_token: CancellationToken,
}

impl TestApp {
    pub async fn new() -> Self {
        let db = TestDb::new().await;
        let storage_path = tempfile::tempdir().expect("Failed to create temp dir");

        let mut config = Config::default_dev();
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
        };

        create_router(state)
    }

    pub async fn start_server(&self) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let app = self.create_router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        (addr, handle)
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
