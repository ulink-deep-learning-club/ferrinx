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
            .xclaim(stream, &self.group_name, &self.consumer_name, self.claim_idle_ms, &entry_ids)
            .await?;

        warn!(
            "Claimed {} stale tasks from {}",
            claimed.len(),
            stream
        );

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
