use async_trait::async_trait;
use chrono::Utc;
use ferrinx_common::*;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::Result;
use crate::traits::{ApiKeyRepository, ModelRepository, TaskRepository, UserRepository};

pub struct SqliteModelRepository {
    pool: SqlitePool,
}

impl SqliteModelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ModelRepository for SqliteModelRepository {
    async fn save(&self, model: &ModelInfo) -> Result<()> {
        let query = r#"
            INSERT INTO models (
                id, name, version, file_path, file_size, storage_backend,
                input_shapes, output_shapes, metadata, is_valid, validation_error,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                file_path = excluded.file_path,
                file_size = excluded.file_size,
                is_valid = excluded.is_valid,
                validation_error = excluded.validation_error,
                updated_at = excluded.updated_at
        "#;

        sqlx::query(query)
            .bind(model.id.to_string())
            .bind(&model.name)
            .bind(&model.version)
            .bind(&model.file_path)
            .bind(model.file_size)
            .bind(&model.storage_backend)
            .bind(&model.input_shapes)
            .bind(&model.output_shapes)
            .bind(&model.metadata)
            .bind(model.is_valid)
            .bind(&model.validation_error)
            .bind(model.created_at.to_rfc3339())
            .bind(model.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<ModelInfo>> {
        let query = "SELECT * FROM models WHERE id = ?1";

        let result: Option<ModelRow> = sqlx::query_as(query)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn find_by_name_version(&self, name: &str, version: &str) -> Result<Option<ModelInfo>> {
        let query = "SELECT * FROM models WHERE name = ?1 AND version = ?2";

        let result: Option<ModelRow> = sqlx::query_as(query)
            .bind(name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn list(&self, filter: &ModelFilter) -> Result<Vec<ModelInfo>> {
        let mut query = String::from("SELECT * FROM models WHERE 1=1");
        let mut binds: Vec<String> = Vec::new();

        if let Some(name) = &filter.name {
            query.push_str(" AND name LIKE ?");
            binds.push(format!("%{}%", name));
        }

        if let Some(is_valid) = filter.is_valid {
            query.push_str(" AND is_valid = ?");
            binds.push(if is_valid { "1" } else { "0" }.to_string());
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = filter.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        let mut sql_query = sqlx::query_as::<_, ModelRow>(&query);
        for bind in binds {
            sql_query = sql_query.bind(bind);
        }

        let results: Vec<ModelRow> = sql_query.fetch_all(&self.pool).await?;

        Ok(results.into_iter().map(|r| r.into()).collect())
    }

    async fn delete(&self, id: &Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM models WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_validation_status(
        &self,
        id: &Uuid,
        is_valid: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let query = r#"
            UPDATE models 
            SET is_valid = ?2, validation_error = ?3, updated_at = ?4
            WHERE id = ?1
        "#;

        sqlx::query(query)
            .bind(id.to_string())
            .bind(is_valid)
            .bind(error)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn exists(&self, name: &str, version: &str) -> Result<bool> {
        let query = "SELECT 1 FROM models WHERE name = ?1 AND version = ?2 LIMIT 1";

        let result: Option<(i32,)> = sqlx::query_as(query)
            .bind(name)
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.is_some())
    }
}

#[derive(Debug, sqlx::FromRow)]
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
    is_valid: bool,
    validation_error: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<ModelRow> for ModelInfo {
    fn from(row: ModelRow) -> Self {
        Self {
            id: Uuid::parse_str(&row.id).unwrap_or_default(),
            name: row.name,
            version: row.version,
            file_path: row.file_path,
            file_size: row.file_size,
            storage_backend: row.storage_backend,
            input_shapes: row.input_shapes.and_then(|s| serde_json::from_str(&s).ok()),
            output_shapes: row.output_shapes.and_then(|s| serde_json::from_str(&s).ok()),
            metadata: row.metadata.and_then(|s| serde_json::from_str(&s).ok()),
            is_valid: row.is_valid,
            validation_error: row.validation_error,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

pub struct SqliteTaskRepository {
    pool: SqlitePool,
}

impl SqliteTaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    async fn save(&self, task: &InferenceTask) -> Result<()> {
        let query = r#"
            INSERT INTO inference_tasks (
                id, model_id, user_id, api_key_id, status, inputs, outputs,
                error_message, priority, retry_count, created_at, started_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#;

        sqlx::query(query)
            .bind(task.id.to_string())
            .bind(task.model_id.to_string())
            .bind(task.user_id.to_string())
            .bind(task.api_key_id.to_string())
            .bind(task.status.as_str())
            .bind(serde_json::to_string(&task.inputs)?)
            .bind(task.outputs.as_ref().map(|o| serde_json::to_string(o)).transpose()?)
            .bind(&task.error_message)
            .bind(task.priority)
            .bind(task.retry_count)
            .bind(task.created_at.to_rfc3339())
            .bind(task.started_at.map(|t| t.to_rfc3339()))
            .bind(task.completed_at.map(|t| t.to_rfc3339()))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<InferenceTask>> {
        let query = "SELECT * FROM inference_tasks WHERE id = ?1";

        let result: Option<TaskRow> = sqlx::query_as(query)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn update_status(&self, id: &Uuid, status: TaskStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let query = match status {
            TaskStatus::Running => {
                "UPDATE inference_tasks SET status = ?2, started_at = ?3 WHERE id = ?1"
            }
            _ => "UPDATE inference_tasks SET status = ?2 WHERE id = ?1",
        };

        if matches!(status, TaskStatus::Running) {
            sqlx::query(query)
                .bind(id.to_string())
                .bind(status.as_str())
                .bind(now)
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("UPDATE inference_tasks SET status = ?2 WHERE id = ?1")
                .bind(id.to_string())
                .bind(status.as_str())
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn set_result(
        &self,
        id: &Uuid,
        status: TaskStatus,
        outputs: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<()> {
        let query = r#"
            UPDATE inference_tasks 
            SET status = ?2, outputs = ?3, error_message = ?4, 
                completed_at = ?5
            WHERE id = ?1
        "#;

        sqlx::query(query)
            .bind(id.to_string())
            .bind(status.as_str())
            .bind(outputs.map(|o| serde_json::to_string(o)).transpose()?)
            .bind(error)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list(&self, filter: &TaskFilter) -> Result<Vec<InferenceTask>> {
        let mut query = String::from("SELECT * FROM inference_tasks WHERE 1=1");
        let mut binds: Vec<String> = Vec::new();

        if let Some(user_id) = filter.user_id {
            query.push_str(" AND user_id = ?");
            binds.push(user_id.to_string());
        }

        if let Some(model_id) = filter.model_id {
            query.push_str(" AND model_id = ?");
            binds.push(model_id.to_string());
        }

        if let Some(status) = filter.status {
            query.push_str(" AND status = ?");
            binds.push(status.as_str().to_string());
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = filter.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        let mut sql_query = sqlx::query_as::<_, TaskRow>(&query);
        for bind in binds {
            sql_query = sql_query.bind(bind);
        }

        let results: Vec<TaskRow> = sql_query.fetch_all(&self.pool).await?;

        Ok(results.into_iter().map(|r| r.into()).collect())
    }

    async fn delete(&self, id: &Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM inference_tasks WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_by_user(&self, user_id: &Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM inference_tasks WHERE user_id = ?1")
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn delete_by_model(&self, model_id: &Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM inference_tasks WHERE model_id = ?1")
            .bind(model_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn cleanup_expired(&self, retention_days: u32, batch_size: usize) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);

        let query = r#"
            DELETE FROM inference_tasks 
            WHERE rowid IN (
                SELECT rowid FROM inference_tasks 
                WHERE status IN ('completed', 'failed', 'cancelled')
                  AND completed_at < ?1
                LIMIT ?2
            )
        "#;

        let result = sqlx::query(query)
            .bind(cutoff.to_rfc3339())
            .bind(batch_size as i64)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn count_by_status(&self, status: TaskStatus) -> Result<i64> {
        let query = "SELECT COUNT(*) FROM inference_tasks WHERE status = ?1";

        let result: (i64,) = sqlx::query_as(query)
            .bind(status.as_str())
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0)
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TaskRow {
    id: String,
    model_id: String,
    user_id: String,
    api_key_id: String,
    status: String,
    inputs: String,
    outputs: Option<String>,
    error_message: Option<String>,
    priority: i32,
    retry_count: i32,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl From<TaskRow> for InferenceTask {
    fn from(row: TaskRow) -> Self {
        Self {
            id: Uuid::parse_str(&row.id).unwrap_or_default(),
            model_id: Uuid::parse_str(&row.model_id).unwrap_or_default(),
            user_id: Uuid::parse_str(&row.user_id).unwrap_or_default(),
            api_key_id: Uuid::parse_str(&row.api_key_id).unwrap_or_default(),
            status: TaskStatus::from_str(&row.status).unwrap_or(TaskStatus::Pending),
            inputs: serde_json::from_str(&row.inputs).unwrap_or(serde_json::Value::Null),
            outputs: row.outputs.and_then(|s| serde_json::from_str(&s).ok()),
            error_message: row.error_message,
            priority: row.priority,
            retry_count: row.retry_count,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            started_at: row.started_at.and_then(|t| {
                chrono::DateTime::parse_from_rfc3339(&t)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }),
            completed_at: row.completed_at.and_then(|t| {
                chrono::DateTime::parse_from_rfc3339(&t)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }),
        }
    }
}

pub struct SqliteApiKeyRepository {
    pool: SqlitePool,
}

impl SqliteApiKeyRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ApiKeyRepository for SqliteApiKeyRepository {
    async fn save(&self, key: &ApiKeyRecord) -> Result<()> {
        let query = r#"
            INSERT INTO api_keys (
                id, user_id, key_hash, name, permissions, is_active, is_temporary,
                last_used_at, expires_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                permissions = excluded.permissions,
                is_active = excluded.is_active,
                expires_at = excluded.expires_at,
                updated_at = excluded.updated_at
        "#;

        sqlx::query(query)
            .bind(key.id.to_string())
            .bind(key.user_id.to_string())
            .bind(&key.key_hash)
            .bind(&key.name)
            .bind(serde_json::to_string(&key.permissions)?)
            .bind(key.is_active)
            .bind(key.is_temporary)
            .bind(key.last_used_at.map(|t| t.to_rfc3339()))
            .bind(key.expires_at.map(|t| t.to_rfc3339()))
            .bind(key.created_at.to_rfc3339())
            .bind(key.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn find_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRecord>> {
        let query = "SELECT * FROM api_keys WHERE key_hash = ?1";

        let result: Option<ApiKeyRow> = sqlx::query_as(query)
            .bind(key_hash)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<ApiKeyRecord>> {
        let query = "SELECT * FROM api_keys WHERE id = ?1";

        let result: Option<ApiKeyRow> = sqlx::query_as(query)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn find_by_user(&self, user_id: &Uuid) -> Result<Vec<ApiKeyRecord>> {
        let query = "SELECT * FROM api_keys WHERE user_id = ?1 ORDER BY created_at DESC";

        let results: Vec<ApiKeyRow> = sqlx::query_as(query)
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        Ok(results.into_iter().map(|r| r.into()).collect())
    }

    async fn find_temporary_by_user(&self, user_id: &Uuid) -> Result<Vec<ApiKeyRecord>> {
        let query =
            "SELECT * FROM api_keys WHERE user_id = ?1 AND is_temporary = 1 ORDER BY created_at DESC";

        let results: Vec<ApiKeyRow> = sqlx::query_as(query)
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        Ok(results.into_iter().map(|r| r.into()).collect())
    }

    async fn update_last_used(&self, id: &Uuid) -> Result<()> {
        let query = "UPDATE api_keys SET last_used_at = ?2 WHERE id = ?1";

        sqlx::query(query)
            .bind(id.to_string())
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn deactivate(&self, id: &Uuid) -> Result<bool> {
        let query = "UPDATE api_keys SET is_active = 0, updated_at = ?2 WHERE id = ?1";

        let result = sqlx::query(query)
            .bind(id.to_string())
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, id: &Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_permissions(&self, id: &Uuid, permissions: &Permissions) -> Result<()> {
        let query = "UPDATE api_keys SET permissions = ?2, updated_at = ?3 WHERE id = ?1";

        sqlx::query(query)
            .bind(id.to_string())
            .bind(serde_json::to_string(permissions)?)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete_by_user(&self, user_id: &Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM api_keys WHERE user_id = ?1")
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn cleanup_expired_temp_keys(&self) -> Result<u64> {
        let now = Utc::now().to_rfc3339();

        let query = "DELETE FROM api_keys WHERE is_temporary = 1 AND expires_at < ?1";

        let result = sqlx::query(query)
            .bind(now)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ApiKeyRow {
    id: String,
    user_id: String,
    key_hash: String,
    name: String,
    permissions: String,
    is_active: bool,
    is_temporary: bool,
    last_used_at: Option<String>,
    expires_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<ApiKeyRow> for ApiKeyRecord {
    fn from(row: ApiKeyRow) -> Self {
        Self {
            id: Uuid::parse_str(&row.id).unwrap_or_default(),
            user_id: Uuid::parse_str(&row.user_id).unwrap_or_default(),
            key_hash: row.key_hash,
            name: row.name,
            permissions: serde_json::from_str(&row.permissions).unwrap_or_default(),
            is_active: row.is_active,
            is_temporary: row.is_temporary,
            last_used_at: row.last_used_at.and_then(|t| {
                chrono::DateTime::parse_from_rfc3339(&t)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }),
            expires_at: row.expires_at.and_then(|t| {
                chrono::DateTime::parse_from_rfc3339(&t)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }),
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

pub struct SqliteUserRepository {
    pool: SqlitePool,
}

impl SqliteUserRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for SqliteUserRepository {
    async fn save(&self, user: &User) -> Result<()> {
        let query = r#"
            INSERT INTO users (
                id, username, password_hash, role, is_active, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                username = excluded.username,
                password_hash = excluded.password_hash,
                role = excluded.role,
                is_active = excluded.is_active,
                updated_at = excluded.updated_at
        "#;

        sqlx::query(query)
            .bind(user.id.to_string())
            .bind(&user.username)
            .bind(&user.password_hash)
            .bind(match user.role {
                UserRole::User => "user",
                UserRole::Admin => "admin",
            })
            .bind(user.is_active)
            .bind(user.created_at.to_rfc3339())
            .bind(user.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>> {
        let query = "SELECT * FROM users WHERE id = ?1";

        let result: Option<UserRow> = sqlx::query_as(query)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>> {
        let query = "SELECT * FROM users WHERE username = ?1";

        let result: Option<UserRow> = sqlx::query_as(query)
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.map(|r| r.into()))
    }

    async fn delete(&self, id: &Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn list(&self, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<User>> {
        let mut query = String::from("SELECT * FROM users ORDER BY created_at DESC");

        if let Some(limit) = limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        let results: Vec<UserRow> = sqlx::query_as(&query).fetch_all(&self.pool).await?;

        Ok(results.into_iter().map(|r| r.into()).collect())
    }

    async fn count(&self) -> Result<u64> {
        let query = "SELECT COUNT(*) FROM users";

        let result: (i64,) = sqlx::query_as(query).fetch_one(&self.pool).await?;

        Ok(result.0 as u64)
    }

    async fn update(&self, id: &Uuid, updates: &UserUpdates) -> Result<()> {
        let mut sets = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref username) = updates.username {
            sets.push("username = ?");
            binds.push(username.clone());
        }

        if let Some(ref password_hash) = updates.password_hash {
            sets.push("password_hash = ?");
            binds.push(password_hash.clone());
        }

        if let Some(ref role) = updates.role {
            sets.push("role = ?");
            binds.push(match role {
                UserRole::User => "user".to_string(),
                UserRole::Admin => "admin".to_string(),
            });
        }

        if let Some(is_active) = updates.is_active {
            sets.push("is_active = ?");
            binds.push(if is_active { "1" } else { "0" }.to_string());
        }

        if sets.is_empty() {
            return Ok(());
        }

        sets.push("updated_at = ?");
        binds.push(Utc::now().to_rfc3339());

        let query = format!("UPDATE users SET {} WHERE id = ?", sets.join(", "));
        binds.push(id.to_string());

        let mut sql_query = sqlx::query(&query);
        for bind in binds {
            sql_query = sql_query.bind(bind);
        }

        sql_query.execute(&self.pool).await?;

        Ok(())
    }

    async fn exists(&self) -> Result<bool> {
        let query = "SELECT 1 FROM users LIMIT 1";

        let result: Option<(i32,)> = sqlx::query_as(query)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result.is_some())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    password_hash: String,
    role: String,
    is_active: bool,
    created_at: String,
    updated_at: String,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: Uuid::parse_str(&row.id).unwrap_or_default(),
            username: row.username,
            password_hash: row.password_hash,
            role: match row.role.as_str() {
                "admin" => UserRole::Admin,
                _ => UserRole::User,
            },
            is_active: row.is_active,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}
