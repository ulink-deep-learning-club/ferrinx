use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::Result;
use crate::redis::RedisClient;

pub type CachedModelsRef = Arc<std::sync::RwLock<HashSet<Uuid>>>;

pub struct ModelReporter {
    worker_id: String,
    redis: Arc<dyn RedisClient>,
    storage: Arc<dyn ferrinx_core::ModelStorage>,
    db: Arc<ferrinx_db::DbContext>,
    cached_models: CachedModelsRef,
    report_interval: Duration,
}

impl ModelReporter {
    pub fn new(
        worker_id: String,
        redis: Arc<dyn RedisClient>,
        storage: Arc<dyn ferrinx_core::ModelStorage>,
        db: Arc<ferrinx_db::DbContext>,
        report_interval_secs: u64,
    ) -> Self {
        Self {
            worker_id,
            redis,
            storage,
            db,
            cached_models: Arc::new(std::sync::RwLock::new(HashSet::new())),
            report_interval: Duration::from_secs(report_interval_secs),
        }
    }

    pub fn with_cached_models(mut self, cached_models: CachedModelsRef) -> Self {
        self.cached_models = cached_models;
        self
    }

    pub async fn scan_available_models(&self) -> Result<HashSet<Uuid>> {
        let filter = ferrinx_common::ModelFilter {
            is_valid: Some(true),
            ..Default::default()
        };
        let models = self.db.models.list(&filter).await?;

        let mut available = HashSet::new();

        for model in models {
            if self.storage.exists(&model.file_path).await? {
                available.insert(model.id);
            }
        }

        debug!("Scanned {} available models", available.len());
        Ok(available)
    }

    pub async fn report_status(&self) -> Result<()> {
        let available_models = self.scan_available_models().await?;
        let cached_models = self.cached_models.read().unwrap().clone();

        let mut status = std::collections::HashMap::new();
        for model_id in &available_models {
            if cached_models.contains(model_id) {
                status.insert(model_id.to_string(), "cached".to_string());
            } else {
                status.insert(model_id.to_string(), "available".to_string());
            }
        }

        self.redis.set_worker_heartbeat(&self.worker_id).await?;
        self.redis
            .set_worker_models(&self.worker_id, &status)
            .await?;

        debug!(
            "Reported model status: {} total, {} cached, {} available",
            status.len(),
            cached_models.len(),
            status.len() - cached_models.len()
        );

        Ok(())
    }

    pub async fn run(self: Arc<Self>, shutdown: tokio_util::sync::CancellationToken) {
        info!("Model reporter started for worker {}", self.worker_id);

        if let Err(e) = self.report_status().await {
            error!("Initial model status report failed: {}", e);
        }

        let mut interval = tokio::time::interval(self.report_interval);

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Model reporter shutting down");

                    if let Err(e) = self.redis.remove_worker_models(&self.worker_id).await {
                        warn!("Failed to remove worker models on shutdown: {}", e);
                    }

                    break;
                }

                _ = interval.tick() => {
                    if let Err(e) = self.report_status().await {
                        error!("Model status report failed: {}", e);
                    }
                }
            }
        }

        info!("Model reporter stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ferrinx_db::DbContext;
    use std::collections::HashMap;

    struct MockRedis {
        worker_models: std::sync::RwLock<HashMap<String, HashMap<String, String>>>,
        heartbeats: std::sync::RwLock<HashSet<String>>,
    }

    impl MockRedis {
        fn new() -> Self {
            Self {
                worker_models: std::sync::RwLock::new(HashMap::new()),
                heartbeats: std::sync::RwLock::new(HashSet::new()),
            }
        }

        fn get_worker_models(&self, worker_id: &str) -> Option<HashMap<String, String>> {
            self.worker_models.read().unwrap().get(worker_id).cloned()
        }

        fn has_heartbeat(&self, worker_id: &str) -> bool {
            self.heartbeats.read().unwrap().contains(worker_id)
        }
    }

    #[async_trait]
    impl RedisClient for MockRedis {
        async fn xread_group(
            &self,
            _group: &str,
            _consumer: &str,
            _stream: &str,
            _count: usize,
            _block_ms: u64,
        ) -> crate::error::Result<Option<Vec<crate::redis::StreamEntry>>> {
            Ok(None)
        }

        async fn xack(&self, _stream: &str, _group: &str, _entry_id: &str) -> crate::error::Result<()> {
            Ok(())
        }

        async fn xpending(
            &self,
            _stream: &str,
            _group: &str,
            _count: usize,
        ) -> crate::error::Result<Vec<crate::redis::PendingInfo>> {
            Ok(Vec::new())
        }

        async fn xclaim(
            &self,
            _stream: &str,
            _group: &str,
            _consumer: &str,
            _min_idle_ms: i64,
            _entry_ids: &[&str],
        ) -> crate::error::Result<Vec<crate::redis::StreamEntry>> {
            Ok(Vec::new())
        }

        async fn xadd(
            &self,
            _stream: &str,
            _data: &HashMap<String, String>,
        ) -> crate::error::Result<String> {
            Ok("test-entry-id".to_string())
        }

        async fn set_json(
            &self,
            _key: &str,
            _value: &serde_json::Value,
            _ttl: Duration,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        async fn get_json(&self, _key: &str) -> crate::error::Result<Option<serde_json::Value>> {
            Ok(None)
        }

        async fn del(&self, _key: &str) -> crate::error::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> crate::error::Result<()> {
            Ok(())
        }

        async fn set_worker_heartbeat(&self, worker_id: &str) -> crate::error::Result<()> {
            self.heartbeats.write().unwrap().insert(worker_id.to_string());
            Ok(())
        }

        async fn set_worker_models(
            &self,
            worker_id: &str,
            models: &HashMap<String, String>,
        ) -> crate::error::Result<()> {
            self.worker_models
                .write()
                .unwrap()
                .insert(worker_id.to_string(), models.clone());
            Ok(())
        }

        async fn get_worker_models(
            &self,
            _worker_id: &str,
        ) -> crate::error::Result<HashMap<String, String>> {
            Ok(HashMap::new())
        }

        async fn get_model_workers(&self, _model_id: &Uuid) -> crate::error::Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn remove_worker_models(&self, worker_id: &str) -> crate::error::Result<()> {
            self.worker_models.write().unwrap().remove(worker_id);
            Ok(())
        }
    }

    struct MockStorage {
        existing_files: std::sync::RwLock<HashSet<String>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                existing_files: std::sync::RwLock::new(HashSet::new()),
            }
        }

        #[allow(dead_code)]
        fn add_file(&self, path: &str) {
            self.existing_files.write().unwrap().insert(path.to_string());
        }
    }

    #[async_trait]
    impl ferrinx_core::ModelStorage for MockStorage {
        async fn exists(&self, path: &str) -> std::result::Result<bool, ferrinx_core::StorageError> {
            Ok(self.existing_files.read().unwrap().contains(path))
        }

        async fn save(
            &self,
            _name: &str,
            _data: &[u8],
        ) -> std::result::Result<String, ferrinx_core::StorageError> {
            Ok("test-path".to_string())
        }

        async fn load(&self, _path: &str) -> std::result::Result<Vec<u8>, ferrinx_core::StorageError> {
            Ok(Vec::new())
        }

        async fn delete(&self, _path: &str) -> std::result::Result<(), ferrinx_core::StorageError> {
            Ok(())
        }

        async fn size(&self, _path: &str) -> std::result::Result<u64, ferrinx_core::StorageError> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn test_model_reporter_new() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());
        let redis = Arc::new(MockRedis::new());
        let storage = Arc::new(MockStorage::new());

        let reporter = ModelReporter::new(
            "worker-1".to_string(),
            redis,
            storage,
            db,
            60,
        );

        assert_eq!(reporter.worker_id, "worker-1");
        assert_eq!(reporter.report_interval, Duration::from_secs(60));
    }

    #[test]
    fn test_model_reporter_with_cached_models() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = ferrinx_common::DatabaseConfig {
                backend: ferrinx_common::DatabaseBackend::Sqlite,
                url: ":memory:".to_string(),
                max_connections: 1,
                run_migrations: true,
            };
            let db = Arc::new(DbContext::new(&config).await.unwrap());
            let redis = Arc::new(MockRedis::new());
            let storage = Arc::new(MockStorage::new());

            let cached_models: CachedModelsRef = Arc::new(std::sync::RwLock::new(HashSet::new()));
            cached_models.write().unwrap().insert(Uuid::new_v4());

            let reporter = ModelReporter::new(
                "worker-1".to_string(),
                redis,
                storage,
                db,
                60,
            )
            .with_cached_models(cached_models.clone());

            assert_eq!(
                Arc::strong_count(&cached_models),
                2
            );
            
            let _ = reporter;
        });
    }

    #[tokio::test]
    async fn test_scan_available_models_empty_db() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());
        let redis = Arc::new(MockRedis::new());
        let storage = Arc::new(MockStorage::new());

        let reporter = ModelReporter::new(
            "worker-1".to_string(),
            redis,
            storage,
            db,
            60,
        );

        let models = reporter.scan_available_models().await.unwrap();
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn test_report_status_empty_db() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());
        let redis = Arc::new(MockRedis::new());
        let storage = Arc::new(MockStorage::new());

        let reporter = ModelReporter::new(
            "worker-1".to_string(),
            redis.clone(),
            storage,
            db,
            60,
        );

        let result = reporter.report_status().await;
        assert!(result.is_ok());

        assert!(redis.has_heartbeat("worker-1"));
        let models = redis.get_worker_models("worker-1");
        assert!(models.is_some());
        assert!(models.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_report_status_with_cached_models() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());
        let redis = Arc::new(MockRedis::new());
        let storage = Arc::new(MockStorage::new());

        let cached_models: CachedModelsRef = Arc::new(std::sync::RwLock::new(HashSet::new()));
        let model_id = Uuid::new_v4();
        cached_models.write().unwrap().insert(model_id);

        let reporter = ModelReporter::new(
            "worker-1".to_string(),
            redis.clone(),
            storage,
            db,
            60,
        )
        .with_cached_models(cached_models);

        let result = reporter.report_status().await;
        assert!(result.is_ok());

        assert!(redis.has_heartbeat("worker-1"));
    }

    #[test]
    fn test_cached_models_ref_type() {
        let cached_models: CachedModelsRef = Arc::new(std::sync::RwLock::new(HashSet::new()));
        let model_id = Uuid::new_v4();

        cached_models.write().unwrap().insert(model_id);

        assert!(cached_models.read().unwrap().contains(&model_id));
    }
}
