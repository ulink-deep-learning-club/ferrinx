use std::sync::Arc;

use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{Result, WorkerError};
use crate::redis::{PendingInfo, RedisClient};

#[derive(Debug, Clone)]
pub struct TaskMessage {
    pub stream: String,
    pub entry_id: String,
    pub data: std::collections::HashMap<String, String>,
}

impl TaskMessage {
    pub fn task_id(&self) -> Result<Uuid> {
        self.data
            .get("task_id")
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or(WorkerError::InvalidTaskMessage)
    }
}

pub struct TaskConsumer {
    redis: Arc<dyn RedisClient>,
    consumer_name: String,
    group_name: String,
    streams: Vec<String>,
    claim_idle_ms: i64,
}

impl TaskConsumer {
    pub fn new(
        redis: Arc<dyn RedisClient>,
        consumer_name: String,
        group_name: String,
        streams: Vec<String>,
    ) -> Self {
        Self {
            redis,
            consumer_name,
            group_name,
            streams,
            claim_idle_ms: 300_000,
        }
    }

    pub fn with_claim_idle_ms(mut self, claim_idle_ms: i64) -> Self {
        self.claim_idle_ms = claim_idle_ms;
        self
    }

    pub async fn poll_task(&self) -> Result<Option<TaskMessage>> {
        for stream in &self.streams {
            if let Some(task) = self.read_from_stream(stream).await? {
                return Ok(Some(task));
            }
        }
        Ok(None)
    }

    async fn read_from_stream(&self, stream: &str) -> Result<Option<TaskMessage>> {
        let entries = self
            .redis
            .xread_group(&self.group_name, &self.consumer_name, stream, 1, 0)
            .await?;

        if let Some(entries) = entries {
            if let Some(entry) = entries.into_iter().next() {
                return Ok(Some(TaskMessage {
                    stream: stream.to_string(),
                    entry_id: entry.id,
                    data: entry.data,
                }));
            }
        }

        Ok(None)
    }

    pub async fn ack_task(&self, stream: &str, entry_id: &str) -> Result<()> {
        self.redis.xack(stream, &self.group_name, entry_id).await?;
        debug!("Acknowledged task {} in stream {}", entry_id, stream);
        Ok(())
    }

    pub async fn claim_pending_tasks(&self) -> Result<Vec<TaskMessage>> {
        let mut tasks = Vec::new();

        for stream in &self.streams {
            match self.claim_pending_from_stream(stream).await {
                Ok(mut pending) => tasks.append(&mut pending),
                Err(e) => {
                    error!("Failed to claim pending tasks from {}: {}", stream, e);
                }
            }
        }

        if !tasks.is_empty() {
            info!("Claimed {} pending tasks", tasks.len());
        }

        Ok(tasks)
    }

    async fn claim_pending_from_stream(&self, stream: &str) -> Result<Vec<TaskMessage>> {
        let pending = self.redis.xpending(stream, &self.group_name, 10).await?;

        if pending.is_empty() {
            return Ok(Vec::new());
        }

        let stale: Vec<&PendingInfo> = pending
            .iter()
            .filter(|p| p.idle_time_ms > self.claim_idle_ms)
            .collect();

        if stale.is_empty() {
            return Ok(Vec::new());
        }

        let entry_ids: Vec<&str> = stale.iter().map(|p| p.id.as_str()).collect();
        let claimed = self
            .redis
            .xclaim(
                stream,
                &self.group_name,
                &self.consumer_name,
                self.claim_idle_ms,
                &entry_ids,
            )
            .await?;

        warn!("Claimed {} stale tasks from {}", claimed.len(), stream);

        Ok(claimed
            .into_iter()
            .map(|entry| TaskMessage {
                stream: stream.to_string(),
                entry_id: entry.id,
                data: entry.data,
            })
            .collect())
    }

    pub async fn health_check(&self) -> Result<()> {
        self.redis.health_check().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;

    struct MockRedis {
        streams: std::sync::RwLock<HashMap<String, Vec<crate::redis::StreamEntry>>>,
        pending: std::sync::RwLock<HashMap<String, Vec<PendingInfo>>>,
        acked: std::sync::RwLock<Vec<String>>,
        should_fail: std::sync::RwLock<bool>,
    }

    impl MockRedis {
        fn new() -> Self {
            Self {
                streams: std::sync::RwLock::new(HashMap::new()),
                pending: std::sync::RwLock::new(HashMap::new()),
                acked: std::sync::RwLock::new(Vec::new()),
                should_fail: std::sync::RwLock::new(false),
            }
        }

        fn add_task(&self, stream: &str, task_id: &str) {
            let mut streams = self.streams.write().unwrap();
            let entry = crate::redis::StreamEntry {
                id: format!("{}-0", chrono::Utc::now().timestamp_millis()),
                data: HashMap::from([("task_id".to_string(), task_id.to_string())]),
            };
            streams.entry(stream.to_string()).or_default().push(entry);
        }

        fn add_pending(&self, stream: &str, pending_info: PendingInfo) {
            let mut pending = self.pending.write().unwrap();
            pending.entry(stream.to_string()).or_default().push(pending_info);
        }

        fn set_should_fail(&self, should_fail: bool) {
            *self.should_fail.write().unwrap() = should_fail;
        }

        fn get_acked(&self) -> Vec<String> {
            self.acked.read().unwrap().clone()
        }
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
        ) -> Result<Option<Vec<crate::redis::StreamEntry>>> {
            if *self.should_fail.read().unwrap() {
                return Err(WorkerError::RedisError("Mock error".to_string()));
            }

            let streams = self.streams.read().unwrap();
            if let Some(entries) = streams.get(stream) {
                let entries: Vec<_> = entries.iter().take(count).cloned().collect();
                if entries.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(entries))
                }
            } else {
                Ok(None)
            }
        }

        async fn xack(&self, stream: &str, _group: &str, entry_id: &str) -> Result<()> {
            if *self.should_fail.read().unwrap() {
                return Err(WorkerError::RedisError("Mock error".to_string()));
            }

            self.acked.write().unwrap().push(format!("{}:{}", stream, entry_id));
            Ok(())
        }

        async fn xpending(
            &self,
            stream: &str,
            _group: &str,
            count: usize,
        ) -> Result<Vec<PendingInfo>> {
            if *self.should_fail.read().unwrap() {
                return Err(WorkerError::RedisError("Mock error".to_string()));
            }

            let pending = self.pending.read().unwrap();
            if let Some(entries) = pending.get(stream) {
                Ok(entries.iter().take(count).cloned().collect())
            } else {
                Ok(Vec::new())
            }
        }

        async fn xclaim(
            &self,
            stream: &str,
            _group: &str,
            _consumer: &str,
            _min_idle_ms: i64,
            entry_ids: &[&str],
        ) -> Result<Vec<crate::redis::StreamEntry>> {
            if *self.should_fail.read().unwrap() {
                return Err(WorkerError::RedisError("Mock error".to_string()));
            }

            let streams = self.streams.read().unwrap();
            if let Some(entries) = streams.get(stream) {
                let claimed: Vec<_> = entries
                    .iter()
                    .filter(|e| entry_ids.contains(&e.id.as_str()))
                    .cloned()
                    .collect();
                Ok(claimed)
            } else {
                Ok(Vec::new())
            }
        }

        async fn xadd(
            &self,
            _stream: &str,
            _data: &HashMap<String, String>,
        ) -> Result<String> {
            Ok("test-entry-id".to_string())
        }

        async fn set_json(
            &self,
            _key: &str,
            _value: &serde_json::Value,
            _ttl: std::time::Duration,
        ) -> Result<()> {
            Ok(())
        }

        async fn get_json(&self, _key: &str) -> Result<Option<serde_json::Value>> {
            Ok(None)
        }

        async fn del(&self, _key: &str) -> Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> Result<()> {
            if *self.should_fail.read().unwrap() {
                return Err(WorkerError::RedisError("Health check failed".to_string()));
            }
            Ok(())
        }

        async fn set_worker_heartbeat(&self, _worker_id: &str) -> Result<()> {
            Ok(())
        }

        async fn set_worker_models(
            &self,
            _worker_id: &str,
            _models: &HashMap<String, String>,
        ) -> Result<()> {
            Ok(())
        }

        async fn get_worker_models(
            &self,
            _worker_id: &str,
        ) -> Result<HashMap<String, String>> {
            Ok(HashMap::new())
        }

        async fn get_model_workers(&self, _model_id: &Uuid) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn remove_worker_models(&self, _worker_id: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_consumer_new() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        assert_eq!(consumer.consumer_name, "test-consumer");
        assert_eq!(consumer.group_name, "test-group");
        assert_eq!(consumer.streams, vec!["stream1"]);
        assert_eq!(consumer.claim_idle_ms, 300_000);
    }

    #[test]
    fn test_consumer_with_claim_idle_ms() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        )
        .with_claim_idle_ms(600_000);

        assert_eq!(consumer.claim_idle_ms, 600_000);
    }

    #[tokio::test]
    async fn test_poll_task_no_tasks() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.poll_task().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_poll_task_with_task() {
        let redis = Arc::new(MockRedis::new());
        let task_id = Uuid::new_v4().to_string();
        redis.add_task("stream1", &task_id);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.poll_task().await.unwrap();
        assert!(result.is_some());

        let task = result.unwrap();
        assert_eq!(task.stream, "stream1");
        assert_eq!(task.data.get("task_id").unwrap(), &task_id);
    }

    #[tokio::test]
    async fn test_poll_task_priority_order() {
        let redis = Arc::new(MockRedis::new());

        let high_task_id = Uuid::new_v4().to_string();
        let normal_task_id = Uuid::new_v4().to_string();

        redis.add_task("stream-normal", &normal_task_id);
        redis.add_task("stream-high", &high_task_id);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream-high".to_string(), "stream-normal".to_string()],
        );

        let result = consumer.poll_task().await.unwrap();
        assert!(result.is_some());

        let task = result.unwrap();
        assert_eq!(task.stream, "stream-high");
        assert_eq!(task.data.get("task_id").unwrap(), &high_task_id);
    }

    #[tokio::test]
    async fn test_ack_task_success() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis.clone(),
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.ack_task("stream1", "entry-123").await;
        assert!(result.is_ok());

        let acked = redis.get_acked();
        assert_eq!(acked.len(), 1);
        assert_eq!(acked[0], "stream1:entry-123");
    }

    #[tokio::test]
    async fn test_ack_task_failure() {
        let redis = Arc::new(MockRedis::new());
        redis.set_should_fail(true);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.ack_task("stream1", "entry-123").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_claim_pending_tasks_no_pending() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.claim_pending_tasks().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_claim_pending_tasks_with_stale() {
        let redis = Arc::new(MockRedis::new());

        let task_id = Uuid::new_v4().to_string();
        redis.add_task("stream1", &task_id);

        let pending_info = PendingInfo {
            id: format!("{}-0", chrono::Utc::now().timestamp_millis()),
            consumer: "other-consumer".to_string(),
            idle_time_ms: 400_000,
            deliveries: 1,
        };
        redis.add_pending("stream1", pending_info);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        )
        .with_claim_idle_ms(300_000);

        let result = consumer.claim_pending_tasks().await.unwrap();
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn test_claim_pending_tasks_not_stale() {
        let redis = Arc::new(MockRedis::new());

        let task_id = Uuid::new_v4().to_string();
        redis.add_task("stream1", &task_id);

        let pending_info = PendingInfo {
            id: format!("{}-0", chrono::Utc::now().timestamp_millis()),
            consumer: "other-consumer".to_string(),
            idle_time_ms: 100_000,
            deliveries: 1,
        };
        redis.add_pending("stream1", pending_info);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        )
        .with_claim_idle_ms(300_000);

        let result = consumer.claim_pending_tasks().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let redis = Arc::new(MockRedis::new());
        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.health_check().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let redis = Arc::new(MockRedis::new());
        redis.set_should_fail(true);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.health_check().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_poll_task_redis_error() {
        let redis = Arc::new(MockRedis::new());
        redis.set_should_fail(true);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string()],
        );

        let result = consumer.poll_task().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_claim_pending_tasks_partial_failure() {
        let redis = Arc::new(MockRedis::new());

        let task_id = Uuid::new_v4().to_string();
        redis.add_task("stream1", &task_id);

        let pending_info = PendingInfo {
            id: format!("{}-0", chrono::Utc::now().timestamp_millis()),
            consumer: "other-consumer".to_string(),
            idle_time_ms: 400_000,
            deliveries: 1,
        };
        redis.add_pending("stream1", pending_info);

        let consumer = TaskConsumer::new(
            redis,
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["stream1".to_string(), "stream2".to_string()],
        )
        .with_claim_idle_ms(300_000);

        let result = consumer.claim_pending_tasks().await.unwrap();
        assert_eq!(result.len(), 1);
    }
}
