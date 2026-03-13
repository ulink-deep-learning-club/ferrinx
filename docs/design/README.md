# Ferrinx 子系统设计文档索引

本文档提供 Ferrinx 项目各子系统和模块设计文档的索引。

## 设计文档列表

### 核心模块设计

1. **[ferrinx-common 模块设计](./01-common-module.md)**
   - 共享代码库
   - 配置管理
   - 公共类型定义
   - 错误码定义
   - 工具函数

2. **[ferrinx-db 模块设计](./02-db-module.md)**
   - 数据库抽象层
   - Repository trait 定义
   - PostgreSQL/SQLite 实现
   - 事务支持
   - 数据库迁移

3. **[ferrinx-core 模块设计](./03-core-module.md)**
   - ONNX 模型加载与管理
   - 推理引擎执行
   - 模型缓存（LRU）
   - 存储抽象层
   - 并发控制

4. **[ferrinx-api 模块设计](./04-api-module.md)**
   - RESTful API 服务
   - HTTP 路由和请求处理
   - 认证授权中间件
   - 同步/异步推理接口
   - 优雅停机

5. **[ferrinx-worker 模块设计](./05-worker-module.md)**
   - 异步推理 Worker
   - Redis Streams 任务消费
   - 任务处理与重试
   - 死信队列
   - 优雅停机

6. **[ferrinx-cli 模块设计](./06-cli-module.md)**
   - 命令行客户端
   - HTTP 客户端封装
   - 用户交互界面
   - 配置管理
   - 输出格式化

### 补充设计文档

7. **[Redis 设计](./07-redis-design.md)**
   - 任务队列（Redis Streams）
   - 结果缓存
   - API Key 缓存
   - 降级策略
   - 故障恢复

8. **[认证授权设计](./08-auth-design.md)**
   - API Key 机制
   - RBAC 权限模型
   - 用户管理
   - 权限检查
   - 安全考虑

9. **[部署运维设计](./09-deployment-design.md)**
   - 部署架构
   - Docker/Kubernetes 部署
   - 监控与日志
   - 备份与恢复
   - 性能调优

## 模块依赖关系

```
ferrinx-common  ← (被所有 crate 依赖)
    ↑
ferrinx-db      ← (依赖 common)
    ↑
ferrinx-core    ← (依赖 common, db)
    ↑
┌───┴────┐
│        │
ferrinx-api     ferrinx-worker  ← (依赖 common, db, core)
│
ferrinx-cli     ← (仅依赖 common，通过 HTTP 与 API 通信)
```

## 实现顺序

根据依赖关系，推荐按以下顺序实现：

### Phase 1: 基础设施
1. **ferrinx-common** - 配置、类型、常量
2. **ferrinx-db** - 迁移脚本 + PostgreSQL Repository

### Phase 2: 核心引擎
3. **ferrinx-core**
   - 推理引擎（spawn_blocking + Semaphore）
   - 模型缓存（LRU）
   - 本地存储

### Phase 3: API 服务
4. **ferrinx-api**
   - Bootstrap 端点
   - Auth middleware（API Key 验证 + Redis/DB 降级）
   - 基础 routes（models, inference/sync）
   - 限流中间件

### Phase 4: 异步推理
5. **ferrinx-worker**
   - Redis Streams 消费
   - 任务处理
   - 重试与死信队列
   - **模型状态上报**
   - **模型感知任务消费**

### Phase 5: CLI 客户端
6. **ferrinx-cli**
   - HTTP client 封装
   - admin 命令（bootstrap）
   - model/infer 命令

### Phase 6: 完善功能
7. 清理任务（过期任务、临时 Key）
8. 监控指标（Prometheus）
9. 文档与测试

## 关键设计决策

### 1. Tensor 数据格式

**设计原则**: 所有推理输入/输出统一使用显式 Tensor 格式，替代隐式的嵌套 JSON 数组。

**Tensor 结构**:
```json
{
  "dtype": "float32",  // float32 | int8 | int64
  "shape": [1, 3, 224, 224],
  "data": "<base64-encoded-binary>"
}
```

**设计优势**:
- **显式形状** : 不再有隐式 shape 推断错误
- **类型安全** : dtype 显式声明，避免类型混淆
- **二进制效率** : base64 编码比 JSON 数组紧凑 30-50%
- **严格验证** : 推理引擎强制 shape 匹配，提前发现错误
- 
```json
{
  "input": {
    "dtype": "float32",
    "shape": [1, 2, 2],
    "data": "AACAPwAAAEAAAEBA"
  }
}
```

### 2. 双模式架构

- **简化模式（无 Redis）**：
  - API Server 独立运行，内置 InferenceEngine
  - 仅支持同步推理（sync_infer）
  - API Key 验证直接查数据库
  - 适合：开发环境、单机部署、无外部依赖场景

- **完整模式（有 Redis）**：
  - API Server + Worker Pool 分布式部署
  - 支持同步和异步推理
  - 异步任务路由到最优 Worker
  - 适合：生产环境、需要水平扩展场景

### 3. 同步 vs 异步推理路径

| 特性 | 同步推理 | 异步推理 |
|------|---------|---------|
| 执行位置 | API 进程内 | Worker 进程 |
| Redis 依赖 | 不依赖 | 必须依赖 |
| 模型来源 | API 本地存储 | Worker 存储 |
| 模型路由 | 不适用 | 智能路由 |
| 延迟 | < 100ms | 可变 |
| 适用场景 | 快速响应 | 大模型、批处理 |

### 4. 模型路由策略（仅异步推理）

Worker 上报模型状态到 Redis，API 路由任务时按优先级选择：

```
优先级 1: Worker 已缓存模型（最快）
优先级 2: Worker 有模型文件但未缓存（需加载）
优先级 3: 无 Worker 有模型 → 返回错误
```

### 5. API Key 不存储明文

- 数据库存储 SHA-256 哈希
- Redis 缓存验证结果（有 Redis 时）
- 数据库降级保证可用性（无 Redis 时）

### 6. PostgreSQL 和 SQLite 双后端

- PostgreSQL：生产环境
- SQLite：开发/测试环境
- 业务代码依赖 trait，不依赖具体实现

### 7. 模块化设计

- 轻量级 CLI（不依赖 core/db）
- 独立 Worker 进程
- 可水平扩展

### 8. 优雅降级

**无 Redis 时（简化模式）**：
- ✅ 同步推理可用（API 本地执行）
- ❌ 异步推理不可用（返回 503）
- ✅ API Key 验证降级到数据库
- 适合开发环境和简单部署场景

**有 Redis 时（完整模式）**：
- ✅ 同步推理可用（API 本地执行）
- ✅ 异步推理可用（路由到 Worker）
- ✅ 智能模型路由（优先已缓存的 Worker）
- ✅ API Key 缓存加速
- 适合生产环境和分布式部署

## 技术栈总结

| 类别 | 技术选型 |
|------|---------|
| 语言 | Rust |
| Web 框架 | axum |
| ONNX Runtime | ort |
| 数据库 | PostgreSQL / SQLite (sqlx) |
| 缓存/队列 | Redis |
| 序列化 | serde / serde_json |
| 配置 | config + toml |
| CLI | clap |
| 日志 | tracing |
| 异步运行时 | tokio |
| 错误处理 | thiserror / anyhow |

## 性能指标

### 目标性能

| 指标 | 目标值 |
|------|--------|
| 同步推理延迟 | < 100ms (不含模型加载) |
| 模型加载时间 | < 5s (首次) |
| 并发推理数 | 4-8 (可配置) |
| 模型缓存命中 | > 90% |
| API 响应时间 | < 50ms (不含推理) |

### 容量规划

| 组件 | 单节点容量 | 备注 |
|------|-----------|------|
| PostgreSQL | 5000+ TPS | 非瓶颈 |
| Redis | 50000+ QPS | 任务队列+缓存 |
| API Server | 1000+ QPS | 同步推理 |
| Worker | 100+ tasks/min | 异步推理 |

## 安全要点

1. **API Key 安全**
   - 不存储明文
   - SHA-256 哈希
   - HTTPS 传输

2. **密码安全**
   - bcrypt 哈希
   - 加盐存储

3. **网络安全**
   - HTTPS 加密
   - 网络隔离
   - Secret 管理

4. **访问控制**
   - RBAC 权限模型
   - 细粒度权限控制
   - 审计日志

## 监控指标

### Prometheus 指标

- `ferrinx_inference_requests_total{mode, status}` - 推理请求数
- `ferrinx_inference_duration_seconds{mode}` - 推理延迟
- `ferrinx_model_cache_hits_total` - 缓存命中数
- `ferrinx_model_cache_misses_total` - 缓存未命中数
- `ferrinx_sync_inference_concurrent_current` - 当前并发数
- `ferrinx_redis_connections_active` - Redis 连接数
- `ferrinx_db_connections_active` - 数据库连接数

### 健康检查

- `/api/v1/health` - 服务健康状态
- `/api/v1/ready` - 服务就绪状态（检查依赖）

### 分布式追踪

使用 OpenTelemetry 实现分布式追踪：

- **Trace Context**: 自动传播 W3C Trace Context
- **Span 覆盖**:
  - HTTP 请求处理
  - 推理执行
  - 数据库查询
  - Redis 操作
- **导出**: 支持 Jaager/Zipkin/OTLP

## API 版本策略

### 版本控制

- URL 路径版本：`/api/v1/`、`/api/v2/`
- 向后兼容承诺：v1 API 至少维护 12 个月

### 兼容性规则

| 变更类型 | 兼容性 | 示例 |
|---------|--------|------|
| 新增端点 | 兼容 | 新增 `/api/v1/models/{id}/stats` |
| 新增可选字段 | 兼容 | 响应新增 `metadata` 字段 |
| 新增必填字段 | 不兼容 | 请求新增必填 `priority` |
| 删除字段 | 不兼容 | 删除响应中的 `legacy_field` |
| 修改字段类型 | 不兼容 | `id` 从 int 改为 string |

### 版本迁移

1. 发布新版本 API
2. 旧版本标记 deprecated
3. 文档标注迁移指南
4. 12 个月后移除旧版本

## 模型版本管理

### 版本策略

- 模型通过 `name` + `version` 唯一标识
- 支持同一模型的多个版本并存
- 默认推理使用最新有效版本

### 版本回滚

```bash
# 1. 查看模型历史版本
ferrinx model list --name my-model

# 2. 标记问题版本为无效
ferrinx model update <model-id> --valid false

# 3. 推理自动使用上一个有效版本
# 或显式指定版本
ferrinx infer sync --model-id <old-version-id> --input data.json
```

### 版本别名（未来扩展）

```bash
# 设置版本别名
ferrinx model alias my-model v1.0.0 --alias production

# 使用别名推理
ferrinx infer sync --model my-model:production --input data.json
```

## 下一步

1. 阅读 [总设计文档](../../design.md)
2. 按实现顺序开发各模块
3. 编写单元测试和集成测试
4. 准备生产环境部署配置
5. 配置监控和告警
