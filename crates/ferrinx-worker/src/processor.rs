use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};
use uuid::Uuid;

use ferrinx_common::{InferenceInput, InferenceTask, TaskStatus};
use ferrinx_core::InferenceEngine;
use ferrinx_db::DbContext;

use crate::consumer::TaskMessage;
use crate::error::{Result, WorkerError};
use crate::redis::RedisClient;

pub struct TaskProcessor {
    db: Arc<DbContext>,
    redis: Arc<dyn RedisClient>,
    engine: Arc<InferenceEngine>,
    max_retries: u32,
    retry_base_delay_ms: u64,
}

impl TaskProcessor {
    pub fn new(
        db: Arc<DbContext>,
        redis: Arc<dyn RedisClient>,
        engine: Arc<InferenceEngine>,
        max_retries: u32,
        retry_base_delay_ms: u64,
    ) -> Self {
        Self {
            db,
            redis,
            engine,
            max_retries,
            retry_base_delay_ms,
        }
    }

    pub async fn process(&self, task_message: TaskMessage) -> Result<()> {
        let task_id = task_message.task_id()?;

        info!("Processing task: {}", task_id);

        let task = self
            .db
            .tasks
            .find_by_id(&task_id)
            .await?
            .ok_or(WorkerError::TaskNotFound(task_id))?;

        if task.status.is_terminal() {
            info!(
                "Task {} already in terminal state: {:?}",
                task_id, task.status
            );
            return Ok(());
        }

        self.db
            .tasks
            .update_status(&task_id, TaskStatus::Running)
            .await?;

        let result = self.execute_inference(&task).await;

        match result {
            Ok(outputs) => {
                self.handle_success(&task_id, &outputs).await?;
                info!("Task {} completed successfully", task_id);
            }
            Err(e) => {
                error!("Task {} execution failed: {}", task_id, e);
                self.handle_failure(&task, &task_message, e).await?;
            }
        }

        Ok(())
    }

    async fn execute_inference(
        &self,
        task: &InferenceTask,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        let model = self
            .db
            .models
            .find_by_id(&task.model_id)
            .await?
            .ok_or_else(|| WorkerError::ModelNotFound(task.model_id.to_string()))?;

        if !model.is_valid() {
            return Err(WorkerError::ModelNotValid(model.id.to_string()));
        }

        let inputs: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_value(task.inputs.clone())?;

        let input = InferenceInput { inputs };

        let output = self
            .engine
            .infer(&model.id, &model.file_path, input)
            .await?;

        Ok(output.outputs)
    }

    async fn handle_success(
        &self,
        task_id: &Uuid,
        outputs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let outputs_json = serde_json::to_value(outputs)?;

        self.db
            .tasks
            .set_result(task_id, TaskStatus::Completed, Some(&outputs_json), None)
            .await?;

        self.cache_result(task_id, &outputs_json).await?;

        Ok(())
    }

    async fn handle_failure(
        &self,
        task: &InferenceTask,
        task_message: &TaskMessage,
        error: WorkerError,
    ) -> Result<()> {
        let retry_count = task.retry_count + 1;
        let task_id = task.id;

        if retry_count < self.max_retries as i32 {
            let delay_ms = self.retry_base_delay_ms * 2u64.pow(retry_count as u32);
            warn!(
                "Task {} will retry (attempt {}/{}), delay {}ms",
                task_id, retry_count, self.max_retries, delay_ms
            );

            self.db
                .tasks
                .update_status(&task_id, TaskStatus::Pending)
                .await?;

            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        } else {
            error!(
                "Task {} exceeded max retries, moving to failed state",
                task_id
            );

            self.move_to_dead_letter(task_message, &error.to_string())
                .await?;

            self.db
                .tasks
                .set_result(&task_id, TaskStatus::Failed, None, Some(&error.to_string()))
                .await?;
        }

        Ok(())
    }

    async fn cache_result(&self, task_id: &Uuid, outputs: &serde_json::Value) -> Result<()> {
        let key = format!("ferrinx:results:{}", task_id);

        self.redis
            .set_json(&key, outputs, Duration::from_secs(86400))
            .await?;

        Ok(())
    }

    async fn move_to_dead_letter(&self, task_message: &TaskMessage, error: &str) -> Result<()> {
        let mut data = task_message.data.clone();
        data.insert("error".to_string(), error.to_string());
        data.insert("retries".to_string(), self.max_retries.to_string());
        data.insert("failed_at".to_string(), chrono::Utc::now().to_rfc3339());

        self.redis.xadd("ferrinx:tasks:dead_letter", &data).await?;

        warn!(
            "Task {} moved to dead letter queue: {}",
            task_message.task_id()?,
            error
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ferrinx_core::InferenceEngine;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct MockRedis {
        xadd_calls: std::sync::Mutex<Vec<(String, HashMap<String, String>)>>,
    }

    impl MockRedis {
        fn new() -> Self {
            Self {
                xadd_calls: std::sync::Mutex::new(Vec::new()),
            }
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

        async fn xack(
            &self,
            _stream: &str,
            _group: &str,
            _entry_id: &str,
        ) -> crate::error::Result<()> {
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
            stream: &str,
            data: &HashMap<String, String>,
        ) -> crate::error::Result<String> {
            let mut calls = self.xadd_calls.lock().unwrap();
            calls.push((stream.to_string(), data.clone()));
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

        async fn set_worker_heartbeat(&self, _worker_id: &str) -> crate::error::Result<()> {
            Ok(())
        }

        async fn set_worker_models(
            &self,
            _worker_id: &str,
            _models: &HashMap<String, String>,
        ) -> crate::error::Result<()> {
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

        async fn remove_worker_models(&self, _worker_id: &str) -> crate::error::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_task_message_task_id() {
        let task_id = Uuid::new_v4();
        let mut data = HashMap::new();
        data.insert("task_id".to_string(), task_id.to_string());

        let task_message = TaskMessage {
            stream: "test-stream".to_string(),
            entry_id: "test-entry".to_string(),
            data,
        };

        assert_eq!(task_message.task_id().unwrap(), task_id);
    }

    #[tokio::test]
    async fn test_task_message_missing_task_id() {
        let task_message = TaskMessage {
            stream: "test-stream".to_string(),
            entry_id: "test-entry".to_string(),
            data: HashMap::new(),
        };

        assert!(task_message.task_id().is_err());
    }

    #[test]
    fn test_processor_new() {
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
            let onnx_config = ferrinx_common::OnnxConfig {
                cache_size: 3,
                preload: vec![],
                execution_provider: ferrinx_common::ExecutionProvider::CPU,
                gpu_device_id: 0,
                dynamic_lib_path: None,
            };
            let engine = Arc::new(InferenceEngine::new(&onnx_config).unwrap());

            let processor = TaskProcessor::new(db, redis, engine, 3, 1000);
            assert_eq!(processor.max_retries, 3);
            assert_eq!(processor.retry_base_delay_ms, 1000);
        });
    }
}
