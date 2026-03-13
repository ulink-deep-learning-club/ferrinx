# ferrinx-db 模块设计

## 1. 模块职责

`ferrinx-db` 提供数据库抽象层，职责包括：
- 定义 Repository trait（按领域拆分）
- 实现 PostgreSQL 和 SQLite 两种后端
- 管理数据库连接池
- 提供事务支持
- 执行数据库迁移

**关键特性**：
- 业务代码只依赖 trait，不依赖具体实现
- 支持 PostgreSQL（生产）和 SQLite（开发/测试）
- 事务支持跨 Repository 操作

## 2. 核心结构设计

### 2.1 数据库上下文

```rust
// src/lib.rs

use sqlx::any::{AnyPool, AnyConnection};
use std::sync::Arc;

pub struct DbContext {
    pool: AnyPool,
    pub models: Arc<dyn ModelRepository>,
    pub tasks: Arc<dyn TaskRepository>,
    pub api_keys: Arc<dyn ApiKeyRepository>,
    pub users: Arc<dyn UserRepository>,
    backend: DatabaseBackend,
}

impl DbContext {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DbError> {
        let pool = AnyPool::connect(&config.url).await?;
        let backend = config.backend.clone();
        
        // SQLite 外键约束启用
        if backend == DatabaseBackend::Sqlite {
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&pool)
                .await?;
        }
        
        let (models, tasks, api_keys, users) = match backend {
            DatabaseBackend::Postgresql => {
                let pool_clone = pool.clone();
                (
                    Arc::new(PostgresModelRepository::new(pool_clone)) as Arc<dyn ModelRepository>,
                    Arc::new(PostgresTaskRepository::new(pool.clone())) as Arc<dyn TaskRepository>,
                    Arc::new(PostgresApiKeyRepository::new(pool.clone())) as Arc<dyn ApiKeyRepository>,
                    Arc::new(PostgresUserRepository::new(pool.clone())) as Arc<dyn UserRepository>,
                )
            }
            DatabaseBackend::Sqlite => {
                let pool_clone = pool.clone();
                (
                    Arc::new(SqliteModelRepository::new(pool_clone)) as Arc<dyn ModelRepository>,
                    Arc::new(SqliteTaskRepository::new(pool.clone())) as Arc<dyn TaskRepository>,
                    Arc::new(SqliteApiKeyRepository::new(pool.clone())) as Arc<dyn ApiKeyRepository>,
                    Arc::new(SqliteUserRepository::new(pool.clone())) as Arc<dyn UserRepository>,
                )
            }
        };
        
        Ok(Self {
            pool,
            models,
            tasks,
            api_keys,
            users,
            backend,
        })
    }
            DatabaseBackend::Sqlite => {
                let pool_clone = pool.clone();
                (
                    Arc::new(SqliteModelRepository::new(pool_clone)) as Arc<dyn ModelRepository>,
                    Arc::new(SqliteTaskRepository::new(pool.clone())) as Arc<dyn TaskRepository>,
                    Arc::new(SqliteApiKeyRepository::new(pool.clone())) as Arc<dyn ApiKeyRepository>,
                    Arc::new(SqliteUserRepository::new(pool.clone())) as Arc<dyn UserRepository>,
                )
            }
        };
        
        Ok(Self {
            pool,
            models,
            tasks,
            api_keys,
            users,
            backend,
        })
    }
    
    /// 开启事务
    pub async fn begin(&self) -> Result<Transaction, DbError> {
        let tx = self.pool.begin().await?;
        Ok(Transaction {
            inner: tx,
            backend: self.backend.clone(),
        })
    }
    
    /// 执行迁移
    pub async fn run_migrations(&self) -> Result<(), DbError> {
        match self.backend {
            DatabaseBackend::Postgresql => {
                sqlx::migrate!("migrations/postgres")
                    .run(&self.pool)
                    .await?;
            }
            DatabaseBackend::Sqlite => {
                sqlx::migrate!("migrations/sqlite")
                    .run(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }
    
    /// 健康检查
    pub async fn health_check(&self) -> Result<(), DbError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}

/// 事务包装器
pub struct Transaction {
    inner: sqlx::Transaction<'static, sqlx::Any>,
    backend: DatabaseBackend,
}

impl Transaction {
    pub async fn commit(self) -> Result<(), DbError> {
        self.inner.commit().await?;
        Ok(())
    }
    
    pub async fn rollback(self) -> Result<(), DbError> {
        self.inner.rollback().await?;
        Ok(())
    }
}
```

### 2.2 Repository Trait 定义

#### 设计方案：方案 B（`_tx` 方法）

考虑到 `async_trait` 与泛型 Executor 结合时的复杂性，采用方案 B：为事务操作提供单独的 `_tx` 方法。

```rust
// src/traits.rs

use async_trait::async_trait;
use ferrinx_common::*;
use super::Transaction;

/// 模型仓储
#[async_trait]
pub trait ModelRepository: Send + Sync {
    /// 保存模型
    async fn save(&self, model: &ModelInfo) -> Result<(), DbError>;
    
    /// 在事务中保存模型
    async fn save_tx(&self, tx: &mut Transaction, model: &ModelInfo) -> Result<(), DbError>;
    
    /// 根据 ID 查找模型
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ModelInfo>, DbError>;
    
    /// 根据名称和版本查找模型
    async fn find_by_name_version(&self, name: &str, version: &str) -> Result<Option<ModelInfo>, DbError>;
    
    /// 列出模型
    async fn list(&self, filter: &ModelFilter) -> Result<Vec<ModelInfo>, DbError>;
    
    /// 删除模型
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 在事务中删除模型
    async fn delete_tx(&self, tx: &mut Transaction, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 检查模型是否存在
    async fn exists(&self, name: &str, version: &str) -> Result<bool, DbError>;
}

/// 模型过滤条件
#[derive(Debug, Clone, Default)]
pub struct ModelFilter {
    pub name: Option<String>,
    pub is_valid: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 推理任务仓储
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// 保存任务
    async fn save(&self, task: &InferenceTask) -> Result<(), DbError>;
    
    /// 根据 ID 查找任务
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<InferenceTask>, DbError>;
    
    /// 更新任务状态
    async fn update_status(&self, id: &uuid::Uuid, status: TaskStatus) -> Result<(), DbError>;
    
    /// 设置任务结果
    async fn set_result(
        &self, 
        id: &uuid::Uuid, 
        status: TaskStatus, 
        outputs: Option<&serde_json::Value>,
        error: Option<&str>
    ) -> Result<(), DbError>;
    
    /// 列出任务
    async fn list(&self, filter: &TaskFilter) -> Result<Vec<InferenceTask>, DbError>;
    
    /// 删除任务
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 在事务中删除用户的所有任务
    async fn delete_by_user_tx(&self, tx: &mut Transaction, user_id: &uuid::Uuid) -> Result<u64, DbError>;
    
    /// 在事务中删除模型的所有任务
    async fn delete_by_model_tx(&self, tx: &mut Transaction, model_id: &uuid::Uuid) -> Result<u64, DbError>;
    
    /// 清理过期任务
    async fn cleanup_expired(&self, retention_days: u32, batch_size: usize) -> Result<u64, DbError>;
}

/// 任务过滤条件
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub user_id: Option<uuid::Uuid>,
    pub model_id: Option<uuid::Uuid>,
    pub status: Option<TaskStatus>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// API Key 仓储
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// 保存 API Key
    async fn save(&self, key: &ApiKeyRecord) -> Result<(), DbError>;
    
    /// 根据哈希查找 API Key
    async fn find_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRecord>, DbError>;
    
    /// 根据 ID 查找 API Key
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ApiKeyRecord>, DbError>;
    
    /// 列出用户的所有 API Key
    async fn find_by_user(&self, user_id: &uuid::Uuid) -> Result<Vec<ApiKeyRecord>, DbError>;
    
    /// 列出用户的临时 API Key
    async fn find_temporary_by_user(&self, user_id: &uuid::Uuid) -> Result<Vec<ApiKeyRecord>, DbError>;
    
    /// 更新最后使用时间
    async fn update_last_used(&self, id: &uuid::Uuid) -> Result<(), DbError>;
    
    /// 停用 API Key
    async fn deactivate(&self, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 删除临时 API Key
    async fn delete_temporary(&self, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 更新权限
    async fn update_permissions(&self, id: &uuid::Uuid, permissions: &Permissions) -> Result<(), DbError>;
    
    /// 在事务中删除用户的所有 API Key
    async fn delete_by_user_tx(&self, tx: &mut Transaction, user_id: &uuid::Uuid) -> Result<u64, DbError>;
    
    /// 清理过期的临时 Key
    async fn cleanup_expired_temp_keys(&self) -> Result<u64, DbError>;
}

/// 用户仓储
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// 保存用户
    async fn save(&self, user: &User) -> Result<(), DbError>;
    
    /// 根据 ID 查找用户
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<User>, DbError>;
    
    /// 根据用户名查找用户
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DbError>;
    
    /// 删除用户
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 在事务中删除用户
    async fn delete_tx(&self, tx: &mut Transaction, id: &uuid::Uuid) -> Result<bool, DbError>;
    
    /// 列出用户
    async fn list(&self, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<User>, DbError>;
    
    /// 统计用户数量
    async fn count(&self) -> Result<u64, DbError>;
    
    /// 更新用户信息
    async fn update(&self, id: &uuid::Uuid, updates: &UserUpdates) -> Result<(), DbError>;
}

/// 用户更新字段
#[derive(Debug, Clone, Default)]
pub struct UserUpdates {
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<UserRole>,
    pub is_active: Option<bool>,
}
```

## 3. PostgreSQL 实现

### 3.1 模型仓储实现

```rust
// src/repositories/model.rs (PostgreSQL)

use sqlx::{Postgres, QueryBuilder};

pub struct PostgresModelRepository {
    pool: AnyPool,
}

impl PostgresModelRepository {
    pub fn new(pool: AnyPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ModelRepository for PostgresModelRepository {
    async fn save(&self, model: &ModelInfo) -> Result<(), DbError> {
        let query = r#"
            INSERT INTO models (
                id, name, version, file_path, file_size, storage_backend,
                input_shapes, output_shapes, metadata, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                version = EXCLUDED.version,
                input_shapes = EXCLUDED.input_shapes,
                output_shapes = EXCLUDED.output_shapes,
                metadata = EXCLUDED.metadata,
                updated_at = EXCLUDED.updated_at
        "#;
        
        sqlx::query(query)
            .bind(&model.id)
            .bind(&model.name)
            .bind(&model.version)
            .bind(&model.file_path)
            .bind(&model.file_size)
            .bind(&model.storage_backend)
            .bind(&model.input_shapes)
            .bind(&model.output_shapes)
            .bind(&model.metadata)
            .bind(&model.created_at)
            .bind(&model.updated_at)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    async fn save_tx(&self, tx: &mut Transaction, model: &ModelInfo) -> Result<(), DbError> {
        // TODO: 实现事务版本的保存方法
        // 需要从 Transaction 中获取连接并执行与 save() 相同的 SQL
        // 主要区别：
        // 1. 使用事务连接而非连接池
        // 2. 不自动提交，由调用方决定 commit/rollback
        // 
        // 实现示例：
        // let conn = tx.get_connection();
        // sqlx::query(query).bind(...).execute(conn).await?;
        unimplemented!("save_tx not yet implemented")
    }
    
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ModelInfo>, DbError> {
        let query = "SELECT * FROM models WHERE id = $1";
        
        let result = sqlx::query_as::<_, ModelInfo>(query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(result)
    }
    
    async fn find_by_name_version(&self, name: &str, version: &str) -> Result<Option<ModelInfo>, DbError> {
        let query = "SELECT * FROM models WHERE name = $1 AND version = $2";
        
        let result = sqlx::query_as::<_, ModelInfo>(query)
            .bind(name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(result)
    }
    
    async fn list(&self, filter: &ModelFilter) -> Result<Vec<ModelInfo>, DbError> {
        let mut query_builder = QueryBuilder::new("SELECT * FROM models WHERE 1=1");
        
        if let Some(name) = &filter.name {
            query_builder.push(" AND name ILIKE ");
            query_builder.push_bind(format!("%{}%", name));
        }
        
        // is_valid is computed from metadata and input_shapes
        if let Some(is_valid) = filter.is_valid {
            if is_valid {
                query_builder.push(" AND metadata IS NOT NULL AND input_shapes IS NOT NULL");
            } else {
                query_builder.push(" AND (metadata IS NULL OR input_shapes IS NULL)");
            }
        }
        
        query_builder.push(" ORDER BY created_at DESC");
        
        if let Some(limit) = filter.limit {
            query_builder.push(" LIMIT ");
            query_builder.push_bind(limit as i64);
        }
        
        if let Some(offset) = filter.offset {
            query_builder.push(" OFFSET ");
            query_builder.push_bind(offset as i64);
        }
        
        let results = query_builder
            .build_query_as::<ModelInfo>()
            .fetch_all(&self.pool)
            .await?;
        
        Ok(results)
    }
    
    async fn delete(&self, id: &uuid::Uuid) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM models WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        
        Ok(result.rows_affected() > 0)
    }
    
    async fn delete_tx(&self, tx: &mut Transaction, id: &uuid::Uuid) -> Result<bool, DbError> {
        // TODO: 实现事务版本的删除方法
        unimplemented!("delete_tx not yet implemented")
    }
    
    async fn exists(&self, name: &str, version: &str) -> Result<bool, DbError> {
        let query = "SELECT 1 FROM models WHERE name = $1 AND version = $2 LIMIT 1";
        
        let result: Option<(i32,)> = sqlx::query_as(query)
            .bind(name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;
        
        Ok(result.is_some())
    }
}
```

### 3.2 任务仓储实现

```rust
// src/repositories/task.rs (PostgreSQL)

pub struct PostgresTaskRepository {
    pool: AnyPool,
}

#[async_trait]
impl TaskRepository for PostgresTaskRepository {
    async fn save(&self, task: &InferenceTask) -> Result<(), DbError> {
        let query = r#"
            INSERT INTO inference_tasks (
                id, model_id, user_id, api_key_id, status, inputs, outputs,
                error_message, priority, retry_count, created_at, started_at, completed_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#;
        
        sqlx::query(query)
            .bind(&task.id)
            .bind(&task.model_id)
            .bind(&task.user_id)
            .bind(&task.api_key_id)
            .bind(&task.status)
            .bind(&task.inputs)
            .bind(&task.outputs)
            .bind(&task.error_message)
            .bind(&task.priority)
            .bind(&task.retry_count)
            .bind(&task.created_at)
            .bind(&task.started_at)
            .bind(&task.completed_at)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    async fn set_result(
        &self, 
        id: &uuid::Uuid, 
        status: TaskStatus, 
        outputs: Option<&serde_json::Value>,
        error: Option<&str>
    ) -> Result<(), DbError> {
        let query = r#"
            UPDATE inference_tasks 
            SET status = $2, outputs = $3, error_message = $4, 
                completed_at = NOW(), updated_at = NOW()
            WHERE id = $1
        "#;
        
        sqlx::query(query)
            .bind(id)
            .bind(status)
            .bind(outputs)
            .bind(error)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    async fn cleanup_expired(&self, retention_days: u32, batch_size: usize) -> Result<u64, DbError> {
        let cutoff_date = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        
        let query = r#"
            DELETE FROM inference_tasks 
            WHERE status IN ('completed', 'failed', 'cancelled')
              AND completed_at < $1
            LIMIT $2
        "#;
        
        let result = sqlx::query(query)
            .bind(cutoff_date)
            .bind(batch_size as i64)
            .execute(&self.pool)
            .await?;
        
        Ok(result.rows_affected())
    }
    
    // ... 其他方法实现
}
```

## 4. SQLite 实现

### 4.1 兼容性处理

```rust
// src/repositories/model.rs (SQLite)

pub struct SqliteModelRepository {
    pool: AnyPool,
}

#[async_trait]
impl ModelRepository for SqliteModelRepository {
    async fn save(&self, model: &ModelInfo) -> Result<(), DbError> {
        // SQLite 与 PostgreSQL 的主要区别：
        // 1. BOOLEAN 类型实际是 INTEGER
        // 2. TIMESTAMP 类型实际是 TEXT
        // 3. ON CONFLICT 语法略有不同
        
        let query = r#"
            INSERT INTO models (
                id, name, version, file_path, file_size, storage_backend,
                input_shapes, output_shapes, metadata, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                input_shapes = excluded.input_shapes,
                output_shapes = excluded.output_shapes,
                metadata = excluded.metadata,
                updated_at = excluded.updated_at
        "#;
        
        // sqlx 会自动处理类型转换
        sqlx::query(query)
            .bind(&model.id)
            .bind(&model.name)
            .bind(&model.version)
            .bind(&model.file_path)
            .bind(&model.file_size)
            .bind(&model.storage_backend)
            .bind(&model.input_shapes)
            .bind(&model.output_shapes)
            .bind(&model.metadata)
            .bind(&model.created_at.to_rfc3339())
            .bind(&model.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    // ... 其他方法实现类似，注意参数占位符使用 ? 而不是 $
}
```

## 5. 数据库迁移

### 5.1 迁移文件组织

```
crates/ferrinx-db/src/migrations/
├── postgres/
│   ├── 20240101_000001_create_users.sql
│   ├── 20240101_000002_create_api_keys.sql
│   ├── 20240101_000003_create_models.sql
│   └── 20240101_000004_create_inference_tasks.sql
└── sqlite/
    ├── 20240101_000001_create_users.sql
    ├── 20240101_000002_create_api_keys.sql
    ├── 20240101_000003_create_models.sql
    └── 20240101_000004_create_inference_tasks.sql
```

### 5.2 PostgreSQL 迁移脚本

```sql
-- migrations/postgres/20240101_000001_create_users.sql

CREATE TABLE users (
    id VARCHAR(36) PRIMARY KEY,
    username VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_is_active ON users(is_active);
```

```sql
-- migrations/postgres/20240101_000002_create_api_keys.sql

CREATE TABLE api_keys (
    id VARCHAR(36) PRIMARY KEY,
    user_id VARCHAR(36) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash VARCHAR(64) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    permissions JSONB NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    is_temporary BOOLEAN NOT NULL DEFAULT false,
    last_used_at TIMESTAMP,
    expires_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_is_active ON api_keys(is_active);
CREATE INDEX idx_api_keys_is_temporary ON api_keys(is_temporary);
CREATE INDEX idx_api_keys_expires_at ON api_keys(expires_at);
```

```sql
-- migrations/postgres/20240101_000003_create_models.sql

CREATE TABLE models (
    id VARCHAR(36) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    version VARCHAR(50) NOT NULL,
    file_path VARCHAR(500) NOT NULL,
    file_size BIGINT,
    storage_backend VARCHAR(50) NOT NULL DEFAULT 'local',
    input_shapes JSONB,
    output_shapes JSONB,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(name, version)
);

CREATE INDEX idx_models_name ON models(name);
CREATE INDEX idx_models_name_version ON models(name, version);

-- Note: is_valid is computed from metadata IS NOT NULL AND input_shapes IS NOT NULL
-- No separate column needed
```

```sql
-- migrations/postgres/20240101_000004_create_inference_tasks.sql

CREATE TABLE inference_tasks (
    id VARCHAR(36) PRIMARY KEY,
    model_id VARCHAR(36) REFERENCES models(id),
    user_id VARCHAR(36) REFERENCES users(id),
    api_key_id VARCHAR(36) REFERENCES api_keys(id),
    status VARCHAR(50) NOT NULL,
    inputs JSONB NOT NULL,
    outputs JSONB,
    error_message TEXT,
    priority INTEGER DEFAULT 5 CHECK (priority >= 1 AND priority <= 10),
    retry_count INTEGER DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_inference_tasks_user_id ON inference_tasks(user_id);
CREATE INDEX idx_inference_tasks_model_id ON inference_tasks(model_id);
CREATE INDEX idx_inference_tasks_status ON inference_tasks(status);
CREATE INDEX idx_inference_tasks_created_at ON inference_tasks(created_at);
CREATE INDEX idx_inference_tasks_priority ON inference_tasks(priority DESC);
CREATE INDEX idx_inference_tasks_completed_at ON inference_tasks(completed_at);
```

### 5.3 SQLite 迁移脚本

```sql
-- migrations/sqlite/20240101_000001_create_users.sql

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user',
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_is_active ON users(is_active);
```

```sql
-- migrations/sqlite/20240101_000002_create_api_keys.sql

CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    permissions TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    is_temporary INTEGER NOT NULL DEFAULT 0,
    last_used_at TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_api_keys_user_id ON api_keys(user_id);
CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_is_active ON api_keys(is_active);
CREATE INDEX idx_api_keys_is_temporary ON api_keys(is_temporary);
```

## 6. 事务使用示例

```rust
// 删除用户及其关联数据
use ferrinx_db::*;

async fn delete_user_cascade(db: &DbContext, user_id: uuid::Uuid) -> Result<(), Error> {
    let mut tx = db.begin().await?;
    
    // 删除用户的所有 API Keys
    db.api_keys.delete_by_user_tx(&mut tx, &user_id).await?;
    
    // 删除用户的所有推理任务
    db.tasks.delete_by_user_tx(&mut tx, &user_id).await?;
    
    // 删除用户
    db.users.delete_tx(&mut tx, &user_id).await?;
    
    // 提交事务
    tx.commit().await?;
    
    Ok(())
}

// 模型上传与验证
async fn upload_model_with_validation(
    db: &DbContext,
    model: ModelInfo,
) -> Result<(), Error> {
    let mut tx = db.begin().await?;
    
    // 保存模型元数据（标记为待验证）
    let mut model = model;
    model.is_valid = false;
    db.models.save_tx(&mut tx, &model).await?;
    
    // 其他相关操作...
    
    tx.commit().await?;
    
    // 异步验证（独立事务）
    tokio::spawn(async move {
        if let Err(e) = validate_and_update_model(&db, &model.id).await {
            error!("Model validation failed: {}", e);
        }
    });
    
    Ok(())
}
```

## 7. 错误处理

```rust
// src/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database connection error: {0}")]
    ConnectionError(#[from] sqlx::Error),
    
    #[error("Transaction error: {0}")]
    TransactionError(String),
    
    #[error("Record not found: {0}")]
    NotFound(String),
    
    #[error("Duplicate record: {0}")]
    Duplicate(String),
    
    #[error("Migration error: {0}")]
    MigrationError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    
    #[error("Invalid UUID format: {0}")]
    InvalidUuid(String),
    
    #[error("Invalid datetime format: {0}")]
    InvalidDateTime(String),
    
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    #[error("Foreign key constraint violation: {0}")]
    ForeignKeyViolation(String),
}

impl From<sqlx::Error> for DbError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => DbError::NotFound("Record not found".to_string()),
            sqlx::Error::Database(e) => {
                let msg = e.message();
                if msg.contains("unique constraint") || msg.contains("UNIQUE constraint failed") {
                    DbError::Duplicate(msg.to_string())
                } else if msg.contains("foreign key") || msg.contains("FOREIGN KEY constraint failed") {
                    DbError::ForeignKeyViolation(msg.to_string())
                } else {
                    DbError::ConnectionError(sqlx::Error::Database(e))
                }
            }
            _ => DbError::ConnectionError(err),
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
```

### 7.1 新错误类型说明

- **InvalidUuid**: UUID 格式解析错误，包含无效的用户输入
- **InvalidDateTime**: 日期时间格式解析错误
- **InvalidInput**: 通用输入验证错误，用于 LIMIT/OFFSET 等参数验证
- **ForeignKeyViolation**: 外键约束违反错误，在启用外键的 SQLite 或 PostgreSQL 中触发

## 8. 数据库行转换

### 8.1 使用 TryFrom 进行类型安全转换

从数据库行转换为领域模型时使用 `TryFrom` 而非 `From`，以处理可能的解析错误：

```rust
// src/repositories/model.rs

/// 数据库行结构
#[derive(sqlx::FromRow)]
struct ModelRow {
    id: String,
    name: String,
    version: String,
    file_path: String,
    file_size: Option<i64>,
    storage_backend: String,
    input_shapes: Option<String>,
    output_shapes: Option<String>,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
}

/// 使用 TryFrom 实现安全的行转换
impl TryFrom<ModelRow> for ModelInfo {
    type Error = crate::error::DbError;
    
    fn try_from(row: ModelRow) -> crate::error::Result<Self> {
        Ok(Self {
            id: Uuid::parse_str(&row.id)
                .map_err(|e| crate::error::DbError::InvalidUuid(
                    format!("Invalid model id '{}': {}", row.id, e)
                ))?,
            name: row.name,
            version: row.version,
            file_path: row.file_path,
            file_size: row.file_size,
            storage_backend: row.storage_backend,
            input_shapes: row.input_shapes
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(crate::error::DbError::SerializationError)?,
            output_shapes: row.output_shapes
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(crate::error::DbError::SerializationError)?,
            metadata: row.metadata
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(crate::error::DbError::SerializationError)?,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)
                .map_err(|e| crate::error::DbError::InvalidDateTime(
                    format!("Invalid created_at '{}': {}", row.created_at, e)
                ))?
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&row.updated_at)
                .map_err(|e| crate::error::DbError::InvalidDateTime(
                    format!("Invalid updated_at '{}': {}", row.updated_at, e)
                ))?
                .with_timezone(&Utc),
        })
    }
}
```

**使用 TryFrom 的优势**：
- 类型安全：所有解析错误都被显式处理
- 详细错误信息：包含具体的字段值和错误原因
- 避免 panic：不会因为无效数据导致程序崩溃
- 易于调试：可以追踪到具体哪个字段解析失败

### 8.2 SQL 注入防护 - LIMIT/OFFSET 验证

为防止 SQL 注入攻击，所有用户提供的 LIMIT 和 OFFSET 参数都需要进行验证：

```rust
// src/repositories/model.rs (PostgreSQL)

/// 最大允许的 LIMIT 值，防止资源耗尽攻击
const MAX_LIMIT: usize = 10000;

impl ModelRepository for PostgresModelRepository {
    async fn list(&self, filter: &ModelFilter) -> Result<Vec<ModelInfo>, DbError> {
        let mut query_builder = QueryBuilder::new("SELECT * FROM models WHERE 1=1");
        
        if let Some(name) = &filter.name {
            query_builder.push(" AND name ILIKE ");
            query_builder.push_bind(format!("%{}%", name));
        }
        
        // is_valid 是计算字段，基于 metadata 和 input_shapes
        if let Some(is_valid) = filter.is_valid {
            if is_valid {
                query_builder.push(" AND metadata IS NOT NULL AND input_shapes IS NOT NULL");
            } else {
                query_builder.push(" AND (metadata IS NULL OR input_shapes IS NULL)");
            }
        }
        
        query_builder.push(" ORDER BY created_at DESC");
        
        // 验证并添加 LIMIT
        if let Some(limit) = filter.limit {
            if limit > MAX_LIMIT {
                return Err(DbError::InvalidInput(
                    format!("Limit {} exceeds maximum allowed {}", limit, MAX_LIMIT)
                ));
            }
            query_builder.push(" LIMIT ");
            query_builder.push_bind(limit as i64);
        }
        
        // 验证并添加 OFFSET
        if let Some(offset) = filter.offset {
            query_builder.push(" OFFSET ");
            query_builder.push_bind(offset as i64);
        }
        
        let rows: Vec<ModelRow> = query_builder
            .build_query_as::<ModelRow>()
            .fetch_all(&self.pool)
            .await?;
        
        // 使用 TryFrom 转换每一行
        rows.into_iter()
            .map(ModelInfo::try_from)
            .collect::<Result<Vec<_>, _>>()
    }
}
```

**SQL 注入防护措施**：
- **参数绑定**：使用 sqlx 的 `push_bind` 方法，自动转义特殊字符
- **LIMIT 上限**：限制最大返回行数，防止 DoS 攻击
- **输入验证**：在构造 SQL 之前验证参数合法性
- **使用 QueryBuilder**：避免字符串拼接构造 SQL

### 8.3 SQLite 外键约束

SQLite 默认不启用外键约束，需要在连接后显式启用：

```rust
// 在 DbContext::new 中启用外键约束
if backend == DatabaseBackend::Sqlite {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
}
```

**外键约束的作用**：
- 防止删除被引用的记录（如删除有 API Key 的用户）
- 级联删除（ON DELETE CASCADE）自动清理关联数据
- 保证数据完整性

## 9. 性能优化

### 9.1 连接池配置

```rust
impl DbContext {
    pub async fn new_with_pool_size(
        config: &DatabaseConfig,
        pool_size: u32,
    ) -> Result<Self, DbError> {
        let pool_opts = sqlx::any::AnyPoolOptions::new()
            .max_connections(pool_size)
            .min_connections(1)
            .connect_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(3600));
        
        let pool = pool_opts.connect(&config.url).await?;
        
        // ... 其他初始化
    }
}
```

### 9.2 批量操作

```rust
impl TaskRepository for PostgresTaskRepository {
    async fn cleanup_expired(&self, retention_days: u32, batch_size: usize) -> Result<u64, DbError> {
        let mut total_deleted = 0;
        
        loop {
            let deleted = self.cleanup_batch(retention_days, batch_size).await?;
            total_deleted += deleted;
            
            if deleted < batch_size as u64 {
                break;
            }
            
            // 短暂休眠，避免长时间占用数据库
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        Ok(total_deleted)
    }
    
    async fn cleanup_batch(&self, retention_days: u32, batch_size: usize) -> Result<u64, DbError> {
        // 单批次清理
    }
}
```

### 10.3 查询优化

```rust
// 使用索引覆盖查询
async fn find_by_hash_minimal(&self, key_hash: &str) -> Result<Option<ApiKeyMinimal>, DbError> {
    let query = "SELECT id, user_id, is_active FROM api_keys WHERE key_hash = $1";
    
    let result = sqlx::query_as::<_, ApiKeyMinimal>(query)
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;
    
    Ok(result)
}
```

## 11. 测试策略

### 11.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    
    async fn setup_test_db() -> DbContext {
        let pool = SqlitePoolOptions::new()
            .connect(":memory:")
            .await
            .unwrap();
        
        // 运行迁移
        sqlx::migrate!("migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        
        DbContext::from_pool(pool).await.unwrap()
    }
    
    #[tokio::test]
    async fn test_user_repository() {
        let db = setup_test_db().await;
        
        let user = User {
            id: uuid::Uuid::new_v4(),
            username: "testuser".to_string(),
            role: UserRole::User,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        
        db.users.save(&user).await.unwrap();
        
        let found = db.users.find_by_username("testuser").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().username, "testuser");
    }
    
    #[tokio::test]
    async fn test_model_unique_constraint() {
        let db = setup_test_db().await;
        
        let model1 = create_test_model("model1", "1.0");
        let model2 = create_test_model("model1", "1.0"); // 相同 name+version
        
        db.models.save(&model1).await.unwrap();
        
        let result = db.models.save(&model2).await;
        assert!(matches!(result, Err(DbError::Duplicate(_))));
    }
    
    #[tokio::test]
    async fn test_transaction_rollback() {
        let db = setup_test_db().await;
        
        let user = create_test_user("user1");
        db.users.save(&user).await.unwrap();
        
        let mut tx = db.begin().await.unwrap();
        
        db.users.delete_tx(&mut tx, &user.id).await.unwrap();
        
        // 不提交，直接 rollback
        tx.rollback().await.unwrap();
        
        // 用户应该还存在
        let found = db.users.find_by_id(&user.id).await.unwrap();
        assert!(found.is_some());
    }
}
```

### 11.2 集成测试

```rust
#[tokio::test]
#[ignore] // 需要真实 PostgreSQL
async fn test_postgres_integration() {
    let config = DatabaseConfig {
        backend: DatabaseBackend::Postgresql,
        url: "postgresql://test:test@localhost/ferrinx_test".to_string(),
        max_connections: 5,
        run_migrations: true,
    };
    
    let db = DbContext::new(&config).await.unwrap();
    
    // 测试完整流程
    let user = create_test_user("integration_user");
    db.users.save(&user).await.unwrap();
    
    let api_key = create_test_api_key(&user.id);
    db.api_keys.save(&api_key).await.unwrap();
    
    // 验证
    let found = db.api_keys.find_by_hash(&api_key.key_hash).await.unwrap();
    assert!(found.is_some());
}
```

## 12. 监控指标

```rust
impl DbContext {
    pub fn get_pool_metrics(&self) -> PoolMetrics {
        PoolMetrics {
            connections: self.pool.size() as u32,
            idle_connections: self.pool.num_idle() as u32,
        }
    }
}

pub struct PoolMetrics {
    pub connections: u32,
    pub idle_connections: u32,
}
```

## 13. 设计要点

### 13.1 抽象与实现分离

- 业务代码依赖 `dyn Repository` trait
- 具体实现通过 `DbContext` 组合
- 便于切换数据库后端

### 13.2 事务支持

- 提供 `_tx` 方法用于事务操作
- `Transaction` 类型封装事务生命周期
- 自动回滚机制

### 13.3 兼容性处理

- PostgreSQL 和 SQLite 共享大部分代码
- 通过 `AnyPool` 实现后端无关
- 迁移脚本分离，适配各自特性

### 13.4 错误处理

- 统一的 `DbError` 类型
- 区分连接错误、查询错误、约束错误
- 便于上层处理和日志记录

## 14. 后续优化

### 14.1 读写分离

```rust
pub struct DbContext {
    read_pool: AnyPool,
    write_pool: AnyPool,
    // ...
}

// 读操作使用 read_pool
// 写操作使用 write_pool
```

### 14.2 查询缓存

```rust
pub struct CachedModelRepository {
    inner: Arc<dyn ModelRepository>,
    cache: Arc<RwLock<LruCache<String, ModelInfo>>>,
}

#[async_trait]
impl ModelRepository for CachedModelRepository {
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<ModelInfo>, DbError> {
        let key = id.to_string();
        
        // 先查缓存
        {
            let cache = self.cache.read().await;
            if let Some(model) = cache.get(&key) {
                return Ok(Some(model.clone()));
            }
        }
        
        // 缓存未命中，查询数据库
        let result = self.inner.find_by_id(id).await?;
        
        // 写入缓存
        if let Some(ref model) = result {
            let mut cache = self.cache.write().await;
            cache.put(key, model.clone());
        }
        
        Ok(result)
    }
}
```

### 14.3 审计日志

```rust
pub struct AuditedRepository<T: Repository> {
    inner: T,
    audit_log: Arc<dyn AuditLog>,
}

impl<T: Repository> Repository for AuditedRepository<T> {
    async fn save(&self, record: &Record) -> Result<(), DbError> {
        self.inner.save(record).await?;
        self.audit_log.log(Operation::Create, record).await;
        Ok(())
    }
}
```
