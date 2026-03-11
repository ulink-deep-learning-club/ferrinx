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
        self.redis.set_worker_models(&self.worker_id, &status).await?;
        
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
