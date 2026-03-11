# Ferrinx - Rust ONNX 推理后端架构设计

## 1. 系统概述

Ferrinx 是一个基于 Rust 的高性能 ONNX 推理后端服务，采用分层架构设计，支持 CLI 和 RESTful API 两种交互方式，同时支持同步和异步推理模式。

### 核心设计特性

- 基于 `ort` 的高性能 ONNX 模型推理
- **同步/异步双模式推理**：毫秒级低延迟场景用同步，批处理/大模型用异步
- RESTful API 支持，带 API Key 验证
- CLI 客户端，通过 HTTP 与服务端通信
- Redis Streams 作为任务队列和缓存层（支持降级）
- PostgreSQL/SQLite 数据持久化（可切换）
- 模块化设计，数据库后端与业务代码解耦
- 独立的推理 Worker，可水平扩展

## 2. 整体架构

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              客户端层                                      │
│  ┌──────────────┐                    ┌──────────────────────┐            │
│  │   CLI Tool   │                    │   RESTful Client     │            │
│  │  (独立进程)   │                    │   (外部应用/前端)     │            │
│  └──────┬───────┘                    └──────────┬───────────┘            │
└─────────┼──────────────────────────────────────┼─────────────────────────┘
          │ HTTP/JSON                          │ HTTP/JSON
          ▼                                     ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                           API Gateway 层                                   │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │  API Server (axum)                                                  │  │
│  │  - API Key 验证 (Redis + DB 降级)                                   │  │
│  │  - 同步推理（直接调用 Inference Engine，有状态）                    │  │
│  │  - 异步推理（推送任务到 Redis Streams）                             │  │
│  │  - 请求路由、限流控制                                                │  │
│  └────────────────────────────┬───────────────────────────────────────┘  │
└───────────────────────────────┼──────────────────────────────────────────┘
                                │
                    ┌───────────┴───────────┐
                    │                       │
                    ▼                       ▼
┌─────────────────────────────┐ ┌─────────────────────────────────────────┐
│    同步推理路径（低延迟）     │ │        异步推理路径（高吞吐）            │
│                             │ │                                         │
│  ⚠️ 有状态：进程内缓存模型   │ │  ┌──────────────┐  ┌─────────────────┐  │
│                             │ │  │ Redis        │  │ Inference Worker│  │
│  ┌───────────────────────┐  │ │  │ Streams      │  │  (独立进程)      │  │
│  │  Inference Engine     │  │ │  │  多优先级队列 │  │                 │  │
│  │  - 模型缓存 (LRU)     │  │ │  └──────┬───────┘  └────────┬────────┘  │
│  │  - 并发限制 (Semaphore)│  │ │         │                   │           │
│  │  - spawn_blocking     │  │ │         │                   ▼           │
│  └───────────────────────┘  │ │         │         ┌─────────────────┐  │
│                             │ │         └────────>│ Inference Engine│  │
└─────────────────────────────┘ │                   │  (模型缓存)      │  │
                                │                   └─────────────────┘  │
                                └─────────────────────────────────────────┘
                                                │
                                                ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                         基础设施层 (Infrastructure)                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────────┐   │
│  │ Redis        │  │  Database    │  │  Model Storage               │   │
│  │ - Streams    │  │  (PG/SQLite) │  │  - Local / S3 / NFS          │   │
│  │ - 结果缓存    │  │  - 数据持久化 │  │  - 存储抽象层                 │   │
│  │ - API Key状态 │  │  - 事务支持   │  │                              │   │
│  │ (支持降级)    │  │              │  │                              │   │
│  └──────────────┘  └──────────────┘  └──────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  ONNX Runtime (ort)                                               │   │
│  │  - 模型推理 (CPU 密集，spawn_blocking)                            │   │
│  │  - GPU 加速支持                                                    │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────┘
```

### 架构说明

#### 1. API Server（有状态服务）

**重要**：同步推理模式下，API Server 是**有状态服务**，因为需要在进程内缓存 ONNX 模型。

**状态特性**：
- 进程内 LRU 模型缓存
- 模型加载后的 Session 对象

**水平扩展策略**：
- **方案 A（推荐）**：使用一致性哈希路由，按 `model_id` 将请求固定路由到特定节点
  - 确保同一模型的请求始终路由到缓存了该模型的节点
  - 减少冷加载次数
  
- **方案 B**：所有节点预加载相同的热门模型子集
  - 配置文件中指定 `preload` 模型列表
  - 适合热门模型数量固定的场景
  
- **方案 C**：接受缓存不命中
  - 节点随机路由
  - cache miss 时冷加载（首次请求延迟较高）

**无状态部分**：
- 异步推理请求处理
- API Key 验证（Redis + DB 降级）

#### 2. Inference Worker（任务分配层面无状态）

独立进程，负责异步推理：
- 从 Redis Streams 消费任务（消费组模式）
- 执行推理并存储结果
- 支持多个 Worker 实例并行消费
- Worker 宕机时，未确认的任务自动重新分配
- 支持优雅停机

**状态性说明**：
- **任务分配层面**：无状态 — 任何 Worker 都能处理任何任务，无需亲和性
- **运行时层面**：有状态 — Worker 进程内有模型 LRU 缓存（ONNX Session 对象）
  - 同一模型的任务发到已缓存该模型的 Worker 效率更高
  - 频繁在不同 Worker 间切换同一模型会导致重复加载
  
**优化方向**（可选，后期）：
- 按 `model_id` 做消费亲和性路由
- Worker 分组，每组负责特定模型子集

**v1 策略**：接受运行时有状态，依赖 Redis Streams 自动任务分配（XREADGROUP）。

#### 3. Redis

- **任务队列**：Redis Streams（消费组 + ACK）
- **结果缓存**：临时存储推理结果
- **API Key 状态缓存**：加速验证
- **支持降级**：Redis 不可用时，同步推理仍可工作，API Key 验证降级到数据库

## 3. 模块设计与依赖关系

### 3.1 模块依赖图

```
ferrinx-common  ← (被所有 crate 依赖，公共类型、配置、常量)
    ↑
ferrinx-db      ← (依赖 common，数据库抽象层)
    ↑
ferrinx-core    ← (依赖 common, db，推理引擎核心)
    ↑
┌───┴────┐
│        │
▼        ▼
ferrinx-api     ferrinx-worker  ← (依赖 common, db, core)
│        ↑
│        │ (仅 HTTP client，不依赖 core/db)
▼
ferrinx-cli     ← (依赖 common，轻量级 HTTP 客户端)
```

**关键点**：
- `ferrinx-cli` **不依赖** `core` 和 `db`，只通过 HTTP 与 API 通信
- 避免把整个推理引擎编译进 CLI 二进制文件
- CLI 体积小，部署方便

### 3.2 项目目录结构

```
ferrinx/
├── Cargo.toml                 # Workspace 配置
├── design.md                  # 架构设计文档
├── config.example.toml        # 配置文件示例
│
├── crates/
│   ├── ferrinx-common/        # 共享代码
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs      # 配置结构
│   │       ├── types.rs       # 公共类型
│   │       ├── constants.rs   # 常量定义
│   │       └── utils.rs       # 工具函数
│   │
│   ├── ferrinx-db/            # 数据库抽象层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs      # Repository trait 定义
│   │       ├── transaction.rs # 事务支持
│   │       ├── repositories/  # 各领域 Repository
│   │       │   ├── mod.rs
│   │       │   ├── model.rs   # ModelRepository
│   │       │   ├── task.rs    # TaskRepository
│   │       │   ├── api_key.rs # ApiKeyRepository
│   │       │   └── user.rs    # UserRepository
│   │       ├── backends/
│   │       │   ├── mod.rs
│   │       │   ├── postgres/  # PostgreSQL 实现
│   │       │   └── sqlite/    # SQLite 实现
│   │       └── migrations/    # 数据库迁移
│   │           ├── 20240101_000001_create_users.sql
│   │           ├── 20240101_000002_create_api_keys.sql
│   │           ├── 20240101_000003_create_models.sql
│   │           └── 20240101_000004_create_inference_tasks.sql
│   │
│   ├── ferrinx-core/          # 核心业务逻辑
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── model/         # 模型管理
│   │       │   ├── mod.rs
│   │       │   ├── loader.rs  # 模型加载器
│   │       │   └── cache.rs   # 模型缓存 (LRU)
│   │       ├── inference/     # 推理引擎
│   │       │   ├── mod.rs
│   │       │   ├── engine.rs  # 推理执行器（含 spawn_blocking）
│   │       │   ├── session.rs # ONNX Session 管理
│   │       │   └── limiter.rs # 并发限制 (Semaphore)
│   │       ├── storage/       # 模型存储抽象
│   │       │   ├── mod.rs
│   │       │   ├── local.rs   # 本地文件系统
│   │       │   └── s3.rs      # S3 存储 (可选)
│   │       └── error.rs       # 错误定义
│   │
│   ├── ferrinx-api/           # RESTful API 服务
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── routes/        # 路由定义
│   │       ├── handlers/      # 请求处理器
│   │       │   ├── mod.rs
│   │       │   ├── model.rs
│   │       │   ├── inference.rs  # 同步/异步推理
│   │       │   └── api_key.rs
│   │       ├── middleware/    # 中间件
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs    # API Key 验证
│   │       │   └── rate_limit.rs
│   │       ├── dto/           # 数据传输对象
│   │       └── shutdown.rs    # 优雅停机
│   │
│   ├── ferrinx-worker/        # 推理 Worker (独立进程)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs        # Worker 入口
│   │       ├── consumer.rs    # Redis Streams 消费
│   │       ├── processor.rs   # 推理处理
│   │       └── retry.rs       # 重试与死信队列
│   │
│   └── ferrinx-cli/           # CLI 客户端
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── commands/      # 子命令
│           │   ├── mod.rs
│           │   ├── admin.rs   # 管理命令（创建用户等）
│           │   ├── model.rs
│           │   ├── infer.rs
│           │   └── api_key.rs
│           └── output.rs      # 输出格式化
│
└── tests/                     # 集成测试
    ├── integration_test.rs
    └── fixtures/
        └── test_model.onnx
```

### 3.3 核心模块说明

#### ferrinx-common
共享代码库（所有 crate 依赖）：
- 配置文件结构定义
- 公共类型定义
- 常量和工具函数
- **无重型依赖**（不依赖 ort、sqlx 等）

#### ferrinx-db
数据库抽象层：
- 按领域拆分 Repository traits
- 提供 PostgreSQL 和 SQLite 两种实现
- 包含数据库迁移脚本
- **事务支持**：跨 Repository 操作
- 业务代码只依赖 trait，不依赖具体实现

#### ferrinx-core
核心业务逻辑层：
- **模型管理**: ONNX 模型加载、验证、版本管理、LRU 缓存
- **推理引擎**: 基于 ort 的推理执行，支持 GPU 加速
  - `spawn_blocking` 执行 CPU 密集推理
  - 并发限制（Semaphore）
  - 超时控制
- **模型存储抽象**: 本地文件系统 / S3 / NFS
- **错误定义**: 统一的错误类型

#### ferrinx-api
RESTful API 服务：
- 提供 HTTP 接口
- **同步推理**：进程内执行，有状态（模型缓存）
- **异步推理**：推送到 Redis Streams
- API Key 验证（Redis 缓存 + 数据库降级）
- 请求限流、优雅停机

#### ferrinx-worker
独立部署的推理 Worker 进程：
- 从 Redis Streams 消费任务（消费组模式）
- 执行推理并存储结果
- 支持重试和死信队列
- Worker 宕机时未确认任务自动重新分配

#### ferrinx-cli
命令行客户端：
- 独立的二进制文件
- 通过 HTTP 与 API 服务通信
- **不依赖 core/db**，轻量级
- 支持管理员命令（创建用户等）

## 4. 数据流设计

### 4.1 同步推理流程（低延迟场景）

适用于推理时间 < 100ms 的场景，如图像分类、文本 embedding 等。

```
CLI/Client                API Server              Inference Engine
    │                         │                         │
    │ POST /inference/sync    │                         │
    │ (api-key + model_id)    │                         │
    ├────────────────────────>│                         │
    │                         │ Validate API Key       │
    │                         │ (Redis/DB)             │
    │                         │                         │
    │                         │ Acquire Semaphore      │
    │                         │ (并发限制)              │
    │                         │                         │
    │                         │ Get Model from Cache   │
    │                         ├────────────────────────>│
    │                         │                         │
    │                         │ spawn_blocking         │
    │                         ├────────────────────────>│
    │                         │ Execute Inference      │
    │                         │ (CPU 密集)             │
    │                         │ Return Result          │
    │                         │<────────────────────────┤
    │                         │                         │
    │                         │ Release Semaphore      │
    │                         │                         │
    │ Return Result           │                         │
    │<────────────────────────┤                         │
    │                         │                         │
    │                         │ [Async] Log to DB      │
    │                         │ (审计用，fire & forget)│
```

**关键点**：
- 单次 HTTP 请求-响应
- `spawn_blocking` 执行 CPU 密集推理（不阻塞 tokio 运行时）
- Semaphore 限制并发推理数（防止内存耗尽）
- 超时保护（默认 30s）
- 异步记录日志（可选，不影响响应延迟）

### 4.2 异步推理流程（批处理/大模型场景）

适用于推理时间较长或需要批处理的场景。

```
CLI/Client          API Server        Redis Streams    Worker        Database
    │                   │                 │               │              │
    │ POST /inference   │                 │               │              │
    ├──────────────────>│                 │               │              │
    │                   │ Validate API Key│               │              │
    │                   ├────────────────>│               │              │
    │                   │                 │               │              │
    │                   │ XADD to Stream  │               │              │
    │                   │ (with priority) │               │              │
    │                   ├────────────────>│               │              │
    │                   │                 │               │              │
    │ Return Task ID    │                 │               │              │
    │<──────────────────┤                 │               │              │
    │                   │                 │               │              │
    │                   │                 │ XREADGROUP    │              │
    │                   │                 │<──────────────┤              │
    │                   │                 │               │              │
    │                   │                 │               │ Load Model   │
    │                   │                 │               ├─────────────>│
    │                   │                 │               │              │
    │                   │                 │               │ Run Inference│
    │                   │                 │               │ (spawn_blocking)
    │                   │                 │               │              │
    │                   │                 │               │ Save Result  │
    │                   │                 │               ├─────────────>│
    │                   │                 │               │              │
    │                   │                 │ XACK          │              │
    │                   │                 │<──────────────┤              │
    │                   │                 │               │              │
    │ GET /inference/{id}                │               │              │
    ├──────────────────>│                 │               │              │
    │                   │ Get Result      │               │              │
    │                   ├────────────────>│               │              │
    │ Return Result     │                 │               │              │
    │<──────────────────┤                 │               │              │
```

### 4.3 模型管理流程

#### 模型验证步骤

模型上传/注册时执行以下验证：

```
验证流程：

1. 文件头检查
   - ONNX protobuf magic number (0x08 0x01 0x12 0x00...)
   - 快速失败，无需解析整个文件

2. ONNX Graph 反序列化
   - 验证格式正确性
   - 检查 protobuf 语法
   - 验证 graph 结构完整性

3. 提取模型元信息
   - input names, shapes, types
   - output names, shapes, types
   - opset version
   - model producer info

4. 名称冲突检查
   - 检查数据库中是否已存在相同 name + version

5. Session 创建验证（可选，较重）
   - 尝试创建 ONNX Session
   - 验证模型可推理
   - 检查执行提供者兼容性（CPU/GPU）

验证配置：
[model_validation]
validate_session = false  # 步骤5，默认关闭
async_validation = true   # 异步验证（不阻塞上传）
validation_timeout_secs = 30
```

#### 上传模型

```
CLI                    API Server              Database            Storage
 │                         │                       │                   │
 │ POST /models/upload     │                       │                   │
 │ (multipart/form-data)   │                       │                   │
 ├────────────────────────>│                       │                   │
 │                         │ [Sync] Validate       │                   │
 │                         │ Steps 1-4             │                   │
 │                         │                       │                   │
 │                         │ Check name conflict   │                   │
 │                         ├──────────────────────>│                   │
 │                         │<──────────────────────┤                   │
 │                         │                       │                   │
 │                         │ Save Model File       │                   │
 │                         ├──────────────────────────────────────────>│
 │                         │                       │                   │
 │                         │ Store Metadata        │                   │
 │                         │ (is_valid=true)       │                   │
 │                         ├──────────────────────>│                   │
 │                         │                       │                   │
 │ Return Model ID         │                       │                   │
 │<────────────────────────┤                       │                   │
 │                         │                       │                   │
 │                         │ [Async] Session验证   │                   │
 │                         │ (可选)                │                   │
 │                         │ Update is_valid       │                   │
 │                         ├──────────────────────>│                   │
```

#### 注册已有模型

```
CLI                    API Server              Database            Storage
 │                         │                       │                   │
 │ POST /models/register   │                       │                   │
 │ (path on server)        │                       │                   │
 ├────────────────────────>│                       │                   │
 │                         │ Verify File Exists    │                   │
 │                         ├──────────────────────────────────────────>│
 │                         │<──────────────────────────────────────────┤
 │                         │                       │                   │
 │                         │ [Sync] Validate       │                   │
 │                         │ Steps 1-4             │                   │
 │                         │                       │                   │
 │                         │ Check name conflict   │                   │
 │                         ├──────────────────────>│                   │
 │                         │<──────────────────────┤                   │
 │                         │                       │                   │
 │                         │ Store Metadata        │                   │
 │                         │ (is_valid=true)       │                   │
 │                         ├──────────────────────>│                   │
 │                         │                       │                   │
 │ Return Model ID         │                       │                   │
 │<────────────────────────┤                       │                   │
```

#### 验证失败处理

```
验证失败流程：

1. Steps 1-2 失败（格式错误）
   - 返回 400 Bad Request
   - 错误信息：INVALID_MODEL_FORMAT
   - 不保存文件和元数据

2. Step 4 失败（名称冲突）
   - 返回 409 Conflict
   - 错误信息：MODEL_ALREADY_EXISTS
   - 不保存文件和元数据

3. Step 5 失败（Session 创建失败）
   - 返回成功（Model ID）
   - 数据库记录 is_valid = false, validation_error = "..."
   - 可通过 GET /models/{id} 查看验证状态
   - 用户可以删除无效模型
```

## 5. API 设计

### 5.1 RESTful API 端点

#### 系统初始化（无认证）
```
POST   /api/v1/bootstrap                # 创建第一个管理员用户（仅当 users 表为空时可用）
```

**Bootstrap 端点说明**：
- 仅当数据库中 `users` 表为空时可调用
- 创建第一个 admin 用户并返回其 API Key
- 之后该端点自动禁用（返回 403 Forbidden）
- 无需任何认证
- 解决"先有鸡还是先有蛋"的 bootstrap 问题

请求/响应：
```json
// POST /api/v1/bootstrap
// Request
{
  "username": "admin",
  "password": "secure_password"
}

// Response
{
  "request_id": "req-xxx",
  "data": {
    "user_id": "user-uuid",
    "username": "admin",
    "role": "admin",
    "api_key": "frx_sk_...",
    "message": "Bootstrap completed. This endpoint is now disabled."
  }
}

// 后续调用返回
// Response (403)
{
  "request_id": "req-xxx",
  "error": {
    "code": "BOOTSTRAP_DISABLED",
    "message": "System already initialized. Bootstrap endpoint is disabled."
  }
}
```

#### 认证相关
```
POST   /api/v1/auth/login               # 用户名+密码登录 → 临时 API Key
POST   /api/v1/auth/logout              # 使当前临时 Key 失效
```

**Login 端点说明**：
- 通过用户名密码获取临时 API Key
- 临时 Key 固定短 TTL（默认 1 小时）
- 适用于 CLI 交互式场景

请求/响应：
```json
// POST /api/v1/auth/login
// Request
{
  "username": "admin",
  "password": "secure_password"
}

// Response
{
  "request_id": "req-xxx",
  "data": {
    "api_key": "frx_sk_temp_...",
    "expires_at": "2024-01-01T11:00:00Z",
    "user": {
      "id": "user-uuid",
      "username": "admin",
      "role": "admin"
    }
  }
}
```

#### 用户管理（管理员 API）
```
POST   /api/v1/admin/users              # 创建用户（管理员）
GET    /api/v1/admin/users              # 列出用户（管理员）
DELETE /api/v1/admin/users/{id}         # 删除用户（管理员）
PUT    /api/v1/admin/users/{id}         # 更新用户信息（管理员）
```

#### API Key 管理
```
POST   /api/v1/api-keys            # 创建新的 API Key
GET    /api/v1/api-keys            # 列出用户的所有 API Key
GET    /api/v1/api-keys/{id}       # 获取 API Key 详情
DELETE /api/v1/api-keys/{id}       # 撤销/删除 API Key
PUT    /api/v1/api-keys/{id}       # 更新 API Key 信息（如名称、权限）
```

#### 模型管理
```
POST   /api/v1/models/upload       # 上传模型文件 (multipart)
POST   /api/v1/models/register     # 注册服务器上已有的模型
GET    /api/v1/models              # 列出模型
GET    /api/v1/models/{id}         # 获取模型详情
DELETE /api/v1/models/{id}         # 删除模型
PUT    /api/v1/models/{id}         # 更新模型信息
```

#### 推理相关
```
POST   /api/v1/inference/sync      # 同步推理（立即返回结果）
POST   /api/v1/inference           # 异步推理（返回 task_id）
GET    /api/v1/inference/{id}      # 查询推理结果
DELETE /api/v1/inference/{id}      # 取消推理任务
GET    /api/v1/inference           # 列出推理任务
```

#### 系统相关
```
GET    /api/v1/health              # 健康检查
GET    /api/v1/ready               # 就绪检查（检查依赖）
GET    /api/v1/metrics             # Prometheus 指标
```

### 5.2 请求/响应格式

所有 API 使用 JSON 格式，API Key 通过请求头传递：

```
Authorization: Bearer frx_sk_a3b8f2e1d4c5a6b7e8f9d0c1b2a3e4f5
```

#### 同步推理请求

```json
// POST /api/v1/inference/sync
// Request
{
  "model_id": "model-123",
  "inputs": {
    "input.1": [[1.0, 2.0, 3.0]]
  }
}

// Response (成功)
{
  "request_id": "req-abc-123",
  "data": {
    "outputs": {
      "output.1": [[0.5, 0.3, 0.2]]
    },
    "latency_ms": 45
  }
}

// Response (错误)
{
  "request_id": "req-abc-124",
  "error": {
    "code": "MODEL_NOT_FOUND",
    "message": "Model with ID 'model-123' not found"
  }
}
```

#### 异步推理请求

```json
// POST /api/v1/inference
// Request
{
  "model_id": "model-123",
  "inputs": {
    "input.1": [[1.0, 2.0, 3.0]]
  },
  "options": {
    "priority": "high",  // high, normal, low
    "timeout": 300
  }
}

// Response
{
  "request_id": "req-abc-125",
  "data": {
    "task_id": "task-456",
    "status": "pending"
  }
}

// GET /api/v1/inference/task-456
// Response (完成)
{
  "request_id": "req-abc-127",
  "data": {
    "task_id": "task-456",
    "status": "completed",
    "outputs": {
      "output.1": [[0.5, 0.3, 0.2]]
    },
    "created_at": "2024-01-01T10:00:00Z",
    "completed_at": "2024-01-01T10:00:05Z",
    "latency_ms": 5000
  }
}
```

#### 错误码定义

| 错误码 | HTTP Status | 说明 |
|--------|-------------|------|
| `INVALID_API_KEY` | 401 | API Key 无效或已撤销 |
| `PERMISSION_DENIED` | 403 | 权限不足 |
| `MODEL_NOT_FOUND` | 404 | 模型不存在 |
| `TASK_NOT_FOUND` | 404 | 任务不存在 |
| `INVALID_INPUT` | 400 | 输入数据格式错误 |
| `INFERENCE_TIMEOUT` | 504 | 推理超时 |
| `INFERENCE_FAILED` | 500 | 推理执行失败 |
| `SERVICE_UNAVAILABLE` | 503 | 服务不可用（如 Redis 宕机时异步推理） |
| `RATE_LIMIT_EXCEEDED` | 429 | 请求频率超限 |

## 6. 配置文件设计

```toml
# config.toml

[server]
host = "0.0.0.0"
port = 8080
workers = 4
# 请求体大小限制 (用于模型上传)
max_request_size_mb = 500
# 优雅停机超时
graceful_shutdown_timeout = 30
# 同步推理并发限制
sync_inference_concurrency = 4
# 同步推理默认超时 (秒)
sync_inference_timeout = 30
# API 版本
api_version = "v1"
# 响应头包含 API 版本
include_version_header = true

[rate_limit]
enabled = true
algorithm = "sliding_window"  # token_bucket, sliding_window
default_rpm = 60              # 默认每分钟请求数
burst = 10                    # 突发允许量
sync_inference_rpm = 30       # 同步推理单独限制
async_inference_rpm = 100     # 异步推理单独限制
# 清理限流计数器的间隔
cleanup_interval_secs = 60

[auth]
# 支持环境变量覆盖: ${FERRINX_API_KEY_SECRET}
api_key_secret = "${FERRINX_API_KEY_SECRET}"
api_key_prefix = "frx_sk"
max_keys_per_user = 10
# 临时 Key 配置（登录生成）
temp_key_ttl_hours = 1
temp_key_prefix = "frx_sk_temp_"

[database]
backend = "postgresql"  # 或 "sqlite"
url = "${FERRINX_DATABASE_URL}"
# url = "sqlite://./data/ferrinx.db"
max_connections = 10
# 生产环境建议设为 false，使用 CLI 手动迁移
run_migrations = false

[redis]
url = "${FERRINX_REDIS_URL}"
pool_size = 10
# Redis Streams 配置
stream_key = "ferrinx:tasks:stream"
consumer_group = "ferrinx-workers"
# 死信队列
dead_letter_stream = "ferrinx:tasks:dead_letter"
# 结果缓存
result_cache_prefix = "ferrinx:results"
result_cache_ttl = 86400  # 推理结果缓存 24 小时（秒）
# API Key 缓存
api_key_store = "ferrinx:api_keys"
api_key_cache_ttl = 3600  # API Key 缓存 1 小时
# Redis 不可用时的降级策略
fallback_to_db = true

[storage]
# 存储后端: local, s3
backend = "local"
path = "./models"
# S3 配置（分布式部署）
# backend = "s3"
# bucket = "ferrinx-models"
# region = "us-east-1"
# endpoint = "https://s3.amazonaws.com"
# access_key = "${AWS_ACCESS_KEY_ID}"
# secret_key = "${AWS_SECRET_ACCESS_KEY}"

[onnx]
cache_size = 5
# 预加载模型
preload = ["model-id-1", "model-id-2"]
execution_provider = "CPU"  # CPU, CUDA, TensorRT
# GPU 设备 ID (CUDA/TensorRT)
gpu_device_id = 0

[logging]
level = "info"  # trace, debug, info, warn, error
format = "json"  # json, text
file = "./logs/ferrinx.log"
# 日志轮转
max_file_size_mb = 100
max_files = 10

[worker]
# Worker 配置 (仅 ferrinx-worker 使用)
consumer_name = ""  # 留空则自动生成 hostname-pid
concurrency = 4  # 并发处理任务数
poll_interval_ms = 100
# 重试策略
max_retries = 3
retry_delay_ms = 1000

[cleanup]
# 定期清理已完成的推理任务记录
enabled = true
# 已完成任务保留天数（0 表示不删除）
completed_task_retention_days = 30
# 已失败任务保留天数
failed_task_retention_days = 7
# 已取消任务保留天数
cancelled_task_retention_days = 3
# 清理任务执行间隔
cleanup_interval_hours = 24
# 每次清理的最大记录数
cleanup_batch_size = 1000
# 临时 API Key 清理间隔
temp_key_cleanup_interval_hours = 1

[model_validation]
# 模型上传时的验证配置
enabled = true
# 验证步骤（按顺序执行）
# 1. 文件头检查（ONNX protobuf magic number）
# 2. 反序列化 ONNX graph（验证格式正确）
# 3. 提取 input/output shapes 和 names
# 4. 检查是否与已有 name+version 冲突
# 5. 尝试创建 Session 验证可推理性（可选）
validate_session = false  # 创建 Session 验证（较重）
validation_timeout_secs = 30
# 异步后台验证（不阻塞上传响应）
async_validation = true
```

## 7. 数据库设计

### 7.1 表结构

#### users 表
```sql
CREATE TABLE users (
    id VARCHAR(36) PRIMARY KEY,
    username VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',  -- user, admin
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

#### api_keys 表
```sql
CREATE TABLE api_keys (
    id VARCHAR(36) PRIMARY KEY,
    user_id VARCHAR(36) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash VARCHAR(64) UNIQUE NOT NULL,  -- SHA-256 hash
    name VARCHAR(255) NOT NULL,
    permissions JSON NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    is_temporary BOOLEAN NOT NULL DEFAULT false,  -- 临时 Key（登录生成）
    last_used_at TIMESTAMP,
    expires_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_is_active ON api_keys(is_active);
CREATE INDEX idx_api_keys_is_temporary ON api_keys(is_temporary);
```

#### models 表
```sql
CREATE TABLE models (
    id VARCHAR(36) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    version VARCHAR(50) NOT NULL,
    file_path VARCHAR(500) NOT NULL,
    file_size BIGINT,
    storage_backend VARCHAR(50) NOT NULL DEFAULT 'local',  -- local, s3
    input_shapes JSON,
    output_shapes JSON,
    metadata JSON,
    is_valid BOOLEAN NOT NULL DEFAULT true,  -- 模型验证状态
    validation_error TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(name, version)
);

CREATE INDEX idx_models_name ON models(name);
CREATE INDEX idx_models_is_valid ON models(is_valid);
```

#### inference_tasks 表
```sql
CREATE TABLE inference_tasks (
    id VARCHAR(36) PRIMARY KEY,
    model_id VARCHAR(36) REFERENCES models(id),
    user_id VARCHAR(36) REFERENCES users(id),
    api_key_id VARCHAR(36) REFERENCES api_keys(id),
    status VARCHAR(50) NOT NULL,  -- pending, running, completed, failed, cancelled
    inputs JSON NOT NULL,
    outputs JSON,
    error_message TEXT,
    -- 优先级：数值越大优先级越高（1=low, 5=normal, 10=high）
    priority INTEGER DEFAULT 5 CHECK (priority >= 1 AND priority <= 10),
    retry_count INTEGER DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TIMESTAMP,
    completed_at TIMESTAMP
);

CREATE INDEX idx_inference_tasks_user_id ON inference_tasks(user_id);
CREATE INDEX idx_inference_tasks_status ON inference_tasks(status);
CREATE INDEX idx_inference_tasks_created_at ON inference_tasks(created_at);
CREATE INDEX idx_inference_tasks_priority ON inference_tasks(priority DESC);
```

### 7.2 数据库迁移

使用 `sqlx` 的 migration 功能：

```bash
# CLI 命令
ferrinx db migrate          # 执行待迁移
ferrinx db migrate --status # 查看迁移状态
ferrinx db rollback         # 回滚最近一次迁移
```

迁移文件：
```
crates/ferrinx-db/src/migrations/
├── 20240101_000001_create_users.sql
├── 20240101_000002_create_api_keys.sql
├── 20240101_000003_create_models.sql
└── 20240101_000004_create_inference_tasks.sql
```

## 8. Database Repository 与事务设计

### 8.1 Repositories 结构

```rust
// ferrinx-db/src/lib.rs

/// 数据库上下文，包含所有 Repository 和共享连接池
pub struct DbContext {
    pool: AnyPool,  // 共享连接池
    pub models: Box<dyn ModelRepository>,
    pub tasks: Box<dyn TaskRepository>,
    pub api_keys: Box<dyn ApiKeyRepository>,
    pub users: Box<dyn UserRepository>,
}

impl DbContext {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DbError> {
        let pool = match config.backend {
            DatabaseBackend::Postgresql => {
                AnyPool::connect(&config.url).await?
            }
            DatabaseBackend::Sqlite => {
                AnyPool::connect(&config.url).await?
            }
        };
        
        Ok(Self {
            models: Box::new(PostgresModelRepository::new(pool.clone())),
            tasks: Box::new(PostgresTaskRepository::new(pool.clone())),
            api_keys: Box::new(PostgresApiKeyRepository::new(pool.clone())),
            users: Box::new(PostgresUserRepository::new(pool.clone())),
            pool,
        })
    }
    
    /// 开启事务，用于跨 Repository 操作
    pub async fn begin(&self) -> Result<Transaction, DbError> {
        self.pool.begin().await.map_err(Into::into)
    }
}
```

### 8.2 Repository Trait 设计

#### 设计选择：泛型 Executor vs `_tx` 方法

**方案 A：泛型 Executor（推荐，sqlx 惯用模式）**

```rust
// ferrinx-db/src/traits.rs

use async_trait::async_trait;

/// 模型仓储
#[async_trait]
pub trait ModelRepository: Send + Sync {
    /// executor 可以是 Pool 也可以是 Transaction
    async fn save<'e, E>(&self, executor: E, model: &ModelInfo) -> Result<(), DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
    
    async fn find_by_id<'e, E>(&self, executor: E, id: &str) -> Result<Option<ModelInfo>, DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
    
    async fn find_by_name_version<'e, E>(&self, executor: E, name: &str, version: &str) -> Result<Option<ModelInfo>, DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
    
    async fn list<'e, E>(&self, executor: E, filter: &ModelFilter) -> Result<Vec<ModelInfo>, DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
    
    async fn delete<'e, E>(&self, executor: E, id: &str) -> Result<bool, DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
    
    async fn update<'e, E>(&self, executor: E, id: &str, updates: &ModelUpdates) -> Result<(), DbError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Any> + Send;
}

// 使用示例
// 无事务
repo.save(&pool, &model).await?;
// 有事务
repo.save(&mut *tx, &model).await?;
```

**优势**：
- 方法数量减半
- 与 sqlx 生态一致
- 灵活性高

**挑战**：
- 与 `async_trait` 结合时生命周期标注复杂
- 需要 `where E: 'e` 等约束

**方案 B：`_tx` 方法（备选，实现简单）**

如果泛型 executor 方案实现过于复杂，保持 `_tx` 方式：

```rust
#[async_trait]
pub trait ModelRepository: Send + Sync {
    async fn save(&self, model: &ModelInfo) -> Result<(), DbError>;
    async fn save_tx(&self, tx: &mut Transaction, model: &ModelInfo) -> Result<(), DbError>;
    // ...
}
```

**建议**：开发初期先用方案 B，熟悉后再考虑重构为方案 A。

#### 完整 Repository Trait 定义

```rust
/// 推理任务仓储
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn save(&self, task: &InferenceTask) -> Result<(), DbError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<InferenceTask>, DbError>;
    async fn update_status(&self, id: &str, status: TaskStatus) -> Result<(), DbError>;
    async fn set_result(&self, id: &str, result: &InferenceResult) -> Result<(), DbError>;
    async fn list(&self, filter: &TaskFilter) -> Result<Vec<InferenceTask>, DbError>;
    async fn delete(&self, id: &str) -> Result<bool, DbError>;
    // 事务方法
    async fn delete_by_user_tx(&self, tx: &mut Transaction, user_id: &str) -> Result<u64, DbError>;
    async fn delete_by_model_tx(&self, tx: &mut Transaction, model_id: &str) -> Result<u64, DbError>;
}

/// API Key 仓储
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn save(&self, key: &ApiKeyRecord) -> Result<(), DbError>;
    async fn find_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRecord>, DbError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<ApiKeyRecord>, DbError>;
    async fn find_by_user(&self, user_id: &str) -> Result<Vec<ApiKeyRecord>, DbError>;
    async fn find_temporary_by_user(&self, user_id: &str) -> Result<Vec<ApiKeyRecord>, DbError>;
    async fn update_last_used(&self, id: &str) -> Result<(), DbError>;
    async fn deactivate(&self, id: &str) -> Result<bool, DbError>;
    async fn delete_temporary(&self, id: &str) -> Result<bool, DbError>;
    async fn update_permissions(&self, id: &str, permissions: &Permissions) -> Result<(), DbError>;
    // 事务方法
    async fn delete_by_user_tx(&self, tx: &mut Transaction, user_id: &str) -> Result<u64, DbError>;
}

/// 用户仓储
#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn save(&self, user: &User) -> Result<(), DbError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, DbError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DbError>;
    async fn delete(&self, id: &str) -> Result<bool, DbError>;
    async fn list(&self) -> Result<Vec<User>, DbError>;
    async fn count(&self) -> Result<u64, DbError>;
}
```

### 8.3 PostgreSQL/SQLite 兼容性说明

#### 兼容性边界

| 特性 | PostgreSQL | SQLite | 兼容方案 |
|------|------------|--------|----------|
| JSON 类型 | JSONB | TEXT | 迁移脚本区分，代码中都用 JSON 函数 |
| RETURNING | 支持 | 3.35+ 支持 | 使用 RETURNING，SQLite 要求 3.35+ |
| 并发写入 | MVCC | 单写者 (WAL) | SQLite 仅用于开发/测试 |
| BOOL 类型 | 原生 | INTEGER | sqlx 自动处理 |
| TIMESTAMP | TIMESTAMPTZ | TEXT | 使用 chrono 序列化 |

#### 定位说明

```
PostgreSQL: 生产环境（推荐）
- 完整的 ACID 支持
- 高并发写入
- JSONB 索引支持
- 单节点足以支撑预期负载（见 21.2 节分析）

SQLite: 开发/测试环境
- 零配置，快速启动
- 单用户场景
- 嵌入式部署
- CI/CD 测试
```

**重要**：Ferrinx 的瓶颈在 ONNX 推理计算，数据库不是瓶颈。单节点 PostgreSQL 完全满足需求，无需主从复制或分片。仅在观测到数据库 CPU > 70% 或连接数接近上限时再考虑扩展。

```rust
if config.backend == DatabaseBackend::Sqlite && config.is_production {
    warn!("SQLite is not recommended for production use");
}
```

### 8.3 跨 Repository 事务示例

```rust
// 删除用户及其关联数据
async fn delete_user_with_cascade(db: &DbContext, user_id: &str) -> Result<(), Error> {
    let mut tx = db.begin().await?;
    
    // 删除用户的 API Keys
    db.api_keys.delete_by_user_tx(&mut tx, user_id).await?;
    
    // 删除用户的推理任务
    db.tasks.delete_by_user_tx(&mut tx, user_id).await?;
    
    // 删除用户
    db.users.delete_tx(&mut tx, user_id).await?;
    
    // 提交事务
    tx.commit().await?;
    
    Ok(())
}
```

## 9. Redis Streams 任务队列

### 9.1 队列设计

使用 **Redis Streams** 实现可靠的任务队列：

```
Stream Keys:
- ferrinx:tasks:high     # 高优先级
- ferrinx:tasks:normal   # 普通优先级
- ferrinx:tasks:low      # 低优先级
- ferrinx:tasks:dead_letter  # 死信队列

Consumer Group:
- ferrinx-workers

Consumer:
- worker-{hostname}-{pid}
```

### 9.2 任务消息格式

```
XADD ferrinx:tasks:normal * task_id "task-456" model_id "model-123" inputs '{"input.1":[[1.0]]}' priority 0 created_at "2024-01-01T10:00:00Z"
```

返回：
```
1704110400000-0  # Stream Entry ID
```

### 9.3 消费流程

```rust
// 伪代码
async fn consume_task(redis: &RedisClient, group: &str, consumer: &str) -> Option<Task> {
    // 按优先级顺序尝试消费
    for stream in ["high", "normal", "low"] {
        let key = format!("ferrinx:tasks:{}", stream);
        
        // 从消费组读取待处理消息
        let result = redis.xreadgroup(
            group, consumer,
            &[(&key, ">")],  // ">" 表示只读取新消息
            count: 1,
            block: 100,  // 阻塞 100ms
        ).await?;
        
        if let Some(entry) = result.first() {
            return Some(parse_task(entry));
        }
    }
    None
}

async fn ack_task(redis: &RedisClient, stream: &str, entry_id: &str) {
    redis.xack(
        format!("ferrinx:tasks:{}", stream),
        "ferrinx-workers",
        entry_id,
    ).await?;
}
```

### 9.4 失败重试与死信队列

```
流程：
1. 任务失败 → retry_count++
2. retry_count < max_retries → 重新放回队列（延迟）
3. retry_count >= max_retries → 移入死信队列

死信队列：
XADD ferrinx:tasks:dead_letter * task_id "task-456" error "OOM" retries 3
```

### 9.5 Worker 宕机恢复

```
场景：Worker 消费任务后宕机，未 XACK

恢复机制：
1. 新 Worker 使用 XAUTOCLAIM 或 XPENDING + XCLAIM
2. 检查 pending 列表中长时间未确认的消息
3. 超时阈值（如 5 分钟）未 XACK → 重新分配给新 Worker
```

## 10. 同步推理的并发控制

### 10.1 推理引擎设计

```rust
// ferrinx-core/src/inference/engine.rs

pub struct InferenceEngine {
    cache: Arc<RwLock<LruCache<String, Arc<Session>>>>,
    semaphore: Arc<Semaphore>,  // 并发限制
    timeout: Duration,
}

impl InferenceEngine {
    pub async fn infer(&self, model_id: &str, inputs: Inputs) -> Result<Outputs, Error> {
        // 1. 获取信号量（限制并发）
        let _permit = self.semaphore.acquire().await?;
        
        // 2. 获取模型 Session（从缓存或加载）
        let session = self.get_or_load_session(model_id).await?;
        
        // 3. spawn_blocking 执行 CPU 密集推理
        let timeout = self.timeout;
        let session_clone = session.clone();
        let inputs_clone = inputs.clone();
        
        let result = tokio::time::timeout(timeout, async {
            tokio::task::spawn_blocking(move || {
                session_clone.run(inputs_clone)
            }).await
        }).await??;
        
        Ok(result)
    }
    
    async fn get_or_load_session(&self, model_id: &str) -> Result<Arc<Session>, Error> {
        // 先读缓存
        {
            let cache = self.cache.read().await;
            if let Some(session) = cache.get(model_id) {
                return Ok(session.clone());
            }
        }
        
        // 加载模型
        let session = self.load_session(model_id).await?;
        
        // 写入缓存
        {
            let mut cache = self.cache.write().await;
            cache.put(model_id.to_string(), session.clone());
        }
        
        Ok(session)
    }
}
```

### 10.2 并发限制配置

```toml
[server]
# 同步推理并发限制
# 防止同时加载过多模型耗尽内存
sync_inference_concurrency = 4

# 同步推理默认超时 (秒)
sync_inference_timeout = 30
```

### 10.3 超时处理

```rust
// 推理超时返回特定错误
match engine.infer(&model_id, inputs).await {
    Ok(result) => Ok(result),
    Err(Error::Timeout) => {
        // 返回 504 Gateway Timeout
        Err(ApiError::InferenceTimeout)
    }
    Err(e) => Err(e.into()),
}
```

## 11. 模型存储抽象层

### 11.1 存储接口

```rust
// ferrinx-core/src/storage/mod.rs

#[async_trait]
pub trait ModelStorage: Send + Sync {
    /// 保存模型文件，返回存储路径
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError>;
    
    /// 加载模型文件
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError>;
    
    /// 删除模型文件
    async fn delete(&self, path: &str) -> Result<(), StorageError>;
    
    /// 检查文件是否存在
    async fn exists(&self, path: &str) -> Result<bool, StorageError>;
    
    /// 获取文件大小
    async fn size(&self, path: &str) -> Result<u64, StorageError>;
}
```

### 11.2 本地存储实现

```rust
// ferrinx-core/src/storage/local.rs

pub struct LocalStorage {
    base_path: PathBuf,
}

impl LocalStorage {
    pub fn new(base_path: &str) -> Self {
        Self {
            base_path: PathBuf::from(base_path),
        }
    }
}

#[async_trait]
impl ModelStorage for LocalStorage {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError> {
        let filename = format!("{}.onnx", model_id);
        let path = self.base_path.join(&filename);
        
        tokio::fs::write(&path, data).await?;
        
        Ok(path.to_string_lossy().to_string())
    }
    
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        Ok(tokio::fs::read(path).await?)
    }
    
    async fn delete(&self, path: &str) -> Result<(), StorageError> {
        tokio::fs::remove_file(path).await?;
        Ok(())
    }
    
    async fn exists(&self, path: &str) -> Result<bool, StorageError> {
        Ok(tokio::fs::metadata(path).await.is_ok())
    }
    
    async fn size(&self, path: &str) -> Result<u64, StorageError> {
        Ok(tokio::fs::metadata(path).await?.len())
    }
}
```

### 11.3 S3 存储实现（可选，通过 feature flag）

#### Feature Flag 配置

```toml
# ferrinx-core/Cargo.toml
[features]
default = ["local-storage"]
local-storage = []
s3-storage = ["aws-sdk-s3", "aws-config"]

[dependencies]
aws-sdk-s3 = { version = "1", optional = true }
aws-config = { version = "1", optional = true }
```

#### 实现代码

```rust
// ferrinx-core/src/storage/s3.rs
#[cfg(feature = "s3-storage")]

pub struct S3Storage {
    bucket: String,
    client: aws_sdk_s3::Client,
}

impl S3Storage {
    pub async fn new(config: &S3Config) -> Result<Self, StorageError> {
        let config = aws_config::load_from_env().await;
        let client = aws_sdk_s3::Client::new(&config);
        Ok(Self {
            bucket: config.bucket.clone(),
            client,
        })
    }
}

#[async_trait]
impl ModelStorage for S3Storage {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError> {
        let key = format!("models/{}.onnx", model_id);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(data.to_vec().into())
            .send()
            .await?;
        Ok(format!("s3://{}/{}", self.bucket, key))
    }
    
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let key = path.strip_prefix(&format!("s3://{}/", self.bucket)).unwrap();
        let output = self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(output.body.collect().await?.to_vec())
    }
    
    // ... 其他方法实现
}
```

#### 编译与部署

```bash
# 默认编译（仅本地存储）
cargo build --release

# 启用 S3 存储
cargo build --release --features s3-storage

# 同时支持（运行时通过配置选择）
cargo build --release --features "local-storage,s3-storage"
```

## 12. 用户管理与认证

### 12.1 系统初始化流程（Bootstrap）

**核心问题**：第一个用户从哪来？

**解决方案**：Bootstrap 端点（无认证，仅当 users 表为空时可用）

```
流程：
1. 首次部署时，执行数据库迁移
   ferrinx db migrate

2. 调用 Bootstrap 端点创建第一个管理员
   POST /api/v1/bootstrap
   {
     "username": "admin",
     "password": "secure_password"
   }
   
   返回：
   {
     "user_id": "user-uuid",
     "api_key": "frx_sk_..."  // 管理员 API Key
   }

3. 使用返回的 API Key 进行后续操作
   ferrinx config set api-key frx_sk_...

4. Bootstrap 端点自动禁用（后续调用返回 403）
```

**安全措施**：
- 仅当 `users` 表为空时可调用
- 数据库中记录 bootstrap 状态（或通过 user count 判断）
- 日志记录 bootstrap 操作
- 生产环境建议在初始化后关闭该端点（通过配置或网络策略）

### 12.2 用户创建流程（管理员）

**方式一：Bootstrap 端点（首次）**

见上文。

**方式二：CLI 命令**

```bash
# 管理员创建用户
ferrinx admin create-user --username user1 --password <password> --role user

# 列出用户
ferrinx admin list-users

# 删除用户
ferrinx admin delete-user <user-id>
```

**方式三：管理 API**

```
POST /api/v1/admin/users
Authorization: Bearer <admin-api-key>

{
  "username": "user1",
  "password": "secure_password",
  "role": "user"
}
```

### 12.3 用户登录流程

**场景**：CLI 交互式使用，需要临时 API Key

```bash
# 登录获取临时 API Key
ferrinx auth login --username admin --password <password>

# 返回临时 Key（有效期 1 小时）
# 自动保存到 ~/.ferrinx/api_key

# 登出（使临时 Key 失效）
ferrinx auth logout
```

**API 端点**：
```
POST /api/v1/auth/login
{
  "username": "admin",
  "password": "secure_password"
}

Response:
{
  "api_key": "frx_sk_temp_...",
  "expires_at": "2024-01-01T11:00:00Z",
  "user": { ... }
}
```

4. 创建用户 API Key
   ferrinx api-key create --name "my-key" --permissions '{"inference":["execute"]}'

5. 使用 API Key 进行推理
   ferrinx infer <model-id> --input '{"input.1":[[1.0]]}'
```

### 12.3 权限模型

```json
{
  "models": ["read", "write", "delete"],
  "inference": ["execute"],
  "api_keys": ["read", "write", "delete"],
  "admin": false
}
```

**角色默认权限**：
- `admin`: 所有权限
- `user`: `inference:execute`, `models:read`, `api_keys:read/write`

## 13. API Key 认证机制

### 13.1 API Key 格式

使用轻量级 opaque key 格式：

```
frx_sk_<random_32_bytes_hex>

示例: frx_sk_a3b8f2e1d4c5a6b7e8f9d0c1b2a3e4f5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0
```

### 13.2 验证流程（含降级）

```rust
async fn validate_api_key(key: &str, redis: &RedisClient, db: &DbContext) -> Result<ApiKeyInfo, Error> {
    let key_hash = sha256(key);
    
    // 尝试从 Redis 获取
    if let Ok(Some(info)) = redis.get_json(&format!("ferrinx:api_keys:{}", key_hash)).await {
        if info.is_active && !info.is_expired() {
            return Ok(info);
        }
    }
    
    // Redis 失败或未命中，降级到数据库
    warn!("Redis unavailable or cache miss, falling back to database");
    if let Some(info) = db.api_keys.find_by_hash(&key_hash).await? {
        if info.is_active && !info.is_expired() {
            // 异步更新缓存（如果 Redis 恢复）
            let redis_clone = redis.clone();
            let key_hash_clone = key_hash.clone();
            let info_clone = info.clone();
            tokio::spawn(async move {
                let _ = redis_clone.set_json(
                    &format!("ferrinx:api_keys:{}", key_hash_clone),
                    &info_clone,
                    Some(Duration::from_secs(3600)),
                ).await;
            });
            
            return Ok(info);
        }
    }
    
    Err(Error::InvalidApiKey)
}
```

## 14. CLI 设计

### 14.1 命令结构

```bash
# 管理员命令
ferrinx admin create-user --username <name> --password <pass> [--role <role>]
ferrinx admin list-users
ferrinx admin delete-user <user-id>
ferrinx admin login --username <name> --password <pass>
ferrinx admin logout

# 数据库管理
ferrinx db migrate [--status]
ferrinx db rollback

# API Key 管理
ferrinx api-key create --name <key-name> [--permissions <json>] [--expires <days>]
ferrinx api-key list
ferrinx api-key info <key-id>
ferrinx api-key revoke <key-id>
ferrinx api-key update <key-id> --name <new-name> --permissions <json>

# 配置
ferrinx config set api-key <api-key>
ferrinx config set api-url <url>
ferrinx config show

# 模型管理
ferrinx model list
ferrinx model upload <model-path> --name <name> --version <version>
ferrinx model register <server-path> --name <name> --version <version>
ferrinx model info <model-id>
ferrinx model delete <model-id>

# 推理（同步）
ferrinx infer <model-id> --input <input-file.json> --output <output-file.json>
ferrinx infer <model-id> --input '{"input.1": [[1.0, 2.0]]}'

# 推理（异步）
ferrinx infer <model-id> --input <input-file.json> --async
ferrinx task list
ferrinx task status <task-id>
ferrinx task cancel <task-id>

# 系统
ferrinx status
ferrinx version
```

### 14.2 配置文件 (CLI)

```toml
# ~/.ferrinx/config.toml
[api]
base_url = "http://localhost:8080/api/v1"
timeout = 30

[auth]
api_key_file = "~/.ferrinx/api_key"
admin_key_file = "~/.ferrinx/admin_key"

[output]
format = "table"  # table, json, toml
```

## 15. 优雅停机设计

### 15.1 API Server 优雅停机

```rust
pub async fn graceful_shutdown(
    signal: CancellationToken,
    server: Server,
    timeout: Duration,
) {
    signal.cancelled().await;
    
    info!("Received shutdown signal, stopping accepting new connections...");
    
    server.graceful_shutdown(Some(timeout));
    
    match tokio::time::timeout(timeout, async {
        server.await.ok();
    }).await {
        Ok(()) => info!("All connections closed gracefully"),
        Err(_) => warn!("Graceful shutdown timeout, forcing exit"),
    }
}
```

### 15.2 Worker 优雅停机

```rust
pub async fn graceful_shutdown(
    signal: CancellationToken,
    consumer: TaskConsumer,
    current_tasks: Arc<AtomicUsize>,
    timeout: Duration,
) {
    signal.cancelled().await;
    
    info!("Received shutdown signal, finishing current tasks...");
    
    consumer.stop().await;
    
    let start = std::time::Instant::now();
    while current_tasks.load(Ordering::Relaxed) > 0 && start.elapsed() < timeout {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    if current_tasks.load(Ordering::Relaxed) > 0 {
        warn!("Some tasks still running after timeout, exiting anyway");
    } else {
        info!("All tasks completed, shutting down");
    }
}
```

## 16. 技术栈

### 16.1 核心依赖

| 模块 | 依赖 | 用途 |
|------|------|------|
| Web 框架 | `axum` | RESTful API |
| ONNX Runtime | `ort` | 模型推理 |
| 数据库 | `sqlx` | PostgreSQL/SQLite |
| Redis | `redis` | Redis Streams、缓存 |
| 序列化 | `serde`, `serde_json` | JSON 序列化 |
| 配置 | `config`, `toml` | 配置文件 |
| CLI | `clap` | 命令行解析 |
| 日志 | `tracing`, `tracing-subscriber` | 日志记录 |
| 异步运行时 | `tokio` | 异步运行时 |
| 错误处理 | `thiserror`, `anyhow` | 错误定义 |
| 加密 | `sha2`, `bcrypt` | SHA-256、密码哈希 |
| UUID | `uuid` | UUID 生成 |
| 异步 Trait | `async-trait` | 异步 trait 支持 |
| 取消令牌 | `tokio-util` | CancellationToken |
| 指标 | `metrics`, `metrics-exporter-prometheus` | Prometheus 指标 |
| S3 (可选) | `aws-sdk-s3` | S3 存储 |

### 16.2 版本约束

```toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "sqlite", "chrono", "uuid"] }
redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }
ort = "2.0"
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "limit", "trace"] }
clap = { version = "4", features = ["derive"] }
async-trait = "0.1"
sha2 = "0.10"
bcrypt = "0.16"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
thiserror = "2"
anyhow = "1"
config = "0.14"
toml = "0.8"
metrics = "0.24"
metrics-exporter-prometheus = "0.16"
```

## 17. 安全设计

### 17.1 认证授权
- Opaque API Key（通过请求头传递）
- API Key Hash 存储（SHA-256）
- Redis 缓存 + 数据库降级验证
- 权限可修改，存储在数据库
- RBAC 权限模型
- API Key 使用记录和审计

### 17.2 数据安全
- 敏感配置支持环境变量
- 密码加密存储（bcrypt）
- HTTPS 传输
- SQL 注入防护（参数化查询）
- 输入验证

### 17.3 API 安全
- 请求限流（基于 API Key）
- 请求大小限制
- CORS 配置
- Request ID 追踪

## 18. 监控与日志

### 18.1 日志系统
- 结构化日志 (JSON)
- 日志级别控制
- 日志轮转
- Request ID 追踪

### 18.2 监控指标

```rust
// Prometheus 指标
- ferrinx_inference_requests_total{mode="sync|async", status="success|error"}
- ferrinx_inference_duration_seconds{mode="sync|async", model_id}
- ferrinx_inference_queue_length{priority="high|normal|low"}
- ferrinx_model_cache_hits_total
- ferrinx_model_cache_misses_total
- ferrinx_api_key_validations_total{source="redis|db"}
- ferrinx_redis_connections_active
- ferrinx_db_connections_active
- ferrinx_sync_inference_concurrent{limit, current}
```

### 18.3 健康检查

```
GET /api/v1/health
{
  "status": "healthy",
  "components": {
    "database": "healthy",
    "redis": "healthy"
  }
}

GET /api/v1/ready
{
  "ready": true,
  "components": {
    "database": true,
    "redis": true
  }
}
```

## 19. 部署方案

### 19.1 单机部署

```
┌─────────────────────────────────────┐
│           单机服务器                  │
│  ┌────────────────────────────────┐ │
│  │  ferrinx-api (同步推理)         │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  ferrinx-worker (异步推理)      │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  PostgreSQL                    │ │
│  └────────────────────────────────┘ │
│  ┌────────────────────────────────┐ │
│  │  Redis                         │ │
│  └────────────────────────────────┘ │
└─────────────────────────────────────┘
```

### 19.2 分布式部署

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
│ 任务无状态   │  │  任务无状态   │  │  任务无状态   │
│ (缓存有状态)│  │  (缓存有状态) │  │  (缓存有状态) │
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
              │      (单节点)          │
              └───────────┴───────────┘
                          │
              ┌───────────┴───────────┐
              │   S3 / NFS Storage    │
              │   (模型文件存储)       │
              └───────────────────────┘
```

### 19.3 路由策略（同步推理）

```nginx
# Nginx 一致性哈希配置示例
upstream ferrinx_api {
    hash $arg_model_id consistent;
    server api1:8080;
    server api2:8080;
    server api3:8080;
}
```

## 20. 测试策略

### 20.1 单元测试
- 核心业务逻辑测试
- Repository 测试（使用内存数据库）
- API Key 验证测试
- 模型缓存测试

### 20.2 集成测试
- API 端到端测试
- 同步/异步推理流程测试
- Redis 降级测试
- 优雅停机测试
- Redis Streams 消费测试

### 20.3 性能测试
- 同步推理延迟测试
- 异步推理吞吐量测试
- 并发压力测试
- 模型缓存效果测试
- spawn_blocking 性能验证

## 21. 扩展性

### 21.1 水平扩展

- **API Server**：有状态（同步推理），需一致性哈希路由
- **Worker**：任务分配层面无状态，运行时有模型缓存，可按需扩缩容
- **Redis Cluster**：支持高可用（任务队列 + 结果缓存）
- **PostgreSQL**：单节点部署（见下方说明）
- **Storage**：S3/NFS 共享存储

### 21.2 数据库容量分析

#### 实际负载特征

**写入压力**：
- 推理任务记录：中等频率（每次推理写一条）
- API Key `last_used_at`：中等频率（可批量延迟写入）
- 模型元数据：极低（偶尔上传）
- 用户/Key 管理：极低（管理操作）

**读取压力**：
- API Key 验证：**高频，但 Redis 是主路径，DB 只是降级**
- 模型信息：低频（可缓存）
- 任务状态查询：中等频率（异步推理轮询）

**核心事实**：
- Redis 承担了最高频的读请求（API Key 验证）
- 数据库实际压力很小
- 系统瓶颈在 **ONNX 推理计算**，不在数据库

#### 单节点 PostgreSQL 容量

| 指标 | 单节点能力 | Ferrinx 预期负载 |
|------|-----------|----------------|
| 写入 TPS | 5,000-10,000+ | ~80 TPS（假设 80 QPS 推理） |
| 读取 QPS | 10,000+ | < 100 QPS（Redis 缓存后） |
| 连接数 | 500+ | ~20-50（连接池配置） |

**结论**：单节点 PostgreSQL 完全满足需求，数据库不是瓶颈。

#### 未来扩展（按需）

当观测到以下指标时再考虑扩展：
- 数据库 CPU > 70% 持续
- 连接数接近上限
- 查询延迟明显增加

扩展方案：
- 读写分离（主从复制）
- 连接池调优
- 查询优化/索引
- 分表（按时间/用户）

**v1 建议**：单节点部署，监控指标，按需扩展。

### 21.2 功能扩展
- 支持更多推理引擎 (TensorFlow, PyTorch)
- 模型版本管理与回滚
- 模型压缩与优化
- GPU 加速支持
- 批处理推理
- 模型预热策略
- 多租户支持

## 22. API 版本迁移策略

### 22.1 版本管理原则

```
当前版本: v1 (路径: /api/v1/)

新版本发布时:
- v2 以 /api/v2/ 路径提供
- v1 和 v2 可同时运行
- v1 维护期：至少 6 个月（从 v2 发布起）
```

### 22.2 响应头约定

所有 API 响应包含版本信息：

```
HTTP/1.1 200 OK
X-API-Version: v1
X-API-Deprecated: false  # v1 接近废弃时设为 true
X-API-Sunset: 2025-06-01  # v1 废弃日期（ISO 8601）
```

### 22.3 版本废弃流程

```
时间线（以 v2 发布为例）：

T+0: v2 发布
  - v1 标记为 deprecated
  - 文档更新，推荐迁移到 v2
  - 响应头添加 X-API-Deprecated: true

T+3 个月: v1 功能冻结
  - v1 不再添加新功能
  - 仅修复 critical bug
  - 邮件通知用户迁移

T+6 个月: v1 废弃
  - v1 端点返回 410 Gone
  - 响应体包含迁移指南
  - 监控 v1 调用量，确认无活跃用户

T+7 个月: v1 下线
  - 移除 v1 代码
  - 更新文档
```

### 22.4 客户端迁移指南

```bash
# CLI 检查 API 版本
ferrinx version --api

# CLI 自动适配（根据服务器支持的版本）
# ~/.ferrinx/config.toml
[api]
base_url = "http://localhost:8080"
version = "auto"  # 或明确指定 v1, v2

# 废弃警告输出
$ ferrinx model list
Warning: API v1 is deprecated and will be removed on 2025-06-01.
Please upgrade to v2. See: https://docs.ferrinx.io/migration/v1-to-v2
```

### 22.5 不兼容变更处理

**破坏性变更类型**：

| 类型 | 示例 | v1→v2 处理 |
|------|------|-----------|
| 端点路径 | `/models` → `/models/list` | v1 保留旧路径，v2 新路径 |
| 请求字段 | `model_id` → `id` | v1 兼容两种字段 |
| 响应格式 | 嵌套结构变化 | v1 返回旧格式，v2 新格式 |
| 认证方式 | API Key → OAuth | v1 继续支持 API Key |

**兼容性保证**：

```
v1 客户端向 v2 服务端请求：
- 返回 400 Bad Request
- 错误信息：UNSUPPORTED_API_VERSION
- 包含支持的版本列表

v2 客户端向 v1 服务端请求：
- 返回 404 Not Found（/api/v2/ 不存在）
- 客户端应降级到 v1
```

## 23. 推荐实现顺序

```
Phase 1: 基础设施
1. ferrinx-common（配置、类型、常量）
2. ferrinx-db（迁移脚本 + PostgreSQL Repository）

Phase 2: 核心引擎
3. ferrinx-core
   - 推理引擎（spawn_blocking + Semaphore）
   - 模型缓存（LRU）
   - 本地存储

Phase 3: API 服务
4. ferrinx-api
   - Bootstrap 端点
   - Auth middleware（API Key 验证 + Redis/DB 降级）
   - 基础 routes（models, inference/sync）
   - 限流中间件

Phase 4: 异步推理
5. ferrinx-worker
   - Redis Streams 消费
   - 任务处理
   - 重试与死信队列

Phase 5: CLI 客户端
6. ferrinx-cli
   - HTTP client 封装
   - admin 命令（bootstrap）
   - model/infer 命令

Phase 6: 完善功能
7. 清理任务（过期任务、临时 Key）
8. 监控指标（Prometheus）
9. 文档与测试
```
