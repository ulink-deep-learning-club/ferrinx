# ferrinx-worker 配置和使用指南

## 概述

`ferrinx-worker` 是 Ferrinx 的分布式推理 Worker 进程，负责从 Redis 任务队列消费异步推理任务并执行计算。

### 主要功能

- **任务消费**：从 Redis Streams 消费异步推理任务
- **模型缓存**：本地 LRU 缓存已加载的模型
- **模型上报**：定期向 Redis 上报本地可用模型状态
- **任务重试**：支持失败任务自动重试
- **死信队列**：处理多次重试失败的任务
- **优雅停机**：安全处理正在执行的任务

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│                      API Server                             │
│  - 接收异步推理请求                                          │
│  - 查询 Redis 获取最佳 Worker                                │
│  - 推送任务到 Redis Streams                                  │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      Redis Layer                            │
│  - ferrinx:tasks:high/normal/low (任务队列)                  │
│  - ferrinx:workers:{id}:models (Worker 模型状态)             │
│  - ferrinx:workers:{id}:heartbeat (Worker 心跳)              │
└──────────────────────────┬──────────────────────────────────┘
                           │
           ┌───────────────┴───────────────┐
           ▼                               ▼
┌──────────────────────┐    ┌──────────────────────────────┐
│     Worker A         │    │        Worker B              │
│  - models: [X, Y]    │    │     - models: [Y, Z]         │
│  - cached: [X]       │    │     - cached: [Z]            │
└──────────────────────┘    └──────────────────────────────┘
```

## 配置

Worker 使用与 API Server 相同的配置文件（`ferrinx.toml`），配置文件中 `[worker]` 部分控制 Worker 行为。

### 完整配置示例

```toml
[server]
host = "127.0.0.1"
port = 8080
workers = 4
max_request_size_mb = 500
graceful_shutdown_timeout = 30
sync_inference_concurrency = 4
sync_inference_timeout = 30
api_version = "v1"
include_version_header = true

[database]
backend = "sqlite"
url = "sqlite://./ferrinx.db"
max_connections = 10
run_migrations = true

[redis]
url = "redis://127.0.0.1:6379"
pool_size = 10
consumer_group = "ferrinx-workers"

[storage]
backend = "local"
path = "./models"

[onnx]
cache_size = 3
execution_provider = "CPU"

# Worker 配置
[worker]
# Worker 消费者名称（留空则自动生成：hostname-pid）
consumer_name = ""

# 并发处理的任务数
concurrency = 4

# 轮询间隔（毫秒）
poll_interval_ms = 100

# 最大重试次数
max_retries = 3

# 重试延迟（毫秒）
retry_delay_ms = 1000

# 任务恢复间隔（秒）：检查并认领超时任务的频率
task_recovery_interval_secs = 300

# 健康检查间隔（秒）
health_check_interval_secs = 30

# 认领空闲时间（毫秒）：超过此时间未确认的任务会被重新认领
claim_idle_ms = 300000

[cleanup]
enabled = true
completed_task_retention_days = 7
failed_task_retention_days = 30
cancelled_task_retention_days = 1
cleanup_interval_hours = 24
cleanup_batch_size = 1000
```

### 配置项说明

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `consumer_name` | String | 自动生成 | Worker 唯一标识，格式为 `hostname-pid` |
| `concurrency` | usize | 4 | 同时处理的任务数 |
| `poll_interval_ms` | u64 | 100 | 轮询 Redis 的间隔 |
| `max_retries` | u32 | 3 | 失败任务的最大重试次数 |
| `retry_delay_ms` | u64 | 1000 | 重试前的延迟 |
| `task_recovery_interval_secs` | u64 | 300 | 检查超时任务的频率 |
| `health_check_interval_secs` | u64 | 30 | Redis 健康检查频率 |
| `claim_idle_ms` | i64 | 300000 | 任务超时时间（5分钟）|

## 启动 Worker

### 基本启动

```bash
# 使用默认配置
./ferrinx-worker

# 指定配置文件
./ferrinx-worker --config /path/to/ferrinx.toml

# 指定环境变量
FERRINX_CONFIG=/path/to/ferrinx.toml ./ferrinx-worker
```

### 使用环境变量

```bash
# 数据库连接
export FERRINX_DATABASE_URL="postgres://user:pass@localhost/ferrinx"

# Redis 连接
export FERRINX_REDIS_URL="redis://127.0.0.1:6379"

# API Key 密钥
export FERRINX_API_KEY_SECRET="your-secret-key"

# 启动 Worker
./ferrinx-worker
```

### Docker 部署

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p ferrinx-worker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates
COPY --from=builder /app/target/release/ferrinx-worker /usr/local/bin/
COPY --from=builder /app/ferrinx.toml /etc/ferrinx/

CMD ["ferrinx-worker", "--config", "/etc/ferrinx/ferrinx.toml"]
```

### Kubernetes 部署

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ferrinx-worker
spec:
  replicas: 3
  selector:
    matchLabels:
      app: ferrinx-worker
  template:
    metadata:
      labels:
        app: ferrinx-worker
    spec:
      containers:
      - name: worker
        image: ferrinx-worker:latest
        args:
          - "--config"
          - "/etc/ferrinx/ferrinx.toml"
        env:
        - name: FERRINX_DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: ferrinx-secrets
              key: database-url
        - name: FERRINX_REDIS_URL
          valueFrom:
            secretKeyRef:
              name: ferrinx-secrets
              key: redis-url
        volumeMounts:
        - name: models
          mountPath: /models
        - name: config
          mountPath: /etc/ferrinx
        resources:
          requests:
            memory: "2Gi"
            cpu: "1000m"
          limits:
            memory: "8Gi"
            cpu: "4000m"
      volumes:
      - name: models
        persistentVolumeClaim:
          claimName: models-pvc
      - name: config
        configMap:
          name: ferrinx-config
```

## 模型路由机制

### 模型状态

Worker 会定期向 Redis 上报本地模型的状态：

- **cached**: 模型已加载到内存（推理最快）
- **available**: 模型文件存在但未加载
- **unavailable**: 模型不存在

### 任务路由流程

1. API 接收异步推理请求
2. 查询 Redis 获取支持该模型的 Worker 列表
3. 优先选择状态为 `cached` 的 Worker
4. 其次选择状态为 `available` 的 Worker
5. 推送任务到对应 Worker 的队列

### 多 Worker 部署建议

```yaml
# 不同 Worker 缓存不同模型
# Worker A: 缓存图像分类模型
# Worker B: 缓存 NLP 模型
# Worker C: 缓存语音模型

# 任务会根据模型类型自动路由到对应 Worker
```

## 监控和日志

### 日志级别

```bash
# 设置日志级别
RUST_LOG=info ./ferrinx-worker
RUST_LOG=debug ./ferrinx-worker
RUST_LOG=error ./ferrinx-worker
```

### 关键日志

```
# Worker 启动
INFO ferrinx_worker: Starting worker: myhost-1234

# 任务处理
INFO ferrinx_worker::processor: Processing task: xxx
INFO ferrinx_worker::processor: Task xxx completed successfully

# 模型加载
INFO ferrinx_core::cache: Loading model xxx into cache
INFO ferrinx_core::cache: Model xxx loaded, cache usage: 2/3

# 错误
ERROR ferrinx_worker::processor: Task xxx execution failed: ...
WARN ferrinx_worker: Redis health check failed: ...
```

### 监控指标

Worker 通过 Redis 上报以下信息：

```bash
# 查看 Worker 心跳
redis-cli GET ferrinx:workers:{worker_id}:heartbeat

# 查看 Worker 模型状态
redis-cli HGETALL ferrinx:workers:{worker_id}:models

# 查看任务队列长度
redis-cli XLEN ferrinx:tasks:high
redis-cli XLEN ferrinx:tasks:normal
redis-cli XLEN ferrinx:tasks:low

# 查看待处理任务
redis-cli XPENDING ferrinx:tasks:normal ferrinx-workers
```

## 故障排除

### 常见问题

#### 1. Worker 无法连接 Redis

```
Error: Redis connection error
```

**解决方案**:
- 检查 Redis 服务是否运行
- 检查 `redis.url` 配置
- 检查网络连通性：`redis-cli -h <host> ping`

#### 2. Worker 不消费任务

```
# 检查 consumer group 是否存在
redis-cli XINFO GROUPS ferrinx:tasks:normal

# 如果不存在，创建 consumer group
redis-cli XGROUP CREATE ferrinx:tasks:normal ferrinx-workers 0 MKSTREAM
redis-cli XGROUP CREATE ferrinx:tasks:high ferrinx-workers 0 MKSTREAM
redis-cli XGROUP CREATE ferrinx:tasks:low ferrinx-workers 0 MKSTREAM
```

#### 3. 任务一直处于 pending 状态

**可能原因**:
- Worker 宕机
- 任务处理超时
- 模型加载失败

**排查步骤**:
```bash
# 1. 检查 Worker 是否存活
redis-cli GET ferrinx:workers:{worker_id}:heartbeat

# 2. 检查待处理任务
redis-cli XPENDING ferrinx:tasks:normal ferrinx-workers - + 10

# 3. 查看任务详情（从 stream 中获取）
redis-cli XRANGE ferrinx:tasks:normal - + COUNT 1
```

#### 4. 模型加载失败

```
Error: Model not found: xxx
```

**解决方案**:
- 检查模型文件是否存在
- 检查 `storage.path` 配置
- 检查模型是否已注册到数据库

### 调试模式

```bash
# 启用详细日志
RUST_LOG=debug ./ferrinx-worker

# 使用 GDB 调试
gdb ./ferrinx-worker
(gdb) run --config ferrinx.toml
```

## 最佳实践

### 1. 生产环境部署

- 至少部署 2 个 Worker 保证高可用
- 使用 Kubernetes 的 HPA 自动扩缩容
- 配置适当的资源限制（内存/CPU）
- 监控 Redis 内存使用情况

### 2. 模型管理

- 预先在 Worker 上部署常用模型
- 合理设置模型缓存大小（`onnx.cache_size`）
- 定期清理不用的模型文件

### 3. 性能优化

- 调整 `concurrency` 匹配 CPU 核心数
- 调整 `poll_interval_ms` 平衡延迟和 CPU 使用
- 使用 SSD 存储模型文件
- 配置适当的任务队列优先级

### 4. 安全配置

- 使用环境变量传递敏感配置
- 限制 Worker 的网络访问（只访问 Redis 和数据库）
- 定期轮换 API Key

## 开发指南

### 本地开发

```bash
# 1. 启动 Redis
docker run -d -p 6379:6379 redis:latest

# 2. 创建配置文件
cat > ferrinx.toml << 'EOF'
[database]
backend = "sqlite"
url = "sqlite://./dev.db"

[redis]
url = "redis://127.0.0.1:6379"

[storage]
path = "./models"

[worker]
concurrency = 2
poll_interval_ms = 100
EOF

# 3. 启动 Worker
cargo run -p ferrinx-worker
```

### 测试

```bash
# 运行 Worker 单元测试
cargo test -p ferrinx-worker --lib

# 运行集成测试
cargo test -p ferrinx-worker --test integration_tests
```
