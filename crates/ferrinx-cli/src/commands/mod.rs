mod admin;
mod api_key;
mod auth;
mod config_cmd;
mod infer;
mod model;
mod task;

pub use admin::handle_admin;
pub use admin::AdminCommands;
pub use api_key::handle_api_key;
pub use api_key::ApiKeyCommands;
pub use auth::handle_auth;
pub use auth::AuthCommands;
pub use config_cmd::handle_config;
pub use config_cmd::ConfigCommands;
pub use infer::handle_infer;
pub use infer::InferCommands;
pub use model::handle_model;
pub use model::ModelCommands;
pub use task::handle_task;
pub use task::TaskCommands;

use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub api_key: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub permissions: Option<ferrinx_common::Permissions>,
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub permissions: Option<ferrinx_common::Permissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadModelRequest {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadModelResponse {
    pub model_id: uuid::Uuid,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterModelRequest {
    pub file_path: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncInferRequest {
    pub model_id: String,
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncInferResponse {
    pub outputs: HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncInferRequest {
    pub model_id: String,
    pub inputs: HashMap<String, serde_json::Value>,
    pub priority: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsyncInferResponse {
    pub task_id: uuid::Uuid,
    pub status: String,
}

pub fn parse_input(input: &str) -> Result<HashMap<String, serde_json::Value>> {
    if input.starts_with('{') {
        let value: HashMap<String, serde_json::Value> =
            serde_json::from_str(input)?;
        Ok(value)
    } else {
        let content = std::fs::read_to_string(input)
            .map_err(|_| CliError::FileNotFound(input.to_string()))?;
        let value: HashMap<String, serde_json::Value> =
            serde_json::from_str(&content)?;
        Ok(value)
    }
}

pub fn parse_permissions(input: &str) -> Result<ferrinx_common::Permissions> {
    let perms: ferrinx_common::Permissions = serde_json::from_str(input)?;
    Ok(perms)
}
