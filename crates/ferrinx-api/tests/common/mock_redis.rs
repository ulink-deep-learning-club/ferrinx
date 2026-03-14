use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

type Result<T> = std::result::Result<T, MockRedisError>;

#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: String,
    pub data: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PendingInfo {
    pub id: String,
    pub consumer: String,
    pub idle_time_ms: i64,
    pub deliveries: i64,
}

#[async_trait]
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

    async fn xpending(&self, stream: &str, group: &str, count: usize) -> Result<Vec<PendingInfo>>;

    async fn xclaim(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        min_idle_ms: i64,
        entry_ids: &[&str],
    ) -> Result<Vec<StreamEntry>>;

    async fn xadd(&self, stream: &str, data: &HashMap<String, String>) -> Result<String>;

    async fn set_json(&self, key: &str, value: &serde_json::Value, ttl: Duration) -> Result<()>;

    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>>;

    async fn del(&self, key: &str) -> Result<()>;

    async fn health_check(&self) -> Result<()>;
}

pub struct MockRedis {
    results: Arc<tokio::sync::RwLock<HashMap<String, serde_json::Value>>>,
    streams: Arc<tokio::sync::RwLock<HashMap<String, Vec<StreamEntry>>>>,
    pending: Arc<tokio::sync::RwLock<HashMap<String, Vec<PendingInfo>>>>,
    acked: Arc<tokio::sync::RwLock<Vec<String>>>,
}

impl MockRedis {
    pub fn new() -> Self {
        Self {
            results: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            streams: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            pending: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            acked: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    pub async fn add_task(&self, stream: &str, task_id: &str) -> String {
        let mut streams = self.streams.write().await;
        let id = format!("{}-0", chrono::Utc::now().timestamp_millis());
        let entry = StreamEntry {
            id: id.clone(),
            data: HashMap::from([("task_id".to_string(), task_id.to_string())]),
        };
        streams.entry(stream.to_string()).or_default().push(entry);
        id
    }

    pub async fn add_task_with_data(&self, stream: &str, data: HashMap<String, String>) -> String {
        let mut streams = self.streams.write().await;
        let id = format!("{}-0", chrono::Utc::now().timestamp_millis());
        let entry = StreamEntry {
            id: id.clone(),
            data,
        };
        streams.entry(stream.to_string()).or_default().push(entry);
        id
    }

    pub async fn get_acked(&self) -> Vec<String> {
        self.acked.read().await.clone()
    }

    pub async fn get_stream_count(&self, stream: &str) -> usize {
        let streams = self.streams.read().await;
        streams.get(stream).map(|v| v.len()).unwrap_or(0)
    }

    pub async fn set_result(&self, task_id: &str, result: serde_json::Value) {
        let mut results = self.results.write().await;
        results.insert(format!("ferrinx:results:{}", task_id), result);
    }

    pub async fn get_result(&self, task_id: &str) -> Option<serde_json::Value> {
        let results = self.results.read().await;
        results
            .get(&format!("ferrinx:results:{}", task_id))
            .cloned()
    }

    pub async fn clear(&self) {
        self.streams.write().await.clear();
        self.results.write().await.clear();
        self.pending.write().await.clear();
        self.acked.write().await.clear();
    }
}

impl Default for MockRedis {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MockRedisError {
    #[error("Mock error: {0}")]
    Error(String),
}

#[async_trait]
impl RedisClient for MockRedis {
    async fn xread_group(
        &self,
        _group: &str,
        _consumer: &str,
        stream: &str,
        count: usize,
        _block_ms: u64,
    ) -> Result<Option<Vec<StreamEntry>>> {
        let mut streams = self.streams.write().await;
        if let Some(entries) = streams.get_mut(stream) {
            let result: Vec<StreamEntry> = entries.drain(..count.min(entries.len())).collect();
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        } else {
            Ok(None)
        }
    }

    async fn xack(&self, _stream: &str, _group: &str, entry_id: &str) -> Result<()> {
        self.acked.write().await.push(entry_id.to_string());
        Ok(())
    }

    async fn xpending(&self, stream: &str, _group: &str, count: usize) -> Result<Vec<PendingInfo>> {
        let pending = self.pending.read().await;
        Ok(pending
            .get(stream)
            .map(|p| p.iter().take(count).cloned().collect())
            .unwrap_or_default())
    }

    async fn xclaim(
        &self,
        stream: &str,
        _group: &str,
        _consumer: &str,
        _min_idle_ms: i64,
        entry_ids: &[&str],
    ) -> Result<Vec<StreamEntry>> {
        let streams = self.streams.read().await;
        if let Some(entries) = streams.get(stream) {
            let claimed: Vec<StreamEntry> = entries
                .iter()
                .filter(|e| entry_ids.contains(&e.id.as_str()))
                .cloned()
                .collect();
            Ok(claimed)
        } else {
            Ok(Vec::new())
        }
    }

    async fn xadd(&self, stream: &str, data: &HashMap<String, String>) -> Result<String> {
        let mut streams = self.streams.write().await;
        let id = format!("{}-0", chrono::Utc::now().timestamp_millis());
        let entry = StreamEntry {
            id: id.clone(),
            data: data.clone(),
        };
        streams.entry(stream.to_string()).or_default().push(entry);
        Ok(id)
    }

    async fn set_json(&self, key: &str, value: &serde_json::Value, _ttl: Duration) -> Result<()> {
        let mut results = self.results.write().await;
        results.insert(key.to_string(), value.clone());
        Ok(())
    }

    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let results = self.results.read().await;
        Ok(results.get(key).cloned())
    }

    async fn del(&self, key: &str) -> Result<()> {
        let mut results = self.results.write().await;
        results.remove(key);
        Ok(())
    }

    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}
