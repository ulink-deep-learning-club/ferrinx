# ferrinx-api 模块设计

## 1. 模块职责

`ferrinx-api` 提供 RESTful API 服务，职责包括：
- HTTP 路由和请求处理
- API Key 认证与授权
- 同步推理接口
- 异步推理任务提交
- 请求限流
- 优雅停机

**关键特性**：
- 基于 `axum` 的高性能 Web 框架
- 中间件架构（认证、限流、日志）
- 同步推理有状态（模型缓存）
- 异步推理通过 Redis Streams

## 2. 核心结构设计

### 2.1 应用状态

```rust
// src/main.rs

use axum::extract::Extension;
use std::sync::Arc;

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    /// 配置
    pub config: Arc<Config>,
    /// 数据库
    pub db: Arc<DbContext>,
    /// Redis 客户端
    pub redis: Option<Arc<RedisClient>>,
    /// 推理引擎
    pub engine: Arc<InferenceEngine>,
    /// 模型加载器
    pub loader: Arc<ModelLoader>,
    /// 存储
    pub storage: Arc<dyn ModelStorage>,
    /// 限流器
    pub rate_limiter: Arc<RateLimiter>,
    /// 取消令牌
    pub cancel_token: CancellationToken,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载配置
    let config = Config::from_file("config.toml")?;
    
    // 初始化日志
    init_logging(&config.logging)?;
    
    // 初始化数据库
    let db = DbContext::new(&config.database).await?;
    if config.database.run_migrations {
        db.run_migrations().await?;
    }
    
    // 初始化 Redis
    let redis = if config.redis.url.is_empty() {
        None
    } else {
        Some(Arc::new(RedisClient::new(&config.redis).await?))
    };
    
    // 初始化推理引擎
    let engine = Arc::new(InferenceEngine::new(&config.onnx)?);
    
    // 初始化存储（当前仅支持本地存储）
    let storage: Arc<dyn ModelStorage> = Arc::new(
        LocalStorage::new(config.storage.path.as_deref().unwrap_or("./models"))?
    );
    
    let loader = Arc::new(ModelLoader::new(storage.clone()));
    
    // 初始化限流器
    let rate_limiter = Arc::new(RateLimiter::new(&config.rate_limit)?);
    
    // 取消令牌
    let cancel_token = CancellationToken::new();
    
    // 应用状态
    let state = AppState {
        config: Arc::new(config.clone()),
        db: Arc::new(db),
        redis,
        engine,
        loader,
        storage,
        rate_limiter,
        cancel_token,
    };
    
    // 构建路由
    let app = create_router(state.clone());
    
    // 启动服务器
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    
    info!("Server listening on {}", addr);
    
    // 优雅停机
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.cancel_token))
        .await?;
    
    Ok(())
}
```

### 2.2 路由定义

```rust
// src/routes/mod.rs

use axum::{
    routing::{get, post, put, delete},
    Router,
};

pub fn create_router(state: AppState) -> Router {
    Router::new()
        // 系统
        .route("/api/v1/health", get(handlers::health))
        .route("/api/v1/ready", get(handlers::ready))
        .route("/api/v1/metrics", get(handlers::metrics))
        
        // Bootstrap（无认证）
        .route("/api/v1/bootstrap", post(handlers::bootstrap))
        
        // 认证
        .route("/api/v1/auth/login", post(handlers::auth::login))
        .route("/api/v1/auth/logout", post(handlers::auth::logout))
        
        // 用户管理（需要 admin 权限）
        .nest("/api/v1/admin", admin_routes())
        
        // API Key 管理
        .nest("/api/v1/api-keys", api_key_routes())
        
        // 模型管理
        .nest("/api/v1/models", model_routes())
        
        // 推理
        .route("/api/v1/inference/sync", post(handlers::inference::sync_infer))
        .route("/api/v1/inference/image", post(handlers::inference::image_infer))
        .route("/api/v1/inference", post(handlers::inference::async_infer))
        .route("/api/v1/inference/:id", get(handlers::inference::get_task))
        .route("/api/v1/inference/:id", delete(handlers::inference::cancel_task))
        .route("/api/v1/inference", get(handlers::inference::list_tasks))
        
        // 中间件（从外到内：logging → rate_limit → auth）
        .layer(middleware::from_fn(middleware::logging::logging_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), middleware::rate_limit::rate_limit_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), middleware::auth::auth_middleware))
        
        // 共享状态
        .with_state(state)
}

fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/users", post(handlers::admin::create_user))
        .route("/users", get(handlers::admin::list_users))
        .route("/users/:id", delete(handlers::admin::delete_user))
        .route("/users/:id", put(handlers::admin::update_user))
}

fn api_key_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(handlers::api_key::create))
        .route("/", get(handlers::api_key::list))
        .route("/:id", get(handlers::api_key::get))
        .route("/:id", delete(handlers::api_key::revoke))
        .route("/:id", put(handlers::api_key::update))
}

fn model_routes() -> Router<AppState> {
    Router::new()
        .route("/upload", post(handlers::model::upload))
        .route("/register", post(handlers::model::register))
        .route("/", get(handlers::model::list))
        .route("/:id", get(handlers::model::get))
        .route("/:id", delete(handlers::model::delete))
        .route("/:id", put(handlers::model::update))
        .route("/:name/:version", get(handlers::model::get_by_name_version))
        .route("/:name/:version", delete(handlers::model::delete_by_name_version))
}
```

### 2.3 请求处理器

```rust
// src/handlers/inference.rs

use axum::{
    extract::{Path, State, Extension},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

/// 同步推理请求
/// 
/// inputs 中的每个值必须是 Tensor 格式:
/// ```json
/// {
///   "dtype": "float32" | "int8" | "int64",
///   "shape": [...],
///   "data": "<base64-encoded-binary>"
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct SyncInferRequest {
    pub model_id: String,
    /// 输入张量，key 为输入层名称，value 为 Tensor 格式
    pub inputs: HashMap<String, serde_json::Value>,
}

/// 同步推理响应
/// 
/// outputs 中的每个值都是 Tensor 格式:
/// ```json
/// {
///   "dtype": "float32" | "int8" | "int64",
///   "shape": [...],
///   "data": "<base64-encoded-binary>"
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct SyncInferResponse {
    /// 输出张量，key 为输出层名称，value 为 Tensor 格式
    pub outputs: HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
}

/// 异步推理请求
#[derive(Debug, Deserialize)]
pub struct AsyncInferRequest {
    pub model_id: String,
    pub inputs: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub options: InferOptions,
}

#[derive(Debug, Deserialize, Default)]
pub struct InferOptions {
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
}

fn default_priority() -> String { "normal".to_string() }
fn default_timeout() -> u32 { 300 }

/// 异步推理响应
#[derive(Debug, Serialize)]
pub struct AsyncInferResponse {
    pub task_id: String,
    pub status: String,
}

/// 图像推理响应
#[derive(Debug, Serialize)]
pub struct ImageInferResponse {
    pub outputs: HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
    pub label: Option<String>,
    pub confidence: Option<f32>,
}

/// 同步推理
pub async fn sync_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Json(req): Json<SyncInferRequest>,
) -> Result<Json<ApiResponse<SyncInferResponse>>, ApiError> {
    // 验证权限
    if !api_key.permissions.inference.contains(&"execute".to_string()) {
        return Err(ApiError::PermissionDenied);
    }
    
    // 获取模型信息
    let model = state.db.models
        .find_by_id(&uuid::Uuid::parse_str(&req.model_id)?)
        .await?
        .ok_or(ApiError::ModelNotFound)?;
    
    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }
    
    // 执行推理
    let input = InferenceInput { inputs: req.inputs };
    let output = state.engine
        .infer(&req.model_id, &model.file_path, input)
        .await?;
    
    // 异步更新 API Key 最后使用时间
    let api_key_id = api_key.id.clone();
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = db.api_keys.update_last_used(&api_key_id).await;
    });
    
    // 异步记录审计日志（可选）
    
    Ok(Json(ApiResponse::success(SyncInferResponse {
        outputs: output.outputs,
        latency_ms: output.latency_ms,
    })))
}

/// 图像推理（带预处理）
pub async fn image_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<ImageInferResponse>>, ApiError> {
    // 验证权限
    if !api_key.permissions.inference.contains(&"execute".to_string()) {
        return Err(ApiError::PermissionDenied);
    }
    
    // 解析 multipart 表单
    let mut model_id: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut model_version: Option<String> = None;
    let mut image_data: Option<Vec<u8>> = None;
    
    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("model_id") => model_id = Some(field.text().await?),
            Some("name") => model_name = Some(field.text().await?),
            Some("version") => model_version = Some(field.text().await?),
            Some("image") => image_data = Some(field.bytes().await?.to_vec()),
            _ => {}
        }
    }
    
    let image_data = image_data.ok_or(ApiError::BadRequest("No image uploaded"))?;
    
    // 获取模型
    let model = if let Some(id) = model_id {
        state.db.models.find_by_id(&uuid::Uuid::parse_str(&id)?).await?
            .ok_or(ApiError::ModelNotFound)?
    } else if let (Some(name), Some(version)) = (model_name, model_version) {
        state.db.models.find_by_name_version(&name, &version).await?
            .ok_or(ApiError::ModelNotFound)?
    } else {
        return Err(ApiError::BadRequest("Either model_id or name+version required"));
    };
    
    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }
    
    // 解析模型配置
    let config: ModelConfig = model.metadata.as_ref()
        .ok_or(ApiError::ModelNotValid)?
        .try_into()?;
    
    // 预处理图像
    let input_tensor = config.preprocess_image(&image_data)?;
    
    // 执行推理
    let input = InferenceInput { 
        inputs: input_tensor 
    };
    let output = state.engine.infer(&model.id, &model.file_path, input).await?;
    
    // 后处理
    let result = config.postprocess(&output.outputs)?;
    
    Ok(Json(ApiResponse::success(ImageInferResponse {
        result,
        latency_ms: output.latency_ms,
    })))
}

/// 异步推理
pub async fn async_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Json(req): Json<AsyncInferRequest>,
) -> Result<Json<ApiResponse<AsyncInferResponse>>, ApiError> {
    // 验证权限
    if !api_key.permissions.inference.contains(&"execute".to_string()) {
        return Err(ApiError::PermissionDenied);
    }
    
    // 检查 Redis 可用性
    let redis = state.redis.as_ref()
        .ok_or(ApiError::RedisUnavailable)?;
    
    // 获取模型信息
    let model = state.db.models
        .find_by_id(&uuid::Uuid::parse_str(&req.model_id)?)
        .await?
        .ok_or(ApiError::ModelNotFound)?;
    
    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }
    
    // 创建任务
    let task_id = uuid::Uuid::new_v4();
    let task = InferenceTask {
        id: task_id,
        model_id: model.id,
        user_id: api_key.user_id,
        api_key_id: api_key.id,
        status: TaskStatus::Pending,
        inputs: serde_json::to_value(&req.inputs)?,
        outputs: None,
        error_message: None,
        priority: match req.options.priority.as_str() {
            "high" => 10,
            "low" => 1,
            _ => 5,
        },
        retry_count: 0,
        created_at: Utc::now(),
        started_at: None,
        completed_at: None,
    };
    
    // 保存任务到数据库
    state.db.tasks.save(&task).await?;
    
    // 推送到 Redis Streams
    redis.push_task(&task).await?;
    
    Ok(Json(ApiResponse::success(AsyncInferResponse {
        task_id: task_id.to_string(),
        status: "pending".to_string(),
    })))
}

/// 查询任务状态
pub async fn get_task(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Path(task_id): Path<String>,
) -> Result<Json<ApiResponse<TaskDetail>>, ApiError> {
    let task_id = uuid::Uuid::parse_str(&task_id)?;
    
    let task = state.db.tasks
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::TaskNotFound)?;
    
    // 验证权限
    if task.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }
    
    Ok(Json(ApiResponse::success(TaskDetail::from(task))))
}

/// 取消任务
pub async fn cancel_task(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Path(task_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let task_id = uuid::Uuid::parse_str(&task_id)?;
    
    let task = state.db.tasks
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::TaskNotFound)?;
    
    // 验证权限
    if task.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }
    
    // 只能取消 pending 状态的任务
    if task.status != TaskStatus::Pending {
        return Err(ApiError::TaskNotCancellable);
    }
    
    // 更新状态
    state.db.tasks
        .update_status(&task_id, TaskStatus::Cancelled)
        .await?;
    
    Ok(Json(ApiResponse::success(())))
}

/// 列出任务
pub async fn list_tasks(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Query(filter): Query<TaskFilterQuery>,
) -> Result<Json<ApiResponse<Vec<TaskDetail>>>, ApiError> {
    let filter = TaskFilter {
        user_id: Some(api_key.user_id),
        model_id: filter.model_id.and_then(|s| uuid::Uuid::parse_str(&s).ok()),
        status: filter.status.and_then(|s| TaskStatus::from_str(&s).ok()),
        limit: filter.limit,
        offset: filter.offset,
    };
    
    let tasks = state.db.tasks.list(&filter).await?;
    
    Ok(Json(ApiResponse::success(
        tasks.into_iter().map(TaskDetail::from).collect()
    )))
}

#[derive(Debug, Deserialize)]
pub struct TaskFilterQuery {
    pub model_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TaskDetail {
    pub task_id: String,
    pub model_id: String,
    pub status: String,
    pub outputs: Option<HashMap<String, serde_json::Value>>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub latency_ms: Option<u64>,
}

impl From<InferenceTask> for TaskDetail {
    fn from(task: InferenceTask) -> Self {
        let outputs = task.outputs.and_then(|v| serde_json::from_value(v).ok());
        let latency_ms = task.started_at.and_then(|start| {
            task.completed_at.map(|end| {
                (end - start).num_milliseconds() as u64
            })
        });
        
        Self {
            task_id: task.id.to_string(),
            model_id: task.model_id.to_string(),
            status: format!("{:?}", task.status).to_lowercase(),
            outputs,
            error_message: task.error_message,
            created_at: task.created_at.to_rfc3339(),
            completed_at: task.completed_at.map(|t| t.to_rfc3339()),
            latency_ms,
        }
    }
}
```

### 2.4 中间件

#### 认证中间件

```rust
// src/middleware/auth.rs

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // 某些路径不需要认证
    let path = req.uri().path();
    if is_public_path(path) {
        return Ok(next.run(req).await);
    }
    
    // 提取 Authorization header
    let auth_header = req.headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(ApiError::MissingApiKey)?;
    
    // 解析 API Key
    let api_key = auth_header
        .strip_prefix("Bearer ")
        .ok_or(ApiError::InvalidApiKeyFormat)?;
    
    // 验证 API Key
    let api_key_info = validate_api_key(api_key, &state).await?;
    
    // 检查权限
    if !check_permission(&api_key_info, path, req.method()) {
        return Err(ApiError::PermissionDenied);
    }
    
    // 注入 API Key 信息到请求扩展
    req.extensions_mut().insert(api_key_info);
    
    Ok(next.run(req).await)
}

async fn validate_api_key(
    key: &str,
    state: &AppState,
) -> Result<ApiKeyInfo, ApiError> {
    let key_hash = sha256_hash(key);
    
    // 尝试从 Redis 获取
    if let Some(ref redis) = state.redis {
        if let Ok(Some(info)) = redis.get_api_key(&key_hash).await {
            if info.is_active && !is_expired(&info) {
                return Ok(info);
            }
        }
    }
    
    // Redis 失败或未命中，降级到数据库
    warn!("Redis unavailable or cache miss, falling back to database");
    
    if let Some(record) = state.db.api_keys.find_by_hash(&key_hash).await? {
        let info = ApiKeyInfo::from(record);
        
        if !info.is_active || is_expired(&info) {
            return Err(ApiError::InvalidApiKey);
        }
        
        // 异步更新 Redis 缓存
        if let Some(ref redis) = state.redis {
            let redis_clone = redis.clone();
            let info_clone = info.clone();
            tokio::spawn(async move {
                let _ = redis_clone.set_api_key(&info_clone).await;
            });
        }
        
        return Ok(info);
    }
    
    Err(ApiError::InvalidApiKey)
}

fn is_public_path(path: &str) -> bool {
    matches!(path, 
        "/api/v1/health" | 
        "/api/v1/ready" |
        "/api/v1/bootstrap" |
        "/api/v1/auth/login"
        // 注意: /api/v1/metrics 需要认证，防止未授权访问监控数据
    )
}

fn check_permission(api_key: &ApiKeyInfo, path: &str, method: &Method) -> bool {
    // Admin 拥有所有权限
    if api_key.permissions.admin {
        return true;
    }
    
    // 根据路径和方法检查权限
    if path.starts_with("/api/v1/admin") {
        return false;
    }
    
    if path.starts_with("/api/v1/models") && method == Method::DELETE {
        return api_key.permissions.models.contains(&"delete".to_string());
    }
    
    if path.starts_with("/api/v1/inference") {
        return api_key.permissions.inference.contains(&"execute".to_string());
    }
    
    true
}

fn is_expired(info: &ApiKeyInfo) -> bool {
    info.expires_at.map_or(false, |exp| Utc::now() > exp)
}
```

#### 限流中间件

```rust
// src/middleware/rate_limit.rs

use axum::{
    body::Body,
    extract::{State, ConnectInfo},
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// 限流器结构 - 使用 DashMap 实现无锁并发
pub struct RateLimiter {
    /// 使用 DashMap 替代 HashMap + RwLock，提供锁-free 的并发访问
    limits: Arc<DashMap<String, RateLimitEntry>>,
    /// 窗口大小（秒）
    window_secs: u64,
}

/// 限流条目
pub struct RateLimitEntry {
    /// 使用 AtomicU64 进行原子计数，避免锁竞争
    count: AtomicU64,
    /// 窗口开始时间
    window_start: std::time::Instant,
}

impl RateLimiter {
    pub fn new(window_secs: u64) -> Self {
        Self {
            limits: Arc::new(DashMap::new()),
            window_secs,
        }
    }
    
    /// 检查是否允许请求 - 使用原子操作保证线程安全
    pub fn check(&self, key: &str, limit: u32) -> bool {
        let now = std::time::Instant::now();
        
        // 获取或创建条目
        let entry = self.limits.entry(key.to_string()).or_insert_with(|| {
            RateLimitEntry {
                count: AtomicU64::new(0),
                window_start: now,
            }
        });
        
        // 检查是否需要重置窗口
        if now.duration_since(entry.window_start).as_secs() >= self.window_secs {
            // 使用 SeqCst 内存顺序确保操作顺序一致性
            entry.count.store(1, Ordering::SeqCst);
            entry.window_start = now;
            return true;
        }
        
        // 原子增加计数器 - 单操作完成检查和增加
        // 使用 SeqCst 顺序保证跨线程可见性
        let new_count = entry.count.fetch_add(1, Ordering::SeqCst) + 1;
        
        if new_count > limit as u64 {
            // 超过限制，回滚计数器
            entry.count.fetch_sub(1, Ordering::SeqCst);
            false
        } else {
            true
        }
    }
}

/// 限流中间件
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if !state.config.rate_limit.enabled {
        return Ok(next.run(req).await);
    }
    
    // 从 ConnectInfo 获取客户端 IP（替代 X-Real-IP 头）
    let ip = req.extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    
    // 组合 API Key ID 和 IP 作为限流键
    let key = format!("{}:{}", api_key.id, ip);
    let limit = get_rate_limit(&req.uri().path(), &state.config.rate_limit);
    
    // 检查限流 - 使用 DashMap 实现锁-free 并发
    let allowed = state.rate_limiter.check(&key, limit);
    
    if !allowed {
        return Err(ApiError::RateLimitExceeded);
    }
    
    Ok(next.run(req).await)
}

fn get_rate_limit(path: &str, config: &RateLimitConfig) -> u32 {
    if path.starts_with("/api/v1/inference/sync") {
        config.sync_inference_rpm
    } else if path.starts_with("/api/v1/inference") {
        config.async_inference_rpm
    } else {
        config.default_rpm
    }
}
```

**限流器改进说明**：

1. **DashMap 替代 HashMap + RwLock**：
   - 提供锁-free 的并发访问
   - 更好的并发性能，避免读写锁竞争
   - 内置分片机制减少冲突

2. **ConnectInfo 替代 X-Real-IP**：
   - 使用 axum 内置的 `ConnectInfo` 扩展获取客户端 IP
   - 更可靠，不依赖反向代理设置的特殊 header
   - 防止 IP  spoofing 攻击

3. **原子操作使用 SeqCst**：
   - `fetch_add` 和 `fetch_sub` 使用 `Ordering::SeqCst`
   - 确保操作顺序一致性，防止竞态条件
   - 单原子操作完成检查和增加，避免 TOCTOU 问题

4. **组合限流键**：
   - 使用 `api_key_id:ip` 作为限流键
   - 防止单个用户多 IP 绕过限流
   - 同时防止单个 IP 使用多个 API Key

### 2.5 统一响应格式

```rust
// src/dto/mod.rs

use serde::{Deserialize, Serialize};

/// 统一 API 响应格式
#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiErrorBody>,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            request_id: generate_request_id(),
            data: Some(data),
            error: None,
        }
    }
    
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            request_id: generate_request_id(),
            data: None,
            error: Some(ApiErrorBody {
                code: code.as_str().to_string(),
                message: message.into(),
            }),
        }
    }
}

fn generate_request_id() -> String {
    format!("req-{}", uuid::Uuid::new_v4())
}
```

### 2.6 模型详情 DTO

```rust
// src/dto/mod.rs

/// 模型详情响应
#[derive(Debug, Serialize)]
pub struct ModelDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    /// 输入层信息（包含层名、形状、数据类型）
    pub input_shapes: Option<serde_json::Value>,
    /// 输出层信息（包含层名、形状、数据类型）
    pub output_shapes: Option<serde_json::Value>,
    /// 模型配置（预处理/后处理管道）
    pub metadata: Option<serde_json::Value>,
    /// 模型是否有效（computed: metadata && input_shapes）
    pub is_valid: bool,
    /// 验证错误信息（computed）
    pub validation_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ferrinx_common::ModelInfo> for ModelDetail {
    fn from(model: ferrinx_common::ModelInfo) -> Self {
        let is_valid = model.is_valid();
        let validation_error = model.validation_error();
        Self {
            id: model.id.to_string(),
            name: model.name,
            version: model.version,
            file_path: model.file_path,
            file_size: model.file_size,
            input_shapes: model.input_shapes,
            output_shapes: model.output_shapes,
            metadata: model.metadata,
            is_valid,
            validation_error,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}
```

**is_valid 计算规则**：
- `is_valid = metadata.is_some() && input_shapes.is_some()`
- 模型需要有配置文件（metadata）和验证通过的输入形状才能用于推理

**validation_error 计算规则**：
- 如果 `input_shapes.is_none()` → "Model failed validation"
- 如果 `metadata.is_none()` → "Missing preprocessing config"
- 否则 → `None`

**input_shapes / output_shapes 格式**：

```json
{
  "input_shapes": [
    {
      "name": "input.1",
      "shape": [-1, 1, 28, 28],
      "element_type": "float32"
    }
  ],
  "output_shapes": [
    {
      "name": "output.1", 
      "shape": [-1, 10],
      "element_type": "float32"
    }
  ]
}
```

**使用场景**：

1. **单输入模型**：用户可使用任意 key 名称（如 `"input"`），系统自动匹配
2. **多输入模型**：用户需先查询 `GET /api/v1/models/{id}` 获取 `input_shapes`，然后使用精确的层名

**示例**：

```bash
# 查询模型详情获取输入层名
curl -H "Authorization: Bearer $API_KEY" \
  http://localhost:8080/api/v1/models/model-123

# 响应
{
  "request_id": "req-xxx",
  "data": {
    "id": "model-123",
    "name": "lenet-mnist",
    "version": "1.0",
    "input_shapes": [
      {"name": "input.1", "shape": [-1, 1, 28, 28], "element_type": "float32"}
    ],
    "output_shapes": [
      {"name": "output.1", "shape": [-1, 10], "element_type": "float32"}
    ],
    ...
  }
}

# 单输入模型 - 任意 key 名称可用
# 输入必须是 Tensor 格式: {dtype, shape, data}
curl -X POST -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "model-123",
    "inputs": {
      "input": {
        "dtype": "float32",
        "shape": [1, 1, 28, 28],
        "data": "<base64-encoded-data>"
      }
    }
  }' \
  http://localhost:8080/api/v1/inference/sync

# 多输入模型 - 必须使用精确层名
curl -X POST -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "multi-input-model",
    "inputs": {
      "image": {
        "dtype": "float32",
        "shape": [1, 3, 224, 224],
        "data": "<base64-encoded-data>"
      },
      "mask": {
        "dtype": "float32",
        "shape": [1, 1, 224, 224],
        "data": "<base64-encoded-data>"
      }
    }
  }' \
  http://localhost:8080/api/v1/inference/sync
```

#### 2.5.1 Tensor 数据格式

所有推理输入/输出都使用统一的 **Tensor** 格式：

```json
{
  "dtype": "float32",  // 数据类型: float32 | int8 | int64
  "shape": [1, 3, 224, 224],  // 张量形状
  "data": "AAABAAIAAw..."  // base64 编码的二进制数据
}
```

**字段说明**:
| 字段 | 类型 | 说明 |
|------|------|------|
| `dtype` | string | 数据类型，支持 `float32`, `int8`, `int64` |
| `shape` | array | 张量形状，如 `[1, 3, 224, 224]` 表示 batch=1, channel=3, height=224, width=224 |
| `data` | string | base64 编码的二进制数据 |

**重要约束**:
1. **显式形状**: Tensor 的 shape 必须与模型期望的输入 shape 完全匹配
2. **类型匹配**: dtype 必须与模型输入类型一致
3. **数据大小**: 数据长度必须等于 shape 各维度的乘积 × 元素大小

**Python 客户端示例**:
```python
import base64
import numpy as np
import requests

# 创建 numpy 数组
input_array = np.random.randn(1, 3, 224, 224).astype(np.float32)

# 转换为 Tensor 格式
def to_tensor(arr):
    return {
        "dtype": "float32" if arr.dtype == np.float32 else "int64",
        "shape": list(arr.shape),
        "data": base64.b64encode(arr.tobytes()).decode('utf-8')
    }

tensor = to_tensor(input_array)

# 发送推理请求
response = requests.post(
    "http://localhost:8080/api/v1/inference/sync",
    headers={"Authorization": f"Bearer {api_key}"},
    json={
        "model_id": "resnet50",
        "inputs": {"input": tensor}
    }
)

# 解析响应
result = response.json()
output_tensor = result["data"]["outputs"]["output"]
output_array = np.frombuffer(
    base64.b64decode(output_tensor["data"]),
    dtype=np.float32
).reshape(output_tensor["shape"])
```

**JavaScript/TypeScript 客户端示例**:
```typescript
import { Buffer } from 'buffer';

// 创建 Float32Array
const data = new Float32Array(1 * 3 * 224 * 224);
// ... fill data ...

// 转换为 Tensor 格式
const tensor = {
  dtype: "float32",
  shape: [1, 3, 224, 224],
  data: Buffer.from(data.buffer).toString('base64')
};

// 发送请求
const response = await fetch('http://localhost:8080/api/v1/inference/sync', {
  method: 'POST',
  headers: {
    'Authorization': `Bearer ${apiKey}`,
    'Content-Type': 'application/json'
  },
  body: JSON.stringify({
    model_id: 'resnet50',
    inputs: { input: tensor }
  })
});

const result = await response.json();
// 解码输出
const outputData = Buffer.from(result.data.outputs.output.data, 'base64');
const outputArray = new Float32Array(outputData.buffer);
```

### 2.6 错误处理

```rust
// src/error.rs

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Invalid API key")]
    InvalidApiKey,
    
    #[error("Missing API key")]
    MissingApiKey,
    
    #[error("Invalid API key format")]
    InvalidApiKeyFormat,
    
    #[error("Permission denied")]
    PermissionDenied,
    
    #[error("Model not found")]
    ModelNotFound,
    
    #[error("Model not valid")]
    ModelNotValid,
    
    #[error("Task not found")]
    TaskNotFound,
    
    #[error("Task not cancellable")]
    TaskNotCancellable,
    
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    #[error("Redis unavailable")]
    RedisUnavailable,
    
    #[error("Database error: {0}")]
    DatabaseError(#[from] DbError),
    
    #[error("Core error: {0}")]
    CoreError(#[from] CoreError),
    
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Internal server error")]
    InternalError,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::InvalidApiKey | ApiError::MissingApiKey | ApiError::InvalidApiKeyFormat => {
                (StatusCode::UNAUTHORIZED, ErrorCode::InvalidApiKey)
            }
            ApiError::PermissionDenied => {
                (StatusCode::FORBIDDEN, ErrorCode::PermissionDenied)
            }
            ApiError::ModelNotFound => {
                (StatusCode::NOT_FOUND, ErrorCode::ModelNotFound)
            }
            ApiError::TaskNotFound => {
                (StatusCode::NOT_FOUND, ErrorCode::TaskNotFound)
            }
            ApiError::ModelNotValid | ApiError::TaskNotCancellable | ApiError::BadRequest(_) => {
                (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput)
            }
            ApiError::RateLimitExceeded => {
                (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimitExceeded)
            }
            ApiError::RedisUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::ServiceUnavailable)
            }
            ApiError::DatabaseError(_) | ApiError::CoreError(_) | ApiError::InternalError => {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
            }
        };
        
        let body = ApiResponse::<()>::error(code, self.to_string());
        
        (status, Json(body)).into_response()
    }
}
```

### 2.7 优雅停机

```rust
// src/shutdown.rs

use tokio::signal;
use tokio_util::sync::CancellationToken;

pub async fn shutdown_signal(cancel_token: CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = cancel_token.cancelled() => {},
    }

    info!("Shutdown signal received");
}
```

## 3. 依赖关系

```toml
# Cargo.toml

[package]
name = "ferrinx-api"
version = "0.1.0"
edition = "2021"

[dependencies]
ferrinx-common = { path = "../ferrinx-common" }
ferrinx-db = { path = "../ferrinx-db" }
ferrinx-core = { path = "../ferrinx-core" }

axum = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }

redis = { workspace = true, optional = true }

[features]
default = ["redis"]
redis = ["dep:redis"]
s3-storage = ["ferrinx-core/s3-storage"]
```

## 4. 测试策略

### 4.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::test::TestClient;
    
    #[tokio::test]
    async fn test_health_endpoint() {
        let app = create_test_app().await;
        let client = TestClient::new(app);
        
        let response = client.get("/api/v1/health").send().await;
        
        assert_eq!(response.status(), StatusCode::OK);
    }
    
    #[tokio::test]
    async fn test_sync_inference_unauthorized() {
        let app = create_test_app().await;
        let client = TestClient::new(app);
        
        let response = client
            .post("/api/v1/inference/sync")
            .json(&json!({
                "model_id": "test-model",
                "inputs": {}
            }))
            .send()
            .await;
        
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
```

### 4.2 集成测试

```rust
#[tokio::test]
#[ignore]
async fn test_full_inference_flow() {
    // 启动测试服务器
    // 创建测试用户和 API Key
    // 上传测试模型
    // 执行推理
    // 验证结果
}
```

## 5. 设计要点

### 5.1 状态管理

- AppState 包含所有共享状态
- 使用 Arc 共享引用
- Clone trait 便于中间件使用

### 5.2 中间件顺序

中间件执行顺序（从外到内）：
1. **日志中间件**（最外层）- 记录所有请求，包括未认证请求
2. **限流中间件** - 在认证前限流，避免认证开销
3. **认证中间件** - 验证 API Key 和权限
4. **路由处理**

注意：axum 的 layer 从底部向上添加，所以代码顺序与执行顺序相反。

### 5.3 错误处理

- 统一的 ApiError 类型
- 实现 IntoResponse trait
- HTTP 状态码自动映射

### 5.4 性能优化

- 异步更新最后使用时间
- 异步记录审计日志
- Redis 缓存验证结果
