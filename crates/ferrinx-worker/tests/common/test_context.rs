use std::sync::Arc;
use std::collections::HashMap;
use async_trait::async_trait;
use ferrinx_common::TaskStatus;
use ferrinx_core::InferenceEngine;
use ferrinx_db::DbContext;
use ferrinx_worker::{TaskConsumer, TaskProcessor, RedisClient};
use uuid::Uuid;
use std::time::Duration;

pub struct TestContext {
    pub db: Arc<DbContext>,
    pub redis: Arc<MockRedis>,
    pub engine: Arc<InferenceEngine>,
}

impl TestContext {
    pub async fn new() -> Self {
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

        Self { db, redis, engine }
    }

    pub async fn create_test_user(&self) -> Uuid {
        let user = ferrinx_common::User {
            id: Uuid::new_v4(),
            username: format!("test-user-{}", Uuid::new_v4()),
            password_hash: "test-hash".to_string(),
            role: ferrinx_common::UserRole::User,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db.users.save(&user).await.unwrap();
        user.id
    }

    pub async fn create_test_api_key(&self, user_id: Uuid) -> Uuid {
        let api_key = ferrinx_common::ApiKeyRecord {
            id: Uuid::new_v4(),
            user_id,
            name: "test-key".to_string(),
            key_hash: format!("test-key-hash-{}", Uuid::new_v4()),
            permissions: ferrinx_common::Permissions::default(),
            is_active: true,
            is_temporary: false,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_used_at: None,
        };

        self.db.api_keys.save(&api_key).await.unwrap();
        api_key.id
    }

    pub async fn create_test_model(&self, _user_id: Uuid) -> Uuid {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let model_path = format!("{}/../../tests/fixtures/models/hanzi_tiny.onnx", manifest_dir);
        
        let model = ferrinx_common::ModelInfo {
            id: Uuid::new_v4(),
            name: "hanzi-tiny".to_string(),
            version: "1.0.0".to_string(),
            file_path: model_path,
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!({"input": [1, 3, 224, 224]})),
            output_shapes: Some(serde_json::json!({"output": [1, 1000]})),
            metadata: Some(serde_json::json!({"labels_file": "hanzi-tiny-labels.json"})),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db.models.save(&model).await.unwrap();
        model.id
    }

    pub async fn create_test_task(
        &self,
        model_id: Uuid,
        user_id: Uuid,
        api_key_id: Uuid,
    ) -> Uuid {
        let input_tensor = ferrinx_common::Tensor::new_f32(
            vec![1, 1, 64, 64],
            &vec![0.0f32; 64 * 64],
        );
        
        self.create_test_task_with_inputs(model_id, user_id, api_key_id, input_tensor).await
    }

    pub async fn create_test_task_with_inputs(
        &self,
        model_id: Uuid,
        user_id: Uuid,
        api_key_id: Uuid,
        input_tensor: ferrinx_common::Tensor,
    ) -> Uuid {
        let task = ferrinx_common::InferenceTask {
            id: Uuid::new_v4(),
            model_id,
            user_id,
            api_key_id,
            inputs: serde_json::json!({"input": input_tensor}),
            priority: 1,
            status: TaskStatus::Pending,
            retry_count: 0,
            outputs: None,
            error_message: None,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
        };

        self.db.tasks.save(&task).await.unwrap();

        let task_data = HashMap::from([
            ("task_id".to_string(), task.id.to_string()),
            ("model_id".to_string(), model_id.to_string()),
            ("user_id".to_string(), user_id.to_string()),
            ("api_key_id".to_string(), api_key_id.to_string()),
            ("priority".to_string(), "1".to_string()),
            ("created_at".to_string(), task.created_at.to_rfc3339()),
        ]);

        self.redis.add_task("ferrinx:tasks:normal", task_data).await;

        task.id
    }

    pub async fn create_test_task_with_invalid_inputs(
        &self,
        model_id: Uuid,
        user_id: Uuid,
        api_key_id: Uuid,
        inputs: serde_json::Value,
    ) -> Uuid {
        let task = ferrinx_common::InferenceTask {
            id: Uuid::new_v4(),
            model_id,
            user_id,
            api_key_id,
            inputs,
            priority: 1,
            status: TaskStatus::Pending,
            retry_count: 0,
            outputs: None,
            error_message: None,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
        };

        self.db.tasks.save(&task).await.unwrap();

        let task_data = HashMap::from([
            ("task_id".to_string(), task.id.to_string()),
            ("model_id".to_string(), model_id.to_string()),
            ("user_id".to_string(), user_id.to_string()),
            ("api_key_id".to_string(), api_key_id.to_string()),
            ("priority".to_string(), "1".to_string()),
            ("created_at".to_string(), task.created_at.to_rfc3339()),
        ]);

        self.redis.add_task("ferrinx:tasks:normal", task_data).await;

        task.id
    }

    pub fn create_consumer(&self) -> Arc<TaskConsumer> {
        Arc::new(TaskConsumer::new(
            self.redis.clone(),
            "test-consumer".to_string(),
            "test-group".to_string(),
            vec!["ferrinx:tasks:normal".to_string()],
        ))
    }

    pub fn create_processor(&self) -> Arc<TaskProcessor> {
        Arc::new(TaskProcessor::new(
            self.db.clone(),
            self.redis.clone(),
            self.engine.clone(),
            3,
            100,
        ))
    }

    pub fn create_processor_no_retry(&self) -> Arc<TaskProcessor> {
        Arc::new(TaskProcessor::new(
            self.db.clone(),
            self.redis.clone(),
            self.engine.clone(),
            0,
            100,
        ))
    }
}

#[derive(Clone)]
pub struct MockRedis {
    streams: std::sync::Arc<tokio::sync::RwLock<HashMap<String, Vec<StreamEntry>>>>,
    cache: std::sync::Arc<tokio::sync::RwLock<HashMap<String, (serde_json::Value, u64)>>>,
    acked: std::sync::Arc<tokio::sync::RwLock<Vec<String>>>,
}

#[derive(Clone, Debug)]
pub struct StreamEntry {
    pub id: String,
    pub data: HashMap<String, String>,
}

impl MockRedis {
    pub fn new() -> Self {
        Self {
            streams: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            cache: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            acked: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    pub async fn add_task(&self, stream: &str, data: HashMap<String, String>) {
        let mut streams = self.streams.write().await;
        let id = format!("{}-0", chrono::Utc::now().timestamp_millis());
        let entry = StreamEntry { id, data };
        streams.entry(stream.to_string()).or_default().push(entry);
    }

    #[allow(dead_code)]
    pub async fn get_stream_count(&self, stream: &str) -> usize {
        let streams = self.streams.read().await;
        streams.get(stream).map(|s| s.len()).unwrap_or(0)
    }

    pub async fn get_acked(&self) -> Vec<String> {
        self.acked.read().await.clone()
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
    ) -> ferrinx_worker::Result<Option<Vec<ferrinx_worker::StreamEntry>>> {
        let mut streams = self.streams.write().await;
        if let Some(entries) = streams.get_mut(stream) {
            let result: Vec<_> = entries
                .drain(..count.min(entries.len()))
                .map(|e| ferrinx_worker::StreamEntry {
                    id: e.id,
                    data: e.data,
                })
                .collect();

            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        } else {
            Ok(None)
        }
    }

    async fn xack(&self, stream: &str, _group: &str, entry_id: &str) -> ferrinx_worker::Result<()> {
        self.acked.write().await.push(format!("{}:{}", stream, entry_id));
        Ok(())
    }

    async fn xpending(
        &self,
        _stream: &str,
        _group: &str,
        _count: usize,
    ) -> ferrinx_worker::Result<Vec<ferrinx_worker::PendingInfo>> {
        Ok(Vec::new())
    }

    async fn xclaim(
        &self,
        _stream: &str,
        _group: &str,
        _consumer: &str,
        _min_idle_ms: i64,
        _entry_ids: &[&str],
    ) -> ferrinx_worker::Result<Vec<ferrinx_worker::StreamEntry>> {
        Ok(Vec::new())
    }

    async fn xadd(
        &self,
        stream: &str,
        data: &HashMap<String, String>,
    ) -> ferrinx_worker::Result<String> {
        let mut streams = self.streams.write().await;
        let id = format!("{}-0", chrono::Utc::now().timestamp_millis());
        let entry = StreamEntry {
            id: id.clone(),
            data: data.clone(),
        };
        streams.entry(stream.to_string()).or_default().push(entry);
        Ok(id)
    }

    async fn set_json(
        &self,
        key: &str,
        value: &serde_json::Value,
        ttl: Duration,
    ) -> ferrinx_worker::Result<()> {
        let mut cache = self.cache.write().await;
        cache.insert(key.to_string(), (value.clone(), ttl.as_secs()));
        Ok(())
    }

    async fn get_json(&self, key: &str) -> ferrinx_worker::Result<Option<serde_json::Value>> {
        let cache = self.cache.read().await;
        Ok(cache.get(key).map(|(v, _)| v.clone()))
    }

    async fn del(&self, key: &str) -> ferrinx_worker::Result<()> {
        let mut cache = self.cache.write().await;
        cache.remove(key);
        Ok(())
    }

    async fn health_check(&self) -> ferrinx_worker::Result<()> {
        Ok(())
    }

    async fn set_worker_heartbeat(&self, _worker_id: &str) -> ferrinx_worker::Result<()> {
        Ok(())
    }

    async fn set_worker_models(
        &self,
        _worker_id: &str,
        _models: &HashMap<String, String>,
    ) -> ferrinx_worker::Result<()> {
        Ok(())
    }

    async fn get_worker_models(
        &self,
        _worker_id: &str,
    ) -> ferrinx_worker::Result<HashMap<String, String>> {
        Ok(HashMap::new())
    }

    async fn get_model_workers(&self, _model_id: &Uuid) -> ferrinx_worker::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn remove_worker_models(&self, _worker_id: &str) -> ferrinx_worker::Result<()> {
        Ok(())
    }
}
