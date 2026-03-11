use ferrinx_common::{DatabaseBackend, DatabaseConfig};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::error::{DbError, Result};
use crate::repositories::{
    SqliteApiKeyRepository, SqliteModelRepository, SqliteTaskRepository, SqliteUserRepository,
};
use crate::traits::{ApiKeyRepository, ModelRepository, TaskRepository, UserRepository};

pub struct DbContext {
    pool: SqlitePool,
    pub models: Arc<dyn ModelRepository>,
    pub tasks: Arc<dyn TaskRepository>,
    pub api_keys: Arc<dyn ApiKeyRepository>,
    pub users: Arc<dyn UserRepository>,
}

impl DbContext {
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        if config.backend != DatabaseBackend::Sqlite {
            return Err(DbError::InvalidInput(
                "Only SQLite backend is currently supported".to_string(),
            ));
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url.replace("sqlite://", ""))
            .await?;

        if config.run_migrations {
            Self::run_migrations(&pool).await?;
        }

        let models = Arc::new(SqliteModelRepository::new(pool.clone()));
        let tasks = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let api_keys = Arc::new(SqliteApiKeyRepository::new(pool.clone()));
        let users = Arc::new(SqliteUserRepository::new(pool.clone()));

        Ok(Self {
            pool,
            models,
            tasks,
            api_keys,
            users,
        })
    }

    pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        let migrations = vec![
            Self::migration_create_users(),
            Self::migration_create_api_keys(),
            Self::migration_create_models(),
            Self::migration_create_inference_tasks(),
        ];

        for migration in migrations {
            sqlx::query(&migration).execute(pool).await?;
        }

        Ok(())
    }

    fn migration_create_users() -> String {
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'user',
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        
        CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
        CREATE INDEX IF NOT EXISTS idx_users_is_active ON users(is_active);
        "#
        .to_string()
    }

    fn migration_create_api_keys() -> String {
        r#"
        CREATE TABLE IF NOT EXISTS api_keys (
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
        
        CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);
        CREATE INDEX IF NOT EXISTS idx_api_keys_key_hash ON api_keys(key_hash);
        CREATE INDEX IF NOT EXISTS idx_api_keys_is_active ON api_keys(is_active);
        CREATE INDEX IF NOT EXISTS idx_api_keys_is_temporary ON api_keys(is_temporary);
        CREATE INDEX IF NOT EXISTS idx_api_keys_expires_at ON api_keys(expires_at);
        "#
        .to_string()
    }

    fn migration_create_models() -> String {
        r#"
        CREATE TABLE IF NOT EXISTS models (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            version TEXT NOT NULL,
            file_path TEXT NOT NULL,
            file_size INTEGER,
            storage_backend TEXT NOT NULL DEFAULT 'local',
            input_shapes TEXT,
            output_shapes TEXT,
            metadata TEXT,
            is_valid INTEGER NOT NULL DEFAULT 1,
            validation_error TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(name, version)
        );
        
        CREATE INDEX IF NOT EXISTS idx_models_name ON models(name);
        CREATE INDEX IF NOT EXISTS idx_models_is_valid ON models(is_valid);
        CREATE INDEX IF NOT EXISTS idx_models_name_version ON models(name, version);
        "#
        .to_string()
    }

    fn migration_create_inference_tasks() -> String {
        r#"
        CREATE TABLE IF NOT EXISTS inference_tasks (
            id TEXT PRIMARY KEY,
            model_id TEXT REFERENCES models(id),
            user_id TEXT REFERENCES users(id),
            api_key_id TEXT REFERENCES api_keys(id),
            status TEXT NOT NULL,
            inputs TEXT NOT NULL,
            outputs TEXT,
            error_message TEXT,
            priority INTEGER DEFAULT 5 CHECK (priority >= 1 AND priority <= 10),
            retry_count INTEGER DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            started_at TEXT,
            completed_at TEXT
        );
        
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_user_id ON inference_tasks(user_id);
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_model_id ON inference_tasks(model_id);
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_status ON inference_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_created_at ON inference_tasks(created_at);
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_priority ON inference_tasks(priority DESC);
        CREATE INDEX IF NOT EXISTS idx_inference_tasks_completed_at ON inference_tasks(completed_at);
        "#
        .to_string()
    }

    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrinx_common::{
        ApiKeyRecord, InferenceTask, ModelInfo, Permissions, TaskStatus, User, UserRole,
    };
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    async fn setup_test_db() -> (NamedTempFile, DbContext) {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            url: format!("sqlite://{}", path),
            max_connections: 1,
            run_migrations: true,
        };

        let db = DbContext::new(&config).await.unwrap();
        (temp_file, db)
    }

    #[tokio::test]
    async fn test_db_health_check() {
        let (_temp, db) = setup_test_db().await;
        assert!(db.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn test_user_repository() {
        let (_temp, db) = setup_test_db().await;

        let user = User {
            id: Uuid::new_v4(),
            username: "testuser".to_string(),
            password_hash: "hash123".to_string(),
            role: UserRole::User,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        db.users.save(&user).await.unwrap();

        let found = db.users.find_by_username("testuser").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().username, "testuser");

        let count = db.users.count().await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_model_repository() {
        let (_temp, db) = setup_test_db().await;

        let model = ModelInfo {
            id: Uuid::new_v4(),
            name: "test-model".to_string(),
            version: "1.0.0".to_string(),
            file_path: "/models/test.onnx".to_string(),
            file_size: Some(1024),
            storage_backend: "local".to_string(),
            input_shapes: None,
            output_shapes: None,
            metadata: None,
            is_valid: true,
            validation_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        db.models.save(&model).await.unwrap();

        let found = db
            .models
            .find_by_name_version("test-model", "1.0.0")
            .await
            .unwrap();
        assert!(found.is_some());

        let exists = db.models.exists("test-model", "1.0.0").await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_api_key_repository() {
        let (_temp, db) = setup_test_db().await;

        let user = User {
            id: Uuid::new_v4(),
            username: "keyuser".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.users.save(&user).await.unwrap();

        let api_key = ApiKeyRecord {
            id: Uuid::new_v4(),
            user_id: user.id,
            key_hash: "test_hash_123".to_string(),
            name: "test-key".to_string(),
            permissions: Permissions::user_default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        db.api_keys.save(&api_key).await.unwrap();

        let found = db.api_keys.find_by_hash("test_hash_123").await.unwrap();
        assert!(found.is_some());

        let user_keys = db.api_keys.find_by_user(&user.id).await.unwrap();
        assert_eq!(user_keys.len(), 1);
    }

    #[tokio::test]
    async fn test_task_repository() {
        let (_temp, db) = setup_test_db().await;

        let user = User {
            id: Uuid::new_v4(),
            username: "taskuser".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.users.save(&user).await.unwrap();

        let api_key = ApiKeyRecord {
            id: Uuid::new_v4(),
            user_id: user.id,
            key_hash: "task_key_hash".to_string(),
            name: "task-key".to_string(),
            permissions: Permissions::user_default(),
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.api_keys.save(&api_key).await.unwrap();

        let model = ModelInfo {
            id: Uuid::new_v4(),
            name: "task-model".to_string(),
            version: "1.0".to_string(),
            file_path: "/models/task.onnx".to_string(),
            file_size: None,
            storage_backend: "local".to_string(),
            input_shapes: None,
            output_shapes: None,
            metadata: None,
            is_valid: true,
            validation_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.models.save(&model).await.unwrap();

        let task = InferenceTask {
            id: Uuid::new_v4(),
            model_id: model.id,
            user_id: user.id,
            api_key_id: api_key.id,
            status: TaskStatus::Pending,
            inputs: serde_json::json!({"input": [1.0, 2.0]}),
            outputs: None,
            error_message: None,
            priority: 5,
            retry_count: 0,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
        };

        db.tasks.save(&task).await.unwrap();

        let found = db.tasks.find_by_id(&task.id).await.unwrap();
        assert!(found.is_some());

        let count = db.tasks.count_by_status(TaskStatus::Pending).await.unwrap();
        assert_eq!(count, 1);
    }
}
