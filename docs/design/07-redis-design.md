# Redis 设计

## 1. Redis 在 Ferrinx 中的角色

Redis 在 Ferrinx 中承担三个核心角色：
1. **任务队列**（Redis Streams）- 异步推理任务
2. **结果缓存** - 推理结果临时存储
3. **API Key 缓存** - 加速验证

## 2. 数据结构设计

### 2.1 任务队列（Redis Streams）

#### Stream Keys

```
ferrinx:tasks:high       # 高优先级任务队列
ferrinx:tasks:normal     # 普通优先级任务队列
ferrinx:tasks:low        # 低优先级任务队列
ferrinx:tasks:dead_letter  # 死信队列
```

#### 消费组配置

```
Consumer Group: ferrinx-workers
Consumers: worker-{hostname}-{pid}
```

#### 任务消息格式

**小型输入（推荐直接放入 Stream）：**
```
XADD ferrinx:tasks:normal * task_id "task-uuid" model_id "model-uuid" \
     user_id "user-uuid" api_key_id "key-uuid" priority "5" \
     created_at "2024-01-01T10:00:00Z" \
     inputs '{"input.1":[[1.0,2.0,3.0]]}'
```

**大型输入（仅存 ID，从数据库读取）：**
```
XADD ferrinx:tasks:normal * task_id "task-uuid" model_id "model-uuid" \
     user_id "user-uuid" api_key_id "key-uuid" priority "5" \
     created_at "2024-01-01T10:00:00Z"
```

**字段说明**：
- `task_id`: 任务 UUID（对应数据库 `inference_tasks.id`）
- `model_id`: 模型 UUID
- `user_id`: 用户 UUID
- `api_key_id`: API Key UUID
- `priority`: 优先级（1-10）
- `created_at`: 创建时间（ISO 8601）
- `inputs`: （可选）小型输入数据，避免额外数据库查询

**设计建议**：
- 小型输入（< 1KB）：直接放入 Stream，减少数据库查询
- 大型输入：仅存 ID，Worker 从数据库读取
- 可通过配置 `stream_include_inputs = true/false` 控制

### 2.2 结果缓存

#### Key Pattern

```
ferrinx:results:{task_id}  # 推理结果
```

#### 数据结构

```json
{
  "outputs": {
    "output.1": [[0.5, 0.3, 0.2]]
  },
  "latency_ms": 45,
  "completed_at": "2024-01-01T10:00:05Z"
}
```

#### TTL

- 默认 24 小时（`result_cache_ttl = 86400`）

### 2.3 API Key 缓存

#### Key Pattern

```
ferrinx:api_keys:{key_hash}  # API Key 信息
```

#### 数据结构

```json
{
  "id": "api-key-uuid",
  "user_id": "user-uuid",
  "permissions": {
    "models": ["read"],
    "inference": ["execute"],
    "api_keys": ["read", "write"],
    "admin": false
  },
  "is_active": true,
  "is_temporary": false,
  "expires_at": "2024-12-31T23:59:59Z"
}
```

#### TTL

- 默认 1 小时（`api_key_cache_ttl = 3600`）

## 3. 核心操作

### 3.1 推送任务

```rust
impl RedisClient {
    /// 推送任务到队列
    pub async fn push_task(&self, task: &InferenceTask) -> Result<(), RedisError> {
        let stream_key = match task.priority {
            8..=10 => "ferrinx:tasks:high",
            4..=7 => "ferrinx:tasks:normal",
            _ => "ferrinx:tasks:low",
        };
        
        let mut conn = self.get_connection().await?;
        
        redis::cmd("XADD")
            .arg(stream_key)
            .arg("*")
            .arg("task_id")
            .arg(&task.id.to_string())
            .arg("model_id")
            .arg(&task.model_id.to_string())
            .arg("user_id")
            .arg(&task.user_id.to_string())
            .arg("api_key_id")
            .arg(&task.api_key_id.to_string())
            .arg("priority")
            .arg(&task.priority.to_string())
            .arg("created_at")
            .arg(&task.created_at.to_rfc3339())
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
}
```

### 3.2 消费任务

```rust
impl RedisClient {
    /// 按优先级消费任务
    pub async fn consume_task(&self, consumer: &str) -> Result<Option<TaskMessage>, RedisError> {
        let streams = [
            ("ferrinx:tasks:high", ">"),
            ("ferrinx:tasks:normal", ">"),
            ("ferrinx:tasks:low", ">"),
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
        let mut conn = self.get_connection().await?;
        
        let result: Option<HashMap<String, Vec<StreamEntry>>> = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg("ferrinx-workers")
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(0)
            .arg("STREAMS")
            .arg(stream)
            .arg(id)
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
}
```

### 3.3 确认任务

```rust
impl RedisClient {
    /// 确认任务完成
    pub async fn ack_task(&self, stream: &str, entry_id: &str) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        
        redis::cmd("XACK")
            .arg(stream)
            .arg("ferrinx-workers")
            .arg(entry_id)
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
}
```

### 3.4 认领超时任务

```rust
impl RedisClient {
    /// 认领超时未确认的任务
    pub async fn claim_pending_tasks(&self, consumer: &str) -> Result<Vec<TaskMessage>, RedisError> {
        let mut tasks = Vec::new();
        
        for stream in &["ferrinx:tasks:high", "ferrinx:tasks:normal", "ferrinx:tasks:low"] {
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
        let mut conn = self.get_connection().await?;
        
        // 获取 pending 列表
        let pending: Vec<(String, String, i64, i64)> = redis::cmd("XPENDING")
            .arg(stream)
            .arg("ferrinx-workers")
            .arg("-")
            .arg("+")
            .arg(10)
            .query_async(&mut conn)
            .await?;
        
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        
        // 认领超时任务（5 分钟未确认）
        let entry_ids: Vec<&str> = pending.iter().map(|(id, _, _, _)| id.as_str()).collect();
        
        let claimed: Vec<StreamEntry> = redis::cmd("XCLAIM")
            .arg(stream)
            .arg("ferrinx-workers")
            .arg(consumer)
            .arg(300000) // 5 分钟
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

### 3.5 缓存 API Key

```rust
impl RedisClient {
    /// 获取缓存的 API Key
    pub async fn get_api_key(&self, key_hash: &str) -> Result<Option<ApiKeyInfo>, RedisError> {
        let key = format!("ferrinx:api_keys:{}", key_hash);
        
        let mut conn = self.get_connection().await?;
        
        let value: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await?;
        
        if let Some(json) = value {
            let info: ApiKeyInfo = serde_json::from_str(&json)?;
            return Ok(Some(info));
        }
        
        Ok(None)
    }
    
    /// 缓存 API Key
    pub async fn set_api_key(&self, info: &ApiKeyInfo) -> Result<(), RedisError> {
        let key = format!("ferrinx:api_keys:{}", sha256_hash(info.key.as_deref().unwrap_or("")));
        let json = serde_json::to_string(info)?;
        
        let mut conn = self.get_connection().await?;
        
        redis::cmd("SETEX")
            .arg(&key)
            .arg(3600) // 1 小时
            .arg(&json)
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
    
    /// 删除缓存的 API Key
    pub async fn delete_api_key(&self, key_hash: &str) -> Result<(), RedisError> {
        let key = format!("ferrinx:api_keys:{}", key_hash);
        
        let mut conn = self.get_connection().await?;
        
        redis::cmd("DEL")
            .arg(&key)
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
}
```

### 3.6 缓存推理结果

```rust
impl RedisClient {
    /// 缓存推理结果
    pub async fn set_result(
        &self,
        task_id: &uuid::Uuid,
        result: &serde_json::Value,
    ) -> Result<(), RedisError> {
        let key = format!("ferrinx:results:{}", task_id);
        let json = serde_json::to_string(result)?;
        
        let mut conn = self.get_connection().await?;
        
        redis::cmd("SETEX")
            .arg(&key)
            .arg(86400) // 24 小时
            .arg(&json)
            .query_async(&mut conn)
            .await?;
        
        Ok(())
    }
    
    /// 获取缓存的推理结果
    pub async fn get_result(
        &self,
        task_id: &uuid::Uuid,
    ) -> Result<Option<serde_json::Value>, RedisError> {
        let key = format!("ferrinx:results:{}", task_id);
        
        let mut conn = self.get_connection().await?;
        
        let value: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await?;
        
        if let Some(json) = value {
            let result: serde_json::Value = serde_json::from_str(&json)?;
            return Ok(Some(result));
        }
        
        Ok(None)
    }
}
```

## 4. 降级策略

### 4.1 Redis 不可用时的降级

```rust
pub async fn validate_api_key_with_fallback(
    key: &str,
    redis: &Option<RedisClient>,
    db: &DbContext,
) -> Result<ApiKeyInfo, Error> {
    let key_hash = sha256_hash(key);
    
    // 尝试从 Redis 获取
    if let Some(ref redis) = redis {
        match redis.get_api_key(&key_hash).await {
            Ok(Some(info)) if info.is_active && !is_expired(&info) => {
                return Ok(info);
            }
            _ => {
                warn!("Redis unavailable or cache miss, falling back to database");
            }
        }
    }
    
    // 降级到数据库
    if let Some(record) = db.api_keys.find_by_hash(&key_hash).await? {
        let info = ApiKeyInfo::from(record);
        
        if !info.is_active || is_expired(&info) {
            return Err(Error::InvalidApiKey);
        }
        
        // 异步更新 Redis 缓存（如果 Redis 恢复）
        if let Some(ref redis) = redis {
            let redis_clone = redis.clone();
            let info_clone = info.clone();
            tokio::spawn(async move {
                let _ = redis_clone.set_api_key(&info_clone).await;
            });
        }
        
        return Ok(info);
    }
    
    Err(Error::InvalidApiKey)
}
```

### 4.2 Redis 不可用时的影响

| 功能 | Redis 可用 | Redis 不可用 | 影响 |
|------|-----------|-------------|------|
| 同步推理 | 正常 | 正常 | 无影响 |
| 异步推理 | 正常 | **不可用** | 返回 503 Service Unavailable |
| API Key 验证 | Redis 缓存 | 数据库查询 | 性能下降，但功能正常 |
| 任务状态查询 | Redis 缓存 | 数据库查询 | 性能下降，但功能正常 |

## 5. 监控指标

### 5.1 关键指标

```rust
// Redis 连接数
gauge!("ferrinx_redis_connections_active").set(pool.size() as f64);

// 任务队列长度
counter!("ferrinx_inference_queue_length", "priority" => "high")
    .increment(get_stream_length("ferrinx:tasks:high")?);

// API Key 缓存命中率
counter!("ferrinx_api_key_cache_hits_total").increment(cache_hits);
counter!("ferrinx_api_key_cache_misses_total").increment(cache_misses);

// 任务处理延迟
histogram!("ferrinx_task_processing_duration_seconds")
    .record(processing_time.as_secs_f64());
```

### 5.2 健康检查

```rust
impl RedisClient {
    pub async fn health_check(&self) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await?;
        
        Ok(())
    }
}
```

## 6. 性能优化

### 6.1 连接池配置

```rust
let pool = redis::aio::ConnectionManager::new(redis_client).await?;

let pool_opts = redis::aio::ConnectionPoolOptions::new()
    .max_connections(config.redis.pool_size)
    .min_connections(1)
    .connection_timeout(Duration::from_secs(5));
```

### 6.2 Pipeline 批量操作

```rust
impl RedisClient {
    /// 批量获取 API Key
    pub async fn get_api_keys_batch(
        &self,
        key_hashes: &[String],
    ) -> Result<Vec<Option<ApiKeyInfo>>, RedisError> {
        let mut conn = self.get_connection().await?;
        let mut pipe = redis::pipe();
        
        for key_hash in key_hashes {
            let key = format!("ferrinx:api_keys:{}", key_hash);
            pipe.cmd("GET").arg(&key);
        }
        
        let results: Vec<Option<String>> = pipe.query_async(&mut conn).await?;
        
        Ok(results.into_iter().map(|opt| {
            opt.and_then(|json| serde_json::from_str(&json).ok())
        }).collect())
    }
}
```

## 7. 故障恢复

### 7.1 Worker 宕机恢复

```
场景：Worker 消费任务后宕机，未 XACK

恢复流程：
1. 新 Worker 定期检查 pending 列表（每 5 分钟）
2. 发现超过 5 分钟未确认的任务
3. 使用 XCLAIM 认领这些任务
4. 重新执行推理
```

### 7.2 Redis 宕机恢复

```
场景：Redis 宕机后恢复

恢复流程：
1. API Server 检测到 Redis 恢复
2. 后续请求正常使用 Redis
3. API Key 缓存自动重建（LRU）
4. 任务队列自动恢复（未 ACK 的任务重新分配）
```

## 8. 安全配置

### 8.1 访问控制

```toml
[redis]
url = "redis://:password@localhost:6379/0"
```

### 8.2 网络隔离

```yaml
# Kubernetes NetworkPolicy
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: redis-access
spec:
  podSelector:
    matchLabels:
      app: ferrinx-api
  policyTypes:
  - Ingress
  ingress:
  - from:
    - podSelector:
        matchLabels:
          app: ferrinx-api
    - podSelector:
        matchLabels:
          app: ferrinx-worker
    ports:
    - port: 6379
```

## 9. 设计要点

### 9.1 消费组模式

- 多 Worker 并行消费
- 任务自动重新分配
- 故障恢复自动

### 9.2 优先级队列

- 三个独立 Stream
- 按顺序消费
- 高优先级任务优先处理

### 9.3 缓存策略

- LRU 淘汰（Redis 自动）
- TTL 过期
- 写入时更新

### 9.4 降级设计

- API Key 验证可降级到数据库
- 异步推理依赖 Redis（不可用时返回 503）
- 同步推理不依赖 Redis
