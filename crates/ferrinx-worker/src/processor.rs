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
            info!("Task {} already in terminal state: {:?}", task_id, task.status);
            return Ok(());
        }

        self.db.tasks.update_status(&task_id, TaskStatus::Running).await?;

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

        if !model.is_valid {
            return Err(WorkerError::ModelNotValid(model.id.to_string()));
        }

        let inputs: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_value(task.inputs.clone())?;

        let input = InferenceInput { inputs };

        let output = self
            .engine
            .infer(&model.id.to_string(), &model.file_path, input)
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

            self.db.tasks.update_status(&task_id, TaskStatus::Pending).await?;

            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        } else {
            error!(
                "Task {} exceeded max retries, moving to failed state",
                task_id
            );

            self.move_to_dead_letter(task_message, &error.to_string()).await?;

            self.db
                .tasks
                .set_result(&task_id, TaskStatus::Failed, None, Some(&error.to_string()))
                .await?;
        }

        Ok(())
    }

    async fn cache_result(
        &self,
        task_id: &Uuid,
        outputs: &serde_json::Value,
    ) -> Result<()> {
        let key = format!("ferrinx:results:{}", task_id);

        self.redis
            .set_json(&key, outputs, Duration::from_secs(86400))
            .await?;

        Ok(())
    }

    async fn move_to_dead_letter(
        &self,
        task_message: &TaskMessage,
        error: &str,
    ) -> Result<()> {
        let mut data = task_message.data.clone();
        data.insert("error".to_string(), error.to_string());
        data.insert("retries".to_string(), self.max_retries.to_string());
        data.insert(
            "failed_at".to_string(),
            chrono::Utc::now().to_rfc3339(),
        );

        self.redis
            .xadd("ferrinx:tasks:dead_letter", &data)
            .await?;

        warn!(
            "Task {} moved to dead letter queue: {}",
            task_message.task_id()?,
            error
        );

        Ok(())
    }
}
