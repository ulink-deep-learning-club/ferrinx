use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use ferrinx_api::middleware::rate_limit::RateLimiter;
use ferrinx_api::routes::{create_router, AppState};
use ferrinx_common::{Config, OnnxConfig};
use ferrinx_core::{LocalStorage, ModelLoader};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::fixtures::{MockInferenceEngine, MockRedis, TestDb};

pub struct TestApp {
    pub db: TestDb,
    pub redis: Arc<MockRedis>,
    pub engine: Arc<MockInferenceEngine>,
    pub config: Arc<Config>,
    pub storage_path: TempDir,
    cancel_token: CancellationToken,
}

impl TestApp {
    pub async fn new() -> Self {
        let db = TestDb::new().await;
        let redis = Arc::new(MockRedis::new());
        let engine = Arc::new(MockInferenceEngine::with_default_response());

        let storage_path = tempfile::tempdir().expect("Failed to create temp dir");

        let config = Arc::new(Config {
            server: ferrinx_common::ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                api_version: "v1".to_string(),
                graceful_shutdown_timeout: 5,
            },
            database: ferrinx_common::DatabaseConfig::default(),
            redis: ferrinx_common::RedisConfig::default(),
            onnx: OnnxConfig::default(),
            storage: ferrinx_common::StorageConfig {
                backend: ferrinx_common::StorageBackend::Local,
                path: Some(storage_path.path().to_str().unwrap().to_string()),
            },
            auth: ferrinx_common::AuthConfig::default(),
            rate_limit: ferrinx_common::RateLimitConfig::default(),
            logging: ferrinx_common::LoggingConfig::default(),
            worker: ferrinx_common::WorkerConfig::default(),
            cleanup: ferrinx_common::CleanupConfig::default(),
        });

        let cancel_token = CancellationToken::new();

        Self {
            db,
            redis,
            engine,
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

        let state = AppState {
            config: self.config.clone(),
            db: self.db.db.clone(),
            redis: None,
            engine: self.engine.clone(),
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
