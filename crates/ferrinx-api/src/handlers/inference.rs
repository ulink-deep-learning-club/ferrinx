use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, TaskDetail},
    error::{ApiError, Result},
    routes::AppState,
};

#[derive(Debug, Deserialize)]
pub struct SyncInferRequest {
    pub model_id: String,
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SyncInferResponse {
    pub outputs: HashMap<String, serde_json::Value>,
    pub latency_ms: u64,
}

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

fn default_priority() -> String {
    "normal".to_string()
}
fn default_timeout() -> u32 {
    300
}

#[derive(Debug, Serialize)]
pub struct AsyncInferResponse {
    pub task_id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskFilterQuery {
    pub model_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn sync_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Json(req): Json<SyncInferRequest>,
) -> Result<Json<ApiResponse<SyncInferResponse>>> {
    if !api_key.permissions.can_execute_inference() {
        return Err(ApiError::PermissionDenied);
    }

    let model_id = Uuid::parse_str(&req.model_id)?;
    let model = state
        .db
        .models
        .find_by_id(&model_id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    if !model.is_valid {
        return Err(ApiError::ModelNotValid);
    }

    let input = ferrinx_common::InferenceInput { inputs: req.inputs };
    let output = state.engine.infer(&req.model_id, &model.file_path, input).await?;

    let api_key_id = api_key.id;
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = db.api_keys.update_last_used(&api_key_id).await;
    });

    Ok(Json(ApiResponse::success(SyncInferResponse {
        outputs: output.outputs,
        latency_ms: output.latency_ms,
    })))
}

pub async fn async_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Json(req): Json<AsyncInferRequest>,
) -> Result<Json<ApiResponse<AsyncInferResponse>>> {
    if !api_key.permissions.can_execute_inference() {
        return Err(ApiError::PermissionDenied);
    }

    if state.redis.is_none() {
        return Err(ApiError::RedisUnavailable);
    }

    let model_id = Uuid::parse_str(&req.model_id)?;
    let model = state
        .db
        .models
        .find_by_id(&model_id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    if !model.is_valid {
        return Err(ApiError::ModelNotValid);
    }

    let task_id = Uuid::new_v4();
    let task = ferrinx_common::InferenceTask {
        id: task_id,
        model_id: model.id,
        user_id: api_key.user_id,
        api_key_id: api_key.id,
        status: ferrinx_common::TaskStatus::Pending,
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

    state.db.tasks.save(&task).await?;

    if let Some(ref redis) = state.redis {
        redis.push_task(&task).await?;
    }

    Ok(Json(ApiResponse::success(AsyncInferResponse {
        task_id: task_id.to_string(),
        status: "pending".to_string(),
    })))
}

pub async fn get_task(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(task_id): Path<String>,
) -> Result<Json<ApiResponse<TaskDetail>>> {
    let task_id = Uuid::parse_str(&task_id)?;

    let task = state
        .db
        .tasks
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::TaskNotFound)?;

    if task.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    Ok(Json(ApiResponse::success(TaskDetail::from(task))))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(task_id): Path<String>,
) -> Result<Json<ApiResponse<()>>> {
    let task_id = Uuid::parse_str(&task_id)?;

    let task = state
        .db
        .tasks
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::TaskNotFound)?;

    if task.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    if task.status != ferrinx_common::TaskStatus::Pending {
        return Err(ApiError::TaskNotCancellable);
    }

    state
        .db
        .tasks
        .update_status(&task_id, ferrinx_common::TaskStatus::Cancelled)
        .await?;

    Ok(Json(ApiResponse::success(())))
}

pub async fn list_tasks(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Query(filter): Query<TaskFilterQuery>,
) -> Result<Json<ApiResponse<Vec<TaskDetail>>>> {
    let filter = ferrinx_common::TaskFilter {
        user_id: Some(api_key.user_id),
        model_id: filter
            .model_id
            .and_then(|s| Uuid::parse_str(&s).ok()),
        status: filter
            .status
            .and_then(|s| ferrinx_common::TaskStatus::from_str(&s)),
        limit: filter.limit,
        offset: filter.offset,
    };

    let tasks = state.db.tasks.list(&filter).await?;

    Ok(Json(ApiResponse::success(
        tasks.into_iter().map(TaskDetail::from).collect(),
    )))
}
