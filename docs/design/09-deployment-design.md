# 部署运维设计

## 1. 部署架构

### 1.1 单机部署

适用于开发、测试和小规模生产环境。

```
┌─────────────────────────────────────┐
│           单机服务器                  │
│  ┌────────────────────────────────┐ │
│  │  ferrinx-api (同步推理)         │ │
│  │  - port: 8080                  │ │
│  │  - workers: 4                  │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  ferrinx-worker (异步推理)      │ │
│  │  - concurrency: 4              │ │
│  │  - instances: 1                │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  PostgreSQL                    │ │
│  │  - port: 5432                  │ │
│  │  - database: ferrinx           │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  Redis                         │ │
│  │  - port: 6379                  │ │
│  └────────────────────────────────┘ │
└─────────────────────────────────────┘
```

### 1.2 分布式部署

适用于生产环境，支持高可用和水平扩展。

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  API Server  │  │  API Server  │  │  API Server  │
│   (Node 1)   │  │   (Node 2)   │  │   (Node 3)   │
│  同步推理     │  │  同步推理     │  │  同步推理     │
│  有状态       │  │  有状态       │  │  有状态       │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                  │                  │
       └──────────────────┼──────────────────┘
                          │
              ┌───────────┴───────────┐
              │   Load Balancer       │
              │   (一致性哈希路由)     │
              └───────────┬───────────┘
                          │
       ┌──────────────────┼──────────────────┐
       │                  │                  │
┌──────▼──────┐  ┌───────▼──────┐  ┌───────▼──────┐
│ Worker 1    │  │  Worker 2    │  │  Worker 3    │
│ 异步推理     │  │  异步推理     │  │  异步推理     │
└──────┬──────┘  └───────┬──────┘  └───────┬──────┘
       │                  │                  │
       └──────────────────┼──────────────────┘
                          │
              ┌───────────┴───────────┐
              │      Redis Cluster    │
              │  (Streams + 缓存)      │
              └───────────┬───────────┘
                          │
              ┌───────────┴───────────┐
              │      PostgreSQL       │
              │      (主从复制)        │
              └───────────┬───────────┘
                          │
              ┌───────────┴───────────┐
              │   S3 / NFS Storage    │
              │   (模型文件存储)       │
              └───────────────────────┘
```

## 2. Docker 部署

### 2.1 Dockerfile

```dockerfile
# ferrinx-api
FROM rust:1.75 as builder

WORKDIR /app
COPY . .
RUN cargo build --release -p ferrinx-api

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ferrinx-api /usr/local/bin/
COPY config.example.toml /etc/ferrinx/config.toml

EXPOSE 8080
CMD ["ferrinx-api", "--config", "/etc/ferrinx/config.toml"]
```

```dockerfile
# ferrinx-worker
FROM rust:1.75 as builder

WORKDIR /app
COPY . .
RUN cargo build --release -p ferrinx-worker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ferrinx-worker /usr/local/bin/
COPY config.example.toml /etc/ferrinx/config.toml

CMD ["ferrinx-worker", "--config", "/etc/ferrinx/config.toml"]
```

### 2.2 Docker Compose

```yaml
version: '3.8'

services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_DB: ferrinx
      POSTGRES_USER: ferrinx
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres_data:/var/lib/postgresql/data
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ferrinx"]
      interval: 10s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5

  api:
    build:
      context: .
      dockerfile: Dockerfile.api
    ports:
      - "8080:8080"
    environment:
      FERRINX_DATABASE_URL: postgresql://ferrinx:${POSTGRES_PASSWORD}@postgres:5432/ferrinx
      FERRINX_REDIS_URL: redis://redis:6379
      FERRINX_API_KEY_SECRET: ${API_KEY_SECRET}
    volumes:
      - ./models:/models
      - ./config.toml:/etc/ferrinx/config.toml:ro
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/api/v1/health"]
      interval: 30s
      timeout: 10s
      retries: 3

  worker:
    build:
      context: .
      dockerfile: Dockerfile.worker
    environment:
      FERRINX_DATABASE_URL: postgresql://ferrinx:${POSTGRES_PASSWORD}@postgres:5432/ferrinx
      FERRINX_REDIS_URL: redis://redis:6379
    volumes:
      - ./models:/models
      - ./config.toml:/etc/ferrinx/config.toml:ro
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy

volumes:
  postgres_data:
```

## 3. Kubernetes 部署

### 3.1 ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: ferrinx-config
data:
  config.toml: |
    [server]
    host = "0.0.0.0"
    port = 8080
    workers = 4
    max_request_size_mb = 500
    graceful_shutdown_timeout = 30
    sync_inference_concurrency = 4
    sync_inference_timeout = 30
    
    [database]
    backend = "postgresql"
    max_connections = 10
    run_migrations = false
    
    [redis]
    pool_size = 10
    stream_key = "ferrinx:tasks:stream"
    consumer_group = "ferrinx-workers"
    dead_letter_stream = "ferrinx:tasks:dead_letter"
    result_cache_ttl = 86400
    api_key_cache_ttl = 3600
    fallback_to_db = true
    
    [storage]
    backend = "local"
    path = "/models"
    
    [onnx]
    cache_size = 5
    execution_provider = "CPU"
    
    [logging]
    level = "info"
    format = "json"
```

### 3.2 Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: ferrinx-secrets
type: Opaque
stringData:
  database-url: postgresql://ferrinx:password@postgres:5432/ferrinx
  redis-url: redis://redis:6379
  api-key-secret: your-secret-key-here
```

### 3.3 API Server Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ferrinx-api
spec:
  replicas: 3
  selector:
    matchLabels:
      app: ferrinx-api
  template:
    metadata:
      labels:
        app: ferrinx-api
    spec:
      containers:
      - name: api
        image: ferrinx-api:latest
        ports:
        - containerPort: 8080
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
        - name: FERRINX_API_KEY_SECRET
          valueFrom:
            secretKeyRef:
              name: ferrinx-secrets
              key: api-key-secret
        volumeMounts:
        - name: config
          mountPath: /etc/ferrinx
          readOnly: true
        - name: models
          mountPath: /models
        resources:
          limits:
            cpu: "4"
            memory: "8Gi"
          requests:
            cpu: "2"
            memory: "4Gi"
        livenessProbe:
          httpGet:
            path: /api/v1/health
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /api/v1/ready
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
      volumes:
      - name: config
        configMap:
          name: ferrinx-config
      - name: models
        persistentVolumeClaim:
          claimName: ferrinx-models-pvc
```

### 3.4 Worker Deployment

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
        - name: config
          mountPath: /etc/ferrinx
          readOnly: true
        - name: models
          mountPath: /models
        resources:
          limits:
            cpu: "4"
            memory: "8Gi"
          requests:
            cpu: "2"
            memory: "4Gi"
      volumes:
      - name: config
        configMap:
          name: ferrinx-config
      - name: models
        persistentVolumeClaim:
          claimName: ferrinx-models-pvc
```

### 3.5 Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: ferrinx-api
spec:
  selector:
    app: ferrinx-api
  ports:
  - port: 80
    targetPort: 8080
  type: LoadBalancer
```

### 3.6 Ingress

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: ferrinx-ingress
  annotations:
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
    nginx.ingress.kubernetes.io/proxy-body-size: "500m"
spec:
  tls:
  - hosts:
    - api.ferrinx.example.com
    secretName: ferrinx-tls
  rules:
  - host: api.ferrinx.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: ferrinx-api
            port:
              number: 80
```

## 4. 监控与日志

### 4.1 Prometheus 指标

```rust
// src/metrics.rs

use metrics::{counter, histogram, gauge};

pub fn record_inference_request(mode: &str, status: &str) {
    counter!("ferrinx_inference_requests_total", 
        "mode" => mode, 
        "status" => status
    ).increment(1);
}

pub fn record_inference_duration(mode: &str, latency_ms: u64) {
    histogram!("ferrinx_inference_duration_seconds", 
        "mode" => mode
    ).record(latency_ms as f64 / 1000.0);
}

pub fn update_cache_metrics(loaded_models: usize, max_size: usize) {
    gauge!("ferrinx_model_cache_size").set(loaded_models as f64);
    gauge!("ferrinx_model_cache_max_size").set(max_size as f64);
}

pub fn update_concurrency_metrics(available: usize, total: usize) {
    gauge!("ferrinx_sync_inference_concurrent_current")
        .set((total - available) as f64);
    gauge!("ferrinx_sync_inference_concurrent_limit")
        .set(total as f64);
}
```

### 4.2 Prometheus 配置

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: prometheus-config
data:
  prometheus.yml: |
    global:
      scrape_interval: 15s
    
    scrape_configs:
    - job_name: 'ferrinx-api'
      kubernetes_sd_configs:
      - role: pod
      relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        action: keep
        regex: ferrinx-api
      - source_labels: [__meta_kubernetes_pod_ip]
        target_label: __address__
        replacement: ${1}:8080
```

### 4.3 Grafana Dashboard

```json
{
  "dashboard": {
    "title": "Ferrinx Monitoring",
    "panels": [
      {
        "title": "Inference Requests",
        "type": "graph",
        "targets": [
          {
            "expr": "rate(ferrinx_inference_requests_total[5m])",
            "legendFormat": "{{mode}} - {{status}}"
          }
        ]
      },
      {
        "title": "Inference Latency",
        "type": "graph",
        "targets": [
          {
            "expr": "histogram_quantile(0.95, rate(ferrinx_inference_duration_seconds_bucket[5m]))",
            "legendFormat": "95th percentile"
          }
        ]
      },
      {
        "title": "Model Cache",
        "type": "graph",
        "targets": [
          {
            "expr": "ferrinx_model_cache_size",
            "legendFormat": "Loaded Models"
          }
        ]
      },
      {
        "title": "Concurrent Inference",
        "type": "graph",
        "targets": [
          {
            "expr": "ferrinx_sync_inference_concurrent_current",
            "legendFormat": "Current"
          },
          {
            "expr": "ferrinx_sync_inference_concurrent_limit",
            "legendFormat": "Limit"
          }
        ]
      }
    ]
  }
}
```

### 4.4 日志配置

```rust
// src/logging.rs

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_logging(config: &LoggingConfig) -> Result<(), Error> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.level));
    
    let json_layer = if config.format == LogFormat::Json {
        Some(tracing_subscriber::fmt::layer().json())
    } else {
        None
    };
    
    let text_layer = if config.format == LogFormat::Text {
        Some(tracing_subscriber::fmt::layer())
    } else {
        None
    };
    
    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(text_layer)
        .init();
    
    Ok(())
}
```

### 4.5 日志示例

```json
{
  "timestamp": "2024-01-01T10:00:00.000Z",
  "level": "INFO",
  "target": "ferrinx_api::handlers::inference",
  "message": "Inference completed",
  "request_id": "req-abc-123",
  "model_id": "model-123",
  "user_id": "user-456",
  "latency_ms": 45,
  "status": "success"
}
```

## 5. 备份与恢复

### 5.1 数据库备份

```bash
#!/bin/bash
# backup.sh

DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="/backups"
DB_NAME="ferrinx"

# PostgreSQL 备份
pg_dump -h localhost -U ferrinx $DB_NAME | gzip > $BACKUP_DIR/ferrinx_$DATE.sql.gz

# 保留最近 7 天的备份
find $BACKUP_DIR -name "ferrinx_*.sql.gz" -mtime +7 -delete
```

### 5.2 模型文件备份

```bash
#!/bin/bash
# backup_models.sh

DATE=$(date +%Y%m%d_%H%M%S)
MODEL_DIR="/models"
BACKUP_DIR="/backups/models"

# 同步到备份目录
rsync -av --delete $MODEL_DIR/ $BACKUP_DIR/

# 或上传到 S3
aws s3 sync $MODEL_DIR s3://ferrinx-backups/models/$DATE/
```

### 5.3 恢复流程

```bash
#!/bin/bash
# restore.sh

BACKUP_FILE=$1
DB_NAME="ferrinx"

# 恢复数据库
gunzip -c $BACKUP_FILE | psql -h localhost -U ferrinx $DB_NAME

# 恢复模型文件（如果需要）
# rsync -av /backups/models/ /models/
```

## 6. 性能调优

### 6.1 PostgreSQL 调优

```sql
-- postgresql.conf

# 连接
max_connections = 100

# 内存
shared_buffers = 2GB
effective_cache_size = 6GB
work_mem = 16MB
maintenance_work_mem = 512MB

# WAL
wal_buffers = 16MB
checkpoint_completion_target = 0.9

# 查询规划
random_page_cost = 1.1
effective_io_concurrency = 200
```

### 6.2 Redis 调优

```conf
# redis.conf

# 内存
maxmemory 4gb
maxmemory-policy allkeys-lru

# 持久化（可选）
save 900 1
save 300 10
save 60 10000

# 网络
tcp-backlog 511
timeout 0
tcp-keepalive 300
```

### 6.3 系统调优

```bash
# /etc/sysctl.conf

# 网络优化
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 30

# 文件描述符
fs.file-max = 100000
```

## 7. 故障排查

### 7.1 常见问题

#### API 服务无响应

```bash
# 检查服务状态
kubectl get pods -l app=ferrinx-api

# 查看日志
kubectl logs -f deployment/ferrinx-api

# 检查资源使用
kubectl top pods

# 检查数据库连接
psql -h postgres -U ferrinx -d ferrinx -c "SELECT count(*) FROM pg_stat_activity;"
```

#### Worker 处理缓慢

```bash
# 检查 Redis 队列长度
redis-cli XLEN ferrinx:tasks:high
redis-cli XLEN ferrinx:tasks:normal
redis-cli XLEN ferrinx:tasks:low

# 检查 pending 任务
redis-cli XPENDING ferrinx:tasks:normal ferrinx-workers

# 查看 Worker 日志
kubectl logs -f deployment/ferrinx-worker
```

### 7.2 性能分析

```bash
# CPU 分析
perf record -g -p <pid>
perf report

# 内存分析
valgrind --tool=massif ./ferrinx-api

# 网络分析
tcpdump -i eth0 port 8080 -w capture.pcap
```

## 8. 设计要点

### 8.1 高可用

- 多副本部署
- 健康检查
- 自动重启
- 负载均衡

### 8.2 可观测性

- Prometheus 指标
- Grafana 可视化
- 结构化日志
- Request ID 追踪

### 8.3 安全性

- HTTPS 加密
- API Key 认证
- 网络隔离
- Secret 管理

### 8.4 可维护性

- 配置管理
- 备份恢复
- 滚动更新
- 故障排查工具
