use crate::constants::{
    REDIS_API_KEY_STORE, REDIS_CONSUMER_GROUP, REDIS_RESULT_CACHE_PREFIX, REDIS_STREAM_KEY_HIGH,
    REDIS_STREAM_KEY_LOW, REDIS_STREAM_KEY_NORMAL,
};
use crate::types::{ApiKeyInfo, InferenceTask};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client as RedisClientInner, RedisResult, Value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("Redis connection error: {0}")]
    Connection(#[from] redis::RedisError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Stream entry not found")]
    EntryNotFound,

    #[error("Consumer group not found")]
    GroupNotFound,
}

#[derive(Debug, Clone)]
pub struct RedisPoolConfig {
    pub url: String,
    pub pool_size: usize,
    pub connection_timeout: Duration,
    pub api_key_cache_ttl: u64,
    pub result_cache_ttl: u64,
    pub task_timeout_ms: u64,
}

impl Default for RedisPoolConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379".to_string(),
            pool_size: 10,
            connection_timeout: Duration::from_secs(5),
            api_key_cache_ttl: 3600,
            result_cache_ttl: 86400,
            task_timeout_ms: 300000,
        }
    }
}

pub struct RedisClient {
    conn: ConnectionManager,
    config: RedisPoolConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    pub stream: String,
    pub entry_id: String,
    pub task_id: Uuid,
    pub model_id: Uuid,
    pub user_id: Uuid,
    pub api_key_id: Uuid,
    pub priority: i32,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<serde_json::Value>,
}

impl RedisClient {
    pub async fn new(config: RedisPoolConfig) -> Result<Self, RedisError> {
        let client = RedisClientInner::open(config.url.as_str())?;
        let conn = ConnectionManager::new(client).await?;

        Ok(Self { conn, config })
    }

    pub async fn health_check(&self) -> Result<(), RedisError> {
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn xadd(
        &self,
        stream: &str,
        data: &HashMap<String, String>,
    ) -> Result<String, RedisError> {
        let mut cmd = redis::cmd("XADD");
        cmd.arg(stream).arg("*");
        for (key, value) in data {
            cmd.arg(key).arg(value);
        }

        let mut conn = self.conn.clone();
        let id: String = cmd.query_async(&mut conn).await?;

        Ok(id)
    }

    pub async fn initialize_consumer_groups(&self) -> Result<(), RedisError> {
        let streams = [
            REDIS_STREAM_KEY_HIGH,
            REDIS_STREAM_KEY_NORMAL,
            REDIS_STREAM_KEY_LOW,
        ];

        for stream in streams.iter() {
            let mut conn = self.conn.clone();
            let result: RedisResult<()> = redis::cmd("XGROUP")
                .arg("CREATE")
                .arg(stream)
                .arg(REDIS_CONSUMER_GROUP)
                .arg("0")
                .arg("MKSTREAM")
                .query_async(&mut conn)
                .await;

            if let Err(e) = result {
                if !e.to_string().contains("BUSYGROUP") {
                    return Err(RedisError::Connection(e));
                }
                debug!("Consumer group already exists for stream: {}", stream);
            } else {
                debug!("Created consumer group for stream: {}", stream);
            }
        }

        Ok(())
    }

    pub async fn push_task(&self, task: &InferenceTask) -> Result<String, RedisError> {
        let stream_key = task.priority_enum().stream_key();
        let mut conn = self.conn.clone();

        let task_id = task.id.to_string();
        let model_id = task.model_id.to_string();
        let user_id = task.user_id.to_string();
        let api_key_id = task.api_key_id.to_string();
        let priority = task.priority.to_string();
        let created_at = task.created_at.to_rfc3339();
        let inputs_json = serde_json::to_string(&task.inputs)?;

        let entry_id: String = redis::cmd("XADD")
            .arg(stream_key)
            .arg("*")
            .arg("task_id")
            .arg(&task_id)
            .arg("model_id")
            .arg(&model_id)
            .arg("user_id")
            .arg(&user_id)
            .arg("api_key_id")
            .arg(&api_key_id)
            .arg("priority")
            .arg(&priority)
            .arg("created_at")
            .arg(&created_at)
            .arg("inputs")
            .arg(&inputs_json)
            .query_async(&mut conn)
            .await?;

        debug!("Pushed task {} to stream {}", task.id, stream_key);
        Ok(entry_id)
    }

    pub async fn consume_task(&self, consumer: &str) -> Result<Option<TaskMessage>, RedisError> {
        let streams = [
            (REDIS_STREAM_KEY_HIGH, ">"),
            (REDIS_STREAM_KEY_NORMAL, ">"),
            (REDIS_STREAM_KEY_LOW, ">"),
        ];

        for (stream, id) in streams.iter() {
            if let Some(task) = self.read_from_stream(stream, consumer, id).await? {
                return Ok(Some(task));
            }
        }

        Ok(None)
    }

    async fn read_from_stream(
        &self,
        stream: &str,
        consumer: &str,
        id: &str,
    ) -> Result<Option<TaskMessage>, RedisError> {
        let mut conn = self.conn.clone();

        let result: Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(REDIS_CONSUMER_GROUP)
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("STREAMS")
            .arg(stream)
            .arg(id)
            .query_async(&mut conn)
            .await?;

        match result {
            Value::Nil => Ok(None),
            Value::Array(streams) => {
                for stream_result in streams {
                    if let Value::Array(mut parts) = stream_result {
                        if parts.len() < 2 {
                            continue;
                        }
                        let stream_name = match &parts[0] {
                            Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
                            _ => continue,
                        };
                        if stream_name != stream {
                            continue;
                        }
                        let entries = std::mem::take(&mut parts[1]);
                        if let Value::Array(entries) = entries {
                            if let Some(entry) = entries.into_iter().next() {
                                if let Value::Array(entry_parts) = entry {
                                    return Ok(Some(self.parse_stream_entry(stream, &entry_parts)?));
                                }
                            }
                        }
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn parse_stream_entry(&self, stream: &str, entry: &[Value]) -> Result<TaskMessage, RedisError> {
        if entry.len() < 2 {
            return Err(RedisError::EntryNotFound);
        }

        let entry_id = match &entry[0] {
            Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
            _ => return Err(RedisError::EntryNotFound),
        };

        let fields = match &entry[1] {
            Value::Array(arr) => arr,
            _ => return Err(RedisError::EntryNotFound),
        };

        let data = self.parse_fields(fields)?;

        let task_id = self.get_field_as_uuid(&data, "task_id")?;
        let model_id = self.get_field_as_uuid(&data, "model_id")?;
        let user_id = self.get_field_as_uuid(&data, "user_id")?;
        let api_key_id = self.get_field_as_uuid(&data, "api_key_id")?;
        let priority = self.get_field_as_i32(&data, "priority")?;
        let created_at = self.get_field_as_string(&data, "created_at")?;
        let inputs = data
            .get("inputs")
            .and_then(|s| serde_json::from_str(s).ok());

        Ok(TaskMessage {
            stream: stream.to_string(),
            entry_id,
            task_id,
            model_id,
            user_id,
            api_key_id,
            priority,
            created_at,
            inputs,
        })
    }

    fn parse_fields(&self, fields: &[Value]) -> Result<HashMap<String, String>, RedisError> {
        let mut map = HashMap::new();
        let mut i = 0;

        while i + 1 < fields.len() {
            let key = match &fields[i] {
                Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
                _ => {
                    i += 2;
                    continue;
                }
            };

            let value = match &fields[i + 1] {
                Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
                _ => {
                    i += 2;
                    continue;
                }
            };

            map.insert(key, value);
            i += 2;
        }

        Ok(map)
    }

    fn get_field_as_string(
        &self,
        data: &HashMap<String, String>,
        field: &str,
    ) -> Result<String, RedisError> {
        data.get(field)
            .cloned()
            .ok_or_else(|| RedisError::EntryNotFound)
    }

    fn get_field_as_uuid(
        &self,
        data: &HashMap<String, String>,
        field: &str,
    ) -> Result<Uuid, RedisError> {
        let s = self.get_field_as_string(data, field)?;
        Uuid::parse_str(&s).map_err(|_| RedisError::EntryNotFound)
    }

    fn get_field_as_i32(
        &self,
        data: &HashMap<String, String>,
        field: &str,
    ) -> Result<i32, RedisError> {
        let s = self.get_field_as_string(data, field)?;
        s.parse::<i32>().map_err(|_| RedisError::EntryNotFound)
    }

    pub async fn ack_task(&self, stream: &str, entry_id: &str) -> Result<(), RedisError> {
        let mut conn = self.conn.clone();

        let _: () = redis::cmd("XACK")
            .arg(stream)
            .arg(REDIS_CONSUMER_GROUP)
            .arg(entry_id)
            .query_async(&mut conn)
            .await?;

        debug!("Acknowledged task {} from stream {}", entry_id, stream);
        Ok(())
    }

    pub async fn claim_pending_tasks(
        &self,
        consumer: &str,
    ) -> Result<Vec<TaskMessage>, RedisError> {
        let mut tasks = Vec::new();

        let streams = [
            REDIS_STREAM_KEY_HIGH,
            REDIS_STREAM_KEY_NORMAL,
            REDIS_STREAM_KEY_LOW,
        ];

        for stream in streams.iter() {
            let pending = self.claim_pending_from_stream(stream, consumer).await?;
            tasks.extend(pending);
        }

        Ok(tasks)
    }

    async fn claim_pending_from_stream(
        &self,
        stream: &str,
        consumer: &str,
    ) -> Result<Vec<TaskMessage>, RedisError> {
        let mut conn = self.conn.clone();

        let pending: Vec<Value> = redis::cmd("XPENDING")
            .arg(stream)
            .arg(REDIS_CONSUMER_GROUP)
            .arg("-")
            .arg("+")
            .arg(10)
            .query_async(&mut conn)
            .await?;

        if pending.is_empty() {
            return Ok(Vec::new());
        }

        let entry_ids: Vec<String> = pending
            .iter()
            .filter_map(|v| {
                if let Value::Array(arr) = v {
                    arr.first().and_then(|id| match id {
                        Value::BulkString(bytes) => {
                            Some(String::from_utf8_lossy(bytes).to_string())
                        }
                        _ => None,
                    })
                } else {
                    None
                }
            })
            .collect();

        if entry_ids.is_empty() {
            return Ok(Vec::new());
        }

        let timeout_str = self.config.task_timeout_ms.to_string();
        let mut cmd = redis::cmd("XCLAIM");
        cmd.arg(stream)
            .arg(REDIS_CONSUMER_GROUP)
            .arg(consumer)
            .arg(&timeout_str);

        for id in &entry_ids {
            cmd.arg(id);
        }

        let claimed: Vec<Vec<Value>> = cmd.query_async(&mut conn).await?;

        let tasks: Vec<TaskMessage> = claimed
            .into_iter()
            .filter_map(|entry| self.parse_stream_entry(stream, &entry).ok())
            .collect();

        if !tasks.is_empty() {
            debug!(
                "Claimed {} pending tasks from stream {}",
                tasks.len(),
                stream
            );
        }

        Ok(tasks)
    }

    pub async fn get_api_key(&self, key_hash: &str) -> Result<Option<ApiKeyInfo>, RedisError> {
        let key = format!("{}:{}", REDIS_API_KEY_STORE, key_hash);
        let mut conn = self.conn.clone();

        let value: Option<String> = conn.get(&key).await?;

        if let Some(json) = value {
            let info: ApiKeyInfo = serde_json::from_str(&json)?;
            return Ok(Some(info));
        }

        Ok(None)
    }

    pub async fn set_api_key(&self, key_hash: &str, info: &ApiKeyInfo) -> Result<(), RedisError> {
        let key = format!("{}:{}", REDIS_API_KEY_STORE, key_hash);
        let json = serde_json::to_string(info)?;
        let mut conn = self.conn.clone();

        let _: () = conn
            .set_ex(&key, json, self.config.api_key_cache_ttl)
            .await?;

        debug!("Cached API key {}", key_hash);
        Ok(())
    }

    pub async fn delete_api_key(&self, key_hash: &str) -> Result<(), RedisError> {
        let key = format!("{}:{}", REDIS_API_KEY_STORE, key_hash);
        let mut conn = self.conn.clone();

        let _: () = conn.del(&key).await?;

        debug!("Deleted API key cache {}", key_hash);
        Ok(())
    }

    pub async fn set_result(
        &self,
        task_id: &Uuid,
        result: &serde_json::Value,
    ) -> Result<(), RedisError> {
        let key = format!("{}:{}", REDIS_RESULT_CACHE_PREFIX, task_id);
        let json = serde_json::to_string(result)?;
        let mut conn = self.conn.clone();

        let _: () = conn
            .set_ex(&key, json, self.config.result_cache_ttl)
            .await?;

        debug!("Cached result for task {}", task_id);
        Ok(())
    }

    pub async fn get_result(
        &self,
        task_id: &Uuid,
    ) -> Result<Option<serde_json::Value>, RedisError> {
        let key = format!("{}:{}", REDIS_RESULT_CACHE_PREFIX, task_id);
        let mut conn = self.conn.clone();

        let value: Option<String> = conn.get(&key).await?;

        if let Some(json) = value {
            let result: serde_json::Value = serde_json::from_str(&json)?;
            return Ok(Some(result));
        }

        Ok(None)
    }

    pub async fn delete_result(&self, task_id: &Uuid) -> Result<(), RedisError> {
        let key = format!("{}:{}", REDIS_RESULT_CACHE_PREFIX, task_id);
        let mut conn = self.conn.clone();

        let _: () = conn.del(&key).await?;

        debug!("Deleted result cache for task {}", task_id);
        Ok(())
    }

    pub async fn get_stream_length(&self, stream: &str) -> Result<u64, RedisError> {
        let mut conn = self.conn.clone();

        let len: u64 = redis::cmd("XLEN")
            .arg(stream)
            .query_async(&mut conn)
            .await?;

        Ok(len)
    }

    pub async fn get_all_stream_lengths(&self) -> Result<HashMap<String, u64>, RedisError> {
        let mut lengths = HashMap::new();

        let streams = [
            REDIS_STREAM_KEY_HIGH,
            REDIS_STREAM_KEY_NORMAL,
            REDIS_STREAM_KEY_LOW,
        ];

        for stream in streams.iter() {
            let len = self.get_stream_length(stream).await?;
            lengths.insert(stream.to_string(), len);
        }

        Ok(lengths)
    }

    pub async fn set_cache<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: u64,
    ) -> Result<(), RedisError> {
        let json = serde_json::to_string(value)?;
        let mut conn = self.conn.clone();

        let _: () = conn.set_ex(key, json, ttl).await?;
        Ok(())
    }

    pub async fn get_cache<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, RedisError> {
        let mut conn = self.conn.clone();

        let value: Option<String> = conn.get(key).await?;

        if let Some(json) = value {
            let result: T = serde_json::from_str(&json)?;
            return Ok(Some(result));
        }

        Ok(None)
    }

    pub async fn delete_cache(&self, key: &str) -> Result<(), RedisError> {
        let mut conn = self.conn.clone();
        let _: () = conn.del(key).await?;
        Ok(())
    }

    pub async fn set_worker_heartbeat(&self, worker_id: &str) -> Result<(), RedisError> {
        use crate::constants::{
            REDIS_WORKER_HEARTBEAT_SUFFIX, REDIS_WORKER_HEARTBEAT_TTL_SECS,
            REDIS_WORKER_MODELS_PREFIX,
        };

        let key = format!(
            "{}:{}:{}",
            REDIS_WORKER_MODELS_PREFIX, worker_id, REDIS_WORKER_HEARTBEAT_SUFFIX
        );
        let mut conn = self.conn.clone();

        let timestamp = chrono::Utc::now().to_rfc3339();
        let _: () = conn
            .set_ex(&key, timestamp, REDIS_WORKER_HEARTBEAT_TTL_SECS)
            .await?;

        debug!("Set heartbeat for worker {}", worker_id);
        Ok(())
    }

    pub async fn set_worker_models(
        &self,
        worker_id: &str,
        models: &HashMap<String, String>,
    ) -> Result<(), RedisError> {
        use crate::constants::{
            REDIS_WORKER_MODELS_PREFIX, REDIS_WORKER_MODELS_SUFFIX, REDIS_WORKER_MODELS_TTL_SECS,
        };
        use crate::types::ModelState;

        let key = format!(
            "{}:{}:{}",
            REDIS_WORKER_MODELS_PREFIX, worker_id, REDIS_WORKER_MODELS_SUFFIX
        );
        let mut conn = self.conn.clone();

        let _: () = conn.del(&key).await?;

        if !models.is_empty() {
            let mut cmd = redis::cmd("HMSET");
            cmd.arg(&key);
            for (model_id, state) in models {
                cmd.arg(model_id).arg(state);
            }
            let _: () = cmd.query_async(&mut conn).await?;

            let _: () = conn
                .expire(&key, REDIS_WORKER_MODELS_TTL_SECS as i64)
                .await?;
        }

        for (model_id, state_str) in models {
            if let Some(state) = ModelState::from_str(state_str) {
                let model_workers_key = format!(
                    "{}:{}:{}",
                    crate::constants::REDIS_MODEL_WORKERS_PREFIX,
                    model_id,
                    crate::constants::REDIS_MODEL_WORKERS_SUFFIX
                );
                let score = state.priority_score();
                let _: () = redis::cmd("ZADD")
                    .arg(&model_workers_key)
                    .arg(score)
                    .arg(worker_id)
                    .query_async(&mut conn)
                    .await?;

                let _: () = conn
                    .expire(&model_workers_key, REDIS_WORKER_MODELS_TTL_SECS as i64)
                    .await?;
            }
        }

        debug!(
            "Set models for worker {}: {} models",
            worker_id,
            models.len()
        );
        Ok(())
    }

    pub async fn get_worker_models(
        &self,
        worker_id: &str,
    ) -> Result<HashMap<String, String>, RedisError> {
        use crate::constants::{REDIS_WORKER_MODELS_PREFIX, REDIS_WORKER_MODELS_SUFFIX};

        let key = format!(
            "{}:{}:{}",
            REDIS_WORKER_MODELS_PREFIX, worker_id, REDIS_WORKER_MODELS_SUFFIX
        );
        let mut conn = self.conn.clone();

        let result: HashMap<String, String> = redis::cmd("HGETALL")
            .arg(&key)
            .query_async(&mut conn)
            .await?;

        Ok(result)
    }

    pub async fn get_model_workers(&self, model_id: &Uuid) -> Result<Vec<String>, RedisError> {
        use crate::constants::{REDIS_MODEL_WORKERS_PREFIX, REDIS_MODEL_WORKERS_SUFFIX};

        let key = format!(
            "{}:{}:{}",
            REDIS_MODEL_WORKERS_PREFIX, model_id, REDIS_MODEL_WORKERS_SUFFIX
        );
        let mut conn = self.conn.clone();

        let workers: Vec<(i64, String)> = redis::cmd("ZRANGE")
            .arg(&key)
            .arg(0)
            .arg(-1)
            .arg("WITHSCORES")
            .query_async(&mut conn)
            .await?;

        let result: Vec<String> = workers
            .into_iter()
            .map(|(_, worker_id)| worker_id)
            .collect();

        debug!("Found {} workers for model {}", result.len(), model_id);
        Ok(result)
    }

    pub async fn get_best_worker_for_model(
        &self,
        model_id: &Uuid,
    ) -> Result<Option<String>, RedisError> {
        use crate::constants::{REDIS_MODEL_WORKERS_PREFIX, REDIS_MODEL_WORKERS_SUFFIX};

        let key = format!(
            "{}:{}:{}",
            REDIS_MODEL_WORKERS_PREFIX, model_id, REDIS_MODEL_WORKERS_SUFFIX
        );
        let mut conn = self.conn.clone();

        let workers: Vec<String> = redis::cmd("ZRANGE")
            .arg(&key)
            .arg(0)
            .arg(0)
            .query_async(&mut conn)
            .await?;

        Ok(workers.into_iter().next())
    }

    pub async fn remove_worker_models(&self, worker_id: &str) -> Result<(), RedisError> {
        use crate::constants::{
            REDIS_WORKER_HEARTBEAT_SUFFIX, REDIS_WORKER_MODELS_PREFIX, REDIS_WORKER_MODELS_SUFFIX,
        };

        let models = self.get_worker_models(worker_id).await?;

        let models_key = format!(
            "{}:{}:{}",
            REDIS_WORKER_MODELS_PREFIX, worker_id, REDIS_WORKER_MODELS_SUFFIX
        );
        let heartbeat_key = format!(
            "{}:{}:{}",
            REDIS_WORKER_MODELS_PREFIX, worker_id, REDIS_WORKER_HEARTBEAT_SUFFIX
        );

        let mut conn = self.conn.clone();
        let _: () = conn.del(&models_key).await?;
        let _: () = conn.del(&heartbeat_key).await?;

        for model_id in models.keys() {
            let model_workers_key = format!(
                "{}:{}:{}",
                crate::constants::REDIS_MODEL_WORKERS_PREFIX,
                model_id,
                crate::constants::REDIS_MODEL_WORKERS_SUFFIX
            );
            let _: () = redis::cmd("ZREM")
                .arg(&model_workers_key)
                .arg(worker_id)
                .query_async(&mut conn)
                .await?;
        }

        debug!("Removed worker {} models", worker_id);
        Ok(())
    }

    pub async fn push_task_to_worker(
        &self,
        worker_id: &str,
        task: &InferenceTask,
    ) -> Result<String, RedisError> {
        let stream_key = format!("ferrinx:worker:{}:tasks", worker_id);
        let mut conn = self.conn.clone();

        let task_id = task.id.to_string();
        let model_id = task.model_id.to_string();
        let user_id = task.user_id.to_string();
        let api_key_id = task.api_key_id.to_string();
        let priority = task.priority.to_string();
        let created_at = task.created_at.to_rfc3339();
        let inputs_json = serde_json::to_string(&task.inputs)?;

        let entry_id: String = redis::cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("task_id")
            .arg(&task_id)
            .arg("model_id")
            .arg(&model_id)
            .arg("user_id")
            .arg(&user_id)
            .arg("api_key_id")
            .arg(&api_key_id)
            .arg("priority")
            .arg(&priority)
            .arg("created_at")
            .arg(&created_at)
            .arg("inputs")
            .arg(&inputs_json)
            .query_async(&mut conn)
            .await?;

        debug!("Pushed task {} to worker stream {}", task.id, stream_key);
        Ok(entry_id)
    }
}

pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_api_key() {
        let key = "frx_sk_test123";
        let hash = hash_api_key(key);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, key);
    }

    #[test]
    fn test_redis_config_default() {
        let config = RedisPoolConfig::default();
        assert_eq!(config.url, "redis://127.0.0.1:6379");
        assert_eq!(config.pool_size, 10);
        assert_eq!(config.api_key_cache_ttl, 3600);
        assert_eq!(config.result_cache_ttl, 86400);
    }
}
