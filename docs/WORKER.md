# ferrinx-worker 配置和使用指南

## 概述

`ferrinx-worker` 是 Ferrinx 的异步推理 Worker 进程，从 Redis 任务队列消费任务并执行推理。

## 配置

Worker 使用 `ferrinx.toml` 配置文件，配置文件中 `[worker]` 部分控制 Worker 行为。

### 配置示例

```toml
[worker]
consumer_name = ""
concurrency = 4
poll_interval_ms = 100
max_retries = 3
retry_delay_ms = 1000
task_recovery_interval_secs = 300
health_check_interval_secs = 30
claim_idle_ms = 300000
```

### 配置项说明

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `consumer_name` | String | 自动生成 | Worker 唯一标识，格式为 `hostname-pid` |
| `concurrency` | usize | 4 | 同时处理的任务数 |
| `poll_interval_ms` | u64 | 100 | 轮询 Redis 的间隔（毫秒） |
| `max_retries` | u32 | 3 | 失败任务的最大重试次数 |
| `retry_delay_ms` | u64 | 1000 | 重试前的延迟（毫秒） |
| `task_recovery_interval_secs` | u64 | 300 | 检查超时任务的频率（秒） |
| `health_check_interval_secs` | u64 | 30 | Redis 健康检查频率（秒） |
| `claim_idle_ms` | i64 | 300000 | 任务超时时间（毫秒），超时后会被其他 Worker 认领 |

### 完整配置文件示例

```toml
[server]
host = "127.0.0.1"
port = 8080

[database]
backend = "sqlite"
url = "sqlite://./ferrinx.db"

[redis]
url = "redis://127.0.0.1:6379"

[storage]
backend = "local"
path = "./models"

[onnx]
cache_size = 3
execution_provider = "CPU"

[worker]
consumer_name = ""
concurrency = 4
poll_interval_ms = 100
max_retries = 3
retry_delay_ms = 1000
task_recovery_interval_secs = 300
health_check_interval_secs = 30
claim_idle_ms = 300000
```

## 启动 Worker

### 基本启动

```bash
./ferrinx-worker
```

### 指定配置文件

```bash
./ferrinx-worker --config /path/to/ferrinx.toml
```

### 环境变量

```bash
export FERRINX_CONFIG=/path/to/ferrinx.toml
./ferrinx-worker
```

## 环境变量

| 变量名 | 说明 |
|--------|------|
| `FERRINX_CONFIG` | 配置文件路径 |
| `FERRINX_DATABASE_URL` | 数据库连接 URL |
| `FERRINX_REDIS_URL` | Redis 连接 URL |
| `FERRINX_API_KEY_SECRET` | API Key 加密密钥 |

## 工作原理

1. Worker 启动时连接 Redis 和数据库
2. 从 Redis Streams 消费任务（`ferrinx:tasks:high/normal/low`）
3. 执行任务并将结果写回数据库
4. 定期上报心跳和本地模型状态到 Redis

## 日志

```bash
# 设置日志级别
RUST_LOG=info ./ferrinx-worker
RUST_LOG=debug ./ferrinx-worker
```

## 故障排除

### Worker 不消费任务

检查 consumer group 是否存在：

```bash
redis-cli XINFO GROUPS ferrinx:tasks:normal
```

如果不存在，创建 consumer group：

```bash
redis-cli XGROUP CREATE ferrinx:tasks:normal ferrinx-workers 0 MKSTREAM
redis-cli XGROUP CREATE ferrinx:tasks:high ferrinx-workers 0 MKSTREAM
redis-cli XGROUP CREATE ferrinx:tasks:low ferrinx-workers 0 MKSTREAM
```

### 检查 Worker 是否存活

```bash
redis-cli GET ferrinx:workers:{worker_id}:heartbeat
```

### 查看待处理任务

```bash
redis-cli XPENDING ferrinx:tasks:normal ferrinx-workers - + 10
```
