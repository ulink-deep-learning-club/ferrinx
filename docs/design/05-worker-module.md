# ferrinx-worker 模块设计

## 1. 模块职责

`ferrinx-worker` 是独立部署的推理 Worker 进程，职责包括：
- 从 Redis Streams 消费任务
- 执行异步推理
- 存储推理结果
- **上报模型状态到 Redis**（新增）
- **支持模型感知的任务消费**（新增）
- 支持重试和死信队列
- 优雅停机

**关键特性**：
- 消费组模式（XREADGROUP）
- 多 Worker 并行消费
- 任务自动重新分配（Worker 宕机时）
- **模型路由：任务优先路由到已缓存模型的 Worker**（新增）
- 本地模型缓存（LRU）

## 2. 模型路由机制

### 2.1 设计目标

当多个 Worker 分布于不同机器，且模型文件分布不均匀时：
- 任务应优先被路由到已缓存该模型的 Worker
- 其次路由到有模型文件但未缓存的 Worker
- 如果没有 Worker 拥有该模型，返回错误

### 2.2 模型状态定义

```rust
enum ModelState {
    Cached,    // 模型已加载到内存（最快）
    Available, // 模型文件存在但未加载（需加载）
    NotFound,  // 模型不存在
}
```

### 2.3 Redis 数据结构

**Worker 模型状态存储**：
```
Key: ferrinx:workers:{worker_id}:models
Type: Hash
Value: {
  "model_uuid_1": "cached",
  "model_uuid_2": "available",
  "model_uuid_3": "available"
}
TTL: 30s（Worker 心跳刷新）
```

**Worker 心跳**：
```
Key: ferrinx:workers:{worker_id}:heartbeat
Type: String
Value: timestamp
TTL: 60s
```

**模型到 Worker 映射（反向索引）**：
```
Key: ferrinx:models:{model_id}:workers
Type: Sorted Set
Score: 优先级分数（cached=0, available=1）
Member: worker_id
TTL: 随 Worker 心跳刷新
```

### 2.4 路由流程

```
API 接收异步推理请求
    │
    ▼
查询 Redis: ferrinx:models:{model_id}:workers
    │
    ├─ 找到 cached Worker (score=0) → 推送到该 Worker 的专属 Stream
    │
    ├─ 找到 available Worker (score=1) → 推送到该 Worker 的专属 Stream
    │
    └─ 无 Worker → 返回错误 NO_WORKER_AVAILABLE
```

### 2.5 Worker 模型状态上报

Worker 启动时和运行期间定期上报：

```rust
// 启动时扫描本地模型
async fn scan_local_models(&self) -> Result<HashSet<Uuid>> {
    let models = self.db.models.list_valid().await?;
    let mut available = HashSet::new();
    
    for model in models {
        if self.storage.exists(&model.file_path).await? {
            available.insert(model.id);
        }
    }
    
    Ok(available)
}

// 定期上报（每 10 秒）
async fn report_model_status(&self, cached_models: &HashSet<Uuid>) -> Result<()> {
    let available_models = self.scan_local_models().await?;
    
    let mut status = HashMap::new();
    for model_id in &available_models {
        if cached_models.contains(model_id) {
            status.insert(model_id.to_string(), "cached");
        } else {
            status.insert(model_id.to_string(), "available");
        }
    }
    
    // 写入 Redis
    self.redis.set_worker_models(&self.worker_id, &status).await?;
    self.redis.set_worker_heartbeat(&self.worker_id).await?;
    
    Ok(())
}
```

### 2.6 任务 Stream 分配

每个 Worker 有专属的 Stream，按模型路由：

```
通用 Stream（降级用）:
- ferrinx:tasks:high
- ferrinx:tasks:normal
- ferrinx:tasks:low

Worker 专属 Stream（模型路由）:
- ferrinx:worker:{worker_id}:tasks
```

任务推送逻辑：
```rust
async fn push_task_with_routing(&self, task: &InferenceTask) -> Result<()> {
    // 查找最优 Worker
    let workers = self.redis.get_model_workers(&task.model_id).await?;
    
    if let Some(best_worker) = workers.first() {
        // 推送到 Worker 专属 Stream
        let stream = format!("ferrinx:worker:{}:tasks", best_worker);
        self.redis.xadd(&stream, &task.to_map()).await?;
    } else {
        // 无 Worker 有模型
        return Err(RedisError::NoWorkerAvailable);
    }
    
    Ok(())
}
```

## 2. 核心结构设计

### 2.1 Worker 主流程

```rust
// src/main.rs

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_file("config.toml")?;
    init_logging(&config.logging)?;
    
    // 初始化依赖
    let db = DbContext::new(&config.database).await?;
    let redis = RedisClient::new(&config.redis).await?;
    let engine = InferenceEngine::new(&config.onnx)?;
    let storage = create_storage(&config.storage)?;
    
    // 创建消费者
    let consumer_name = config.worker.consumer_name.clone()
        .unwrap_or_else(|| generate_consumer_name());
    
    let consumer = TaskConsumer::new(
        redis.clone(),
        consumer_name,
        config.worker.concurrency,
    );
    
    // 创建处理器
    let processor = TaskProcessor::new(
        db,
        redis,
        engine,
        storage,
        config.worker.max_retries,
    );
    
    // 取消令牌
    let cancel_token = CancellationToken::new();
    
    // 运行 Worker
    run_worker(consumer, processor, cancel_token, config.worker.poll_interval_ms).await
}

async fn run_worker(
    mut consumer: TaskConsumer,
    processor: TaskProcessor,
    cancel_token: CancellationToken,
    poll_interval_ms: u64,
) -> Result<(), Error> {
    let mut current_tasks = Arc::new(AtomicUsize::new(0));
    
    info!("Worker started");
    
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                info!("Shutdown signal received");
                break;
            }
            
            result = consumer.poll_task() => {
                match result {
                    Ok(Some(task_message)) => {
                        let processor = processor.clone();
                        let current_tasks = current_tasks.clone();
                        let timeout_secs = 300; // 5 minutes max
                        
                        current_tasks.fetch_add(1, Ordering::Relaxed);
                        
                        tokio::spawn(async move {
                            let result = tokio::time::timeout(
                                Duration::from_secs(timeout_secs),
                                processor.process(task_message)
                            ).await;
                            
                            match result {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => error!("Task processing failed: {}", e),
                                Err(_) => error!("Task processing timed out after {}s", timeout_secs),
                            }
                            current_tasks.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Ok(None) => {
                        // 没有任务，短暂休眠
                        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                    }
                    Err(e) => {
                        error!("Failed to poll task: {}", e);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
    
    // 等待所有任务完成
    let timeout = Duration::from_secs(30);
    let start = Instant::now();
    while current_tasks.load(Ordering::Relaxed) > 0 && start.elapsed() < timeout {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    if current_tasks.load(Ordering::Relaxed) > 0 {
        warn!("Some tasks still running after timeout");
    } else {
        info!("All tasks completed");
    }
    
    Ok(())
}
```

### 2.2 任务消费者

```rust
// src/consumer.rs

use redis::aio::Connection;

pub struct TaskConsumer {
    redis: RedisClient,
    consumer_name: String,
    group_name: String,
    streams: Vec<String>,
    concurrency: usize,
}

impl TaskConsumer {
    pub fn new(
        redis: RedisClient,
        consumer_name: String,
        concurrency: usize,
    ) -> Self {
        Self {
            redis,
            consumer_name,
            group_name: "ferrinx-workers".to_string(),
            streams: vec![
                "ferrinx:tasks:high".to_string(),
                "ferrinx:tasks:normal".to_string(),
                "ferrinx:tasks:low".to_string(),
            ],
            concurrency,
        }
    }
    
    /// 轮询任务
    pub async fn poll_task(&mut self) -> Result<Option<TaskMessage>, WorkerError> {
        // 按优先级顺序消费
        for stream in &self.streams {
            if let Some(task) = self.read_from_stream(stream).await? {
                return Ok(Some(task));
            }
        }
        
        Ok(None)
    }
    
    /// 从指定 Stream 读取任务
    async fn read_from_stream(&mut self, stream: &str) -> Result<Option<TaskMessage>, WorkerError> {
        let mut conn = self.redis.get_connection().await?;
        
        // XREADGROUP GROUP {group} {consumer} COUNT 1 BLOCK {timeout} STREAMS {stream} >
        let result: Option<HashMap<String, Vec<StreamEntry>>> = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.group_name)
            .arg(&self.consumer_name)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(0) // 非阻塞
            .arg("STREAMS")
            .arg(stream)
            .arg(">") // 只读取新消息
            .query_async(&mut conn)
            .await?;
        
        if let Some(result) = result {
            if let Some(entries) = result.get(stream) {
                if let Some(entry) = entries.first() {
                    return Ok(Some(TaskMessage {
                        stream: stream.to_string(),
                        entry_id: entry.id.clone(),
                        data: entry.data.clone(),
                    }));
                }
            }
        }
        
        Ok(None)
    }
    
    /// 确认任务完成
    pub async fn ack_task(&mut self, stream: &str, entry_id: &str) -> Result<(), WorkerError> {
        let mut conn = self.redis.get_connection().await?;
        
        redis::cmd("XACK")
            .arg(stream)
            .arg(&self.group_name)
            .arg(entry_id)
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
    
    /// 认领超时任务
    pub async fn claim_pending_tasks(&mut self) -> Result<Vec<TaskMessage>, WorkerError> {
        let mut tasks = Vec::new();
        
        for stream in &self.streams {
            let pending = self.claim_pending_from_stream(stream).await?;
            tasks.extend(pending);
        }
        
        Ok(tasks)
    }
    
    async fn claim_pending_from_stream(&mut self, stream: &str) -> Result<Vec<TaskMessage>, WorkerError> {
        let mut conn = self.redis.get_connection().await?;
        
        // XPENDING {stream} {group} - + {count}
        let pending: Vec<(String, String, i64, i64)> = redis::cmd("XPENDING")
            .arg(stream)
            .arg(&self.group_name)
            .arg("-")
            .arg("+")
            .arg(10) // 每次认领 10 个
            .query_async(&mut conn)
            .await?;
        
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        
        // XCLAIM {stream} {group} {consumer} {min_idle_time} {entry_id}...
        let entry_ids: Vec<&str> = pending.iter().map(|(id, _, _, _)| id.as_str()).collect();
        
        let claimed: Vec<StreamEntry> = redis::cmd("XCLAIM")
            .arg(stream)
            .arg(&self.group_name)
            .arg(&self.consumer_name)
            .arg(300000) // 5 分钟未确认的任务
            .args(&entry_ids)
            .query_async(&mut conn)
            .await?;
        
        Ok(claimed.into_iter().map(|entry| TaskMessage {
            stream: stream.to_string(),
            entry_id: entry.id,
            data: entry.data,
        }).collect())
    }
}
```

### 2.3 任务处理器

```rust
// src/processor.rs

pub struct TaskProcessor {
    db: DbContext,
    redis: RedisClient,
    engine: InferenceEngine,
    storage: Arc<dyn ModelStorage>,
    max_retries: u32,
}

impl TaskProcessor {
    pub fn new(
        db: DbContext,
        redis: RedisClient,
        engine: InferenceEngine,
        storage: Arc<dyn ModelStorage>,
        max_retries: u32,
    ) -> Self {
        Self {
            db,
            redis,
            engine,
            storage,
            max_retries,
        }
    }
    
    /// 处理任务（带超时保护）
    pub async fn process(&self, task_message: TaskMessage) -> Result<(), WorkerError> {
        let task_id = task_message.task_id()?;
        
        info!("Processing task: {}", task_id);
        
        // 从数据库加载任务
        let task = self.db.tasks
            .find_by_id(&task_id)
            .await?
            .ok_or(WorkerError::TaskNotFound(task_id))?;
        
        // 更新状态为 running
        self.db.tasks
            .update_status(&task_id, TaskStatus::Running)
            .await?;
        
        // 执行推理（内部已有超时保护）
        let result = self.execute_inference(&task).await;
        
        // 处理结果
        match result {
            Ok(outputs) => {
                // 成功
                self.handle_success(&task_id, outputs).await?;
                self.redis.ack_task(&task_message.stream, &task_message.entry_id).await?;
                info!("Task completed: {}", task_id);
            }
            Err(e) => {
                // 失败
                self.handle_failure(&task, &task_message, e).await?;
            }
        }
        
        Ok(())
    }
    
    /// 执行推理
    async fn execute_inference(&self, task: &InferenceTask) -> Result<HashMap<String, serde_json::Value>, WorkerError> {
        // 获取模型信息
        let model = self.db.models
            .find_by_id(&task.model_id)
            .await?
            .ok_or(WorkerError::ModelNotFound(task.model_id.to_string()))?;
        
        if !model.is_valid {
            return Err(WorkerError::ModelNotValid(model.id.to_string()));
        }
        
        // 解析输入
        let inputs: HashMap<String, serde_json::Value> = serde_json::from_value(task.inputs.clone())?;
        
        // 执行推理
        let input = InferenceInput { inputs };
        let output = self.engine
            .infer(&model.id.to_string(), &model.file_path, input)
            .await?;
        
        Ok(output.outputs)
    }
    
    /// 处理成功
    async fn handle_success(
        &self,
        task_id: &uuid::Uuid,
        outputs: HashMap<String, serde_json::Value>,
    ) -> Result<(), WorkerError> {
        let outputs_json = serde_json::to_value(&outputs)?;
        
        self.db.tasks
            .set_result(task_id, TaskStatus::Completed, Some(&outputs_json), None)
            .await?;
        
        // 缓存结果到 Redis
        self.cache_result(task_id, &outputs_json).await?;
        
        Ok(())
    }
    
    /// 处理失败
    async fn handle_failure(
        &self,
        task: &InferenceTask,
        task_message: &TaskMessage,
        error: WorkerError,
    ) -> Result<(), WorkerError> {
        error!("Task {} failed: {}", task.id, error);
        
        let retry_count = task.retry_count + 1;
        
        if retry_count < self.max_retries as i32 {
            // 重试
            self.db.tasks
                .update_retry_count(&task.id, retry_count)
                .await?;
            
            // 延迟后重新放回队列
            let delay = Duration::from_millis(1000 * 2u64.pow(retry_count as u32));
            tokio::time::sleep(delay).await;
            
            // 不 ACK，让任务重新可见
            warn!("Task {} will retry (attempt {}/{})", task.id, retry_count, self.max_retries);
        } else {
            // 移入死信队列
            self.move_to_dead_letter(task_message, &error.to_string()).await?;
            
            self.db.tasks
                .set_result(&task.id, TaskStatus::Failed, None, Some(&error.to_string()))
                .await?;
            
            // ACK 任务
            self.redis.ack_task(&task_message.stream, &task_message.entry_id).await?;
            
            error!("Task {} moved to dead letter queue", task.id);
        }
        
        Ok(())
    }
    
    /// 缓存结果
    async fn cache_result(
        &self,
        task_id: &uuid::Uuid,
        outputs: &serde_json::Value,
    ) -> Result<(), WorkerError> {
        let key = format!("ferrinx:results:{}", task_id);
        
        self.redis
            .set_json(&key, outputs, Duration::from_secs(86400))
            .await?;
        
        Ok(())
    }
    
    /// 移入死信队列
    async fn move_to_dead_letter(
        &self,
        task_message: &TaskMessage,
        error: &str,
    ) -> Result<(), WorkerError> {
        let mut conn = self.redis.get_connection().await?;
        
        let data = task_message.data.clone();
        let mut dead_letter_data = data.clone();
        dead_letter_data.insert("error".to_string(), error.to_string());
        dead_letter_data.insert("retries".to_string(), self.max_retries.to_string());
        
        redis::cmd("XADD")
            .arg("ferrinx:tasks:dead_letter")
            .arg("*")
            .args(&dead_letter_data.iter().flat_map(|(k, v)| vec![k.as_str(), v.as_str()]))
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TaskMessage {
    pub stream: String,
    pub entry_id: String,
    pub data: HashMap<String, String>,
}

impl TaskMessage {
    pub fn task_id(&self) -> Result<uuid::Uuid, WorkerError> {
        self.data
            .get("task_id")
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or(WorkerError::InvalidTaskMessage)
    }
}
```

### 2.4 定期维护任务

```rust
// src/maintenance.rs

impl TaskProcessor {
    /// 清理过期任务
    pub async fn cleanup_expired_tasks(&self, retention_days: u32) -> Result<u64, WorkerError> {
        let deleted = self.db.tasks
            .cleanup_expired(retention_days, 1000)
            .await?;
        
        if deleted > 0 {
            info!("Cleaned up {} expired tasks", deleted);
        }
        
        Ok(deleted)
    }
    
    /// 清理过期的临时 API Key
    pub async fn cleanup_expired_temp_keys(&self) -> Result<u64, WorkerError> {
        let deleted = self.db.api_keys
            .cleanup_expired_temp_keys()
            .await?;
        
        if deleted > 0 {
            info!("Cleaned up {} expired temporary API keys", deleted);
        }
        
        Ok(deleted)
    }
}
```

## 3. Worker 部署与扩展

### 3.1 单进程模式

```bash
# 启动单个 Worker
ferrinx-worker --config config.toml
```

### 3.2 多进程模式

```bash
# 启动多个 Worker 进程
ferrinx-worker --config config.toml --concurrency 8
ferrinx-worker --config config.toml --concurrency 8
ferrinx-worker --config config.toml --concurrency 8
```

### 3.3 Kubernetes 部署

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ferrinx-worker
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: worker
        image: ferrinx-worker:latest
        command: ["ferrinx-worker"]
        args: ["--config", "/config/config.toml"]
        resources:
          limits:
            cpu: "4"
            memory: "8Gi"
          requests:
            cpu: "2"
            memory: "4Gi"
```

## 4. 设计要点

### 4.1 任务分配无状态

- 任何 Worker 可以处理任何任务
- 基于 Redis Streams 消费组
- Worker 宕机时任务自动重新分配

### 4.2 模型缓存有状态

- Worker 内部有 LRU 缓存
- 相同模型任务发到已缓存 Worker 效率更高
- 可通过 Worker 分组优化

### 4.3 错误处理

- 重试机制（指数退避）
- 死信队列
- 错误日志记录

### 4.4 性能优化

- 并发处理多个任务
- 模型缓存
- 结果缓存
