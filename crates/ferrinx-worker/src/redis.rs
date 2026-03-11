use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: String,
    pub data: HashMap<String, String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PendingInfo {
    pub id: String,
    pub consumer: String,
    pub idle_time_ms: i64,
    pub deliveries: i64,
}

#[async_trait]
#[allow(dead_code)]
pub trait RedisClient: Send + Sync {
    async fn xread_group(
        &self,
        group: &str,
        consumer: &str,
        stream: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Option<Vec<StreamEntry>>>;

    async fn xack(&self, stream: &str, group: &str, entry_id: &str) -> Result<()>;

    async fn xpending(
        &self,
        stream: &str,
        group: &str,
        count: usize,
    ) -> Result<Vec<PendingInfo>>;

    async fn xclaim(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        min_idle_ms: i64,
        entry_ids: &[&str],
    ) -> Result<Vec<StreamEntry>>;

    async fn xadd(
        &self,
        stream: &str,
        data: &HashMap<String, String>,
    ) -> Result<String>;

    async fn set_json(
        &self,
        key: &str,
        value: &serde_json::Value,
        ttl: Duration,
    ) -> Result<()>;

    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>>;

    async fn del(&self, key: &str) -> Result<()>;

    async fn health_check(&self) -> Result<()>;

    async fn set_worker_heartbeat(&self, worker_id: &str) -> Result<()>;

    async fn set_worker_models(&self, worker_id: &str, models: &HashMap<String, String>) -> Result<()>;

    async fn get_worker_models(&self, worker_id: &str) -> Result<HashMap<String, String>>;

    async fn get_model_workers(&self, model_id: &Uuid) -> Result<Vec<String>>;

    async fn remove_worker_models(&self, worker_id: &str) -> Result<()>;
}

pub fn create_redis_client(url: &str) -> Result<std::sync::Arc<dyn RedisClient>> {
    let config = ferrinx_common::RedisPoolConfig {
        url: url.to_string(),
        pool_size: 10,
        connection_timeout: std::time::Duration::from_secs(5),
        api_key_cache_ttl: 3600,
        result_cache_ttl: 86400,
        task_timeout_ms: 300000,
    };

    let rt = tokio::runtime::Handle::current();
    let client = rt.block_on(async {
        ferrinx_common::RedisClient::new(config).await
    }).map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))?;

    Ok(std::sync::Arc::new(RealRedisClientAdapter(client)))
}

pub struct RealRedisClientAdapter(ferrinx_common::RedisClient);

#[async_trait]
impl RedisClient for RealRedisClientAdapter {
    async fn xread_group(
        &self,
        _group: &str,
        consumer: &str,
        _stream: &str,
        _count: usize,
        _block_ms: u64,
    ) -> Result<Option<Vec<StreamEntry>>> {
        let task = self.0.consume_task(consumer).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))?;

        Ok(task.map(|t| {
            let mut data = std::collections::HashMap::new();
            data.insert("task_id".to_string(), t.task_id.to_string());
            data.insert("model_id".to_string(), t.model_id.to_string());
            data.insert("user_id".to_string(), t.user_id.to_string());
            data.insert("api_key_id".to_string(), t.api_key_id.to_string());
            data.insert("priority".to_string(), t.priority.to_string());
            data.insert("created_at".to_string(), t.created_at);
            if let Some(inputs) = t.inputs {
                if let Ok(json) = serde_json::to_string(&inputs) {
                    data.insert("inputs".to_string(), json);
                }
            }
            vec![StreamEntry {
                id: t.entry_id,
                data,
            }]
        }))
    }

    async fn xack(&self, stream: &str, _group: &str, entry_id: &str) -> Result<()> {
        self.0.ack_task(stream, entry_id).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn xpending(
        &self,
        _stream: &str,
        _group: &str,
        _count: usize,
    ) -> Result<Vec<PendingInfo>> {
        Ok(Vec::new())
    }

    async fn xclaim(
        &self,
        _stream: &str,
        _group: &str,
        consumer: &str,
        _min_idle_ms: i64,
        _entry_ids: &[&str],
    ) -> Result<Vec<StreamEntry>> {
        let tasks = self.0.claim_pending_tasks(consumer).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))?;

        Ok(tasks.into_iter().map(|t| {
            let mut data = std::collections::HashMap::new();
            data.insert("task_id".to_string(), t.task_id.to_string());
            data.insert("model_id".to_string(), t.model_id.to_string());
            data.insert("user_id".to_string(), t.user_id.to_string());
            data.insert("api_key_id".to_string(), t.api_key_id.to_string());
            data.insert("priority".to_string(), t.priority.to_string());
            data.insert("created_at".to_string(), t.created_at);
            if let Some(inputs) = t.inputs {
                if let Ok(json) = serde_json::to_string(&inputs) {
                    data.insert("inputs".to_string(), json);
                }
            }
            StreamEntry {
                id: t.entry_id,
                data,
            }
        }).collect())
    }

    async fn xadd(
        &self,
        stream: &str,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<String> {
        self.0.xadd(stream, data).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn set_json(
        &self,
        key: &str,
        value: &serde_json::Value,
        ttl: std::time::Duration,
    ) -> Result<()> {
        self.0.set_cache(key, value, ttl.as_secs()).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>> {
        self.0.get_cache(key).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn del(&self, key: &str) -> Result<()> {
        self.0.delete_cache(key).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn health_check(&self) -> Result<()> {
        self.0.health_check().await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn set_worker_heartbeat(&self, worker_id: &str) -> Result<()> {
        self.0.set_worker_heartbeat(worker_id).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn set_worker_models(&self, worker_id: &str, models: &HashMap<String, String>) -> Result<()> {
        self.0.set_worker_models(worker_id, models).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn get_worker_models(&self, worker_id: &str) -> Result<HashMap<String, String>> {
        self.0.get_worker_models(worker_id).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn get_model_workers(&self, model_id: &Uuid) -> Result<Vec<String>> {
        self.0.get_model_workers(model_id).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }

    async fn remove_worker_models(&self, worker_id: &str) -> Result<()> {
        self.0.remove_worker_models(worker_id).await
            .map_err(|e| crate::error::WorkerError::RedisError(e.to_string()))
    }
}
