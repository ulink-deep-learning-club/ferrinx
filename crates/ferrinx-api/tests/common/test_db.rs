use std::sync::Arc;

use ferrinx_common::{
    ApiKeyRecord, DatabaseBackend, DatabaseConfig, InferenceTask, ModelInfo, Permissions, User,
    UserRole,
};
use ferrinx_db::DbContext;
use tempfile::NamedTempFile;
use uuid::Uuid;

pub struct TestDb {
    pub db: Arc<DbContext>,
    _temp_file: NamedTempFile,
}

impl TestDb {
    pub async fn new() -> Self {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let path = temp_file.path().to_str().expect("Invalid path");

        let config = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            url: format!("sqlite://{}", path),
            max_connections: 5,
            run_migrations: true,
        };

        let db = DbContext::new(&config)
            .await
            .expect("Failed to create test database");

        Self {
            db: Arc::new(db),
            _temp_file: temp_file,
        }
    }

    pub fn temp_file_path(&self) -> String {
        self._temp_file.path().to_str().expect("Invalid path").to_string()
    }

    pub async fn create_user(&self, username: &str, role: UserRole) -> User {
        let user = User {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash: hash_password("password"),
            role,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        self.db.users.save(&user).await.expect("Failed to create user");
        user
    }

    pub async fn create_api_key(&self, user: &User, name: &str, admin: bool) -> (Uuid, String) {
        let key_id = Uuid::new_v4();
        let raw_key = generate_raw_key();
        let key_hash = ferrinx_common::hash_key(&raw_key);

        let api_key = ApiKeyRecord {
            id: key_id,
            user_id: user.id,
            key_hash,
            name: name.to_string(),
            permissions: if admin {
                Permissions::admin_default()
            } else {
                Permissions::user_default()
            },
            is_active: true,
            is_temporary: false,
            last_used_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db
            .api_keys
            .save(&api_key)
            .await
            .expect("Failed to create API key");

        (key_id, raw_key)
    }

    pub async fn create_model(&self, name: &str, version: &str) -> ModelInfo {
        self.create_model_with_config(name, version, true).await
    }

    pub async fn create_model_with_config(&self, name: &str, version: &str, with_config: bool) -> ModelInfo {
        let model_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/common/models/lenet.onnx");
        
        let file_size = std::fs::metadata(&model_path)
            .map(|m| m.len() as i64)
            .ok();

        let metadata = if with_config {
            Some(serde_json::json!({
                "inputs": {
                    "preprocess": [
                        {"type": "resize", "size": [28, 28]},
                        {"type": "normalize", "mean": 0.5, "std": 0.5}
                    ]
                },
                "outputs": {
                    "postprocess": [
                        {"type": "argmax"}
                    ]
                }
            }))
        } else {
            None
        };

        let model = ModelInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            version: version.to_string(),
            file_path: model_path.to_string_lossy().to_string(),
            file_size,
            storage_backend: "local".to_string(),
            input_shapes: Some(serde_json::json!([
                {"name": "import/Placeholder:0", "shape": [1, 1, 28, 28], "element_type": "float32"}
            ])),
            output_shapes: Some(serde_json::json!([
                {"name": "output", "shape": [1, 10], "element_type": "float32"}
            ])),
            metadata,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.db
            .models
            .save(&model)
            .await
            .expect("Failed to create model");
        model
    }

    pub async fn create_task(
        &self,
        model: &ModelInfo,
        user: &User,
        api_key_id: &Uuid,
    ) -> InferenceTask {
        let task = InferenceTask {
            id: Uuid::new_v4(),
            model_id: model.id,
            user_id: user.id,
            api_key_id: *api_key_id,
            status: ferrinx_common::TaskStatus::Pending,
            inputs: serde_json::json!({"input": [1.0, 2.0, 3.0]}),
            outputs: None,
            error_message: None,
            priority: 5,
            retry_count: 0,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
        };

        self.db
            .tasks
            .save(&task)
            .await
            .expect("Failed to create task");
        task
    }

    pub async fn health_check(&self) -> bool {
        self.db.health_check().await.is_ok()
    }
}

fn hash_password(password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn generate_raw_key() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    hex::encode(random_bytes)
}
