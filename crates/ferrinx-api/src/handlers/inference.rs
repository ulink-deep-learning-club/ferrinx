use std::collections::HashMap;

use axum::{
    extract::{Multipart, Path, Query, State},
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
pub struct ImageInferRequest {
    pub model_id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImageInferResponse {
    pub result: serde_json::Value,
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

    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }

    let input = ferrinx_common::InferenceInput { inputs: req.inputs };
    let output = state.engine.infer(&model_id, &model.file_path, input).await?;

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

pub async fn image_infer(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<ImageInferResponse>>> {
    if !api_key.permissions.can_execute_inference() {
        return Err(ApiError::PermissionDenied);
    }

    let mut model_id: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut model_version: Option<String> = None;
    let mut image_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ApiError::BadRequest(format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "model_id" => {
                model_id = Some(field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read model_id: {}", e))
                })?);
            }
            "name" => {
                model_name = Some(field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read name: {}", e))
                })?);
            }
            "version" => {
                model_version = Some(field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read version: {}", e))
                })?);
            }
            "image" => {
                image_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("Failed to read image: {}", e)))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let image_data = image_data.ok_or_else(|| ApiError::BadRequest("No image uploaded".to_string()))?;

    let model = if let Some(id) = model_id {
        let model_id = Uuid::parse_str(&id)?;
        state
            .db
            .models
            .find_by_id(&model_id)
            .await?
            .ok_or(ApiError::ModelNotFound)?
    } else if let (Some(name), Some(version)) = (model_name, model_version) {
        state
            .db
            .models
            .find_by_name_version(&name, &version)
            .await?
            .ok_or(ApiError::ModelNotFound)?
    } else {
        return Err(ApiError::BadRequest("Either model_id or name+version is required".to_string()));
    };

    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }

    let model_config: ferrinx_core::model::config::ModelConfig = model.metadata
        .as_ref()
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .ok_or_else(|| ApiError::BadRequest("Model has no preprocessing config".to_string()))?;

    let input_config = model_config.inputs.first()
        .ok_or_else(|| ApiError::BadRequest("Model has no input config".to_string()))?;

    let img = image::load_from_memory(&image_data)
        .map_err(|e| ApiError::BadRequest(format!("Invalid image: {}", e)))?;

    let pipeline = ferrinx_core::PreprocessPipeline::new(input_config.preprocess.clone());
    let preprocessed = pipeline
        .run(ferrinx_core::TransformData::Image(img))
        .map_err(|e| ApiError::BadRequest(format!("Preprocessing failed: {}", e)))?;

    let tensor = preprocessed.into_tensor_f32()
        .map_err(|e| ApiError::BadRequest(format!("Expected tensor after preprocessing: {}", e)))?;

    let input_name = input_config.name.clone();
    let flat_data: Vec<f32> = tensor.iter().cloned().collect();

    let inputs = HashMap::from([(
        input_name,
        serde_json::Value::Array(flat_data.into_iter().map(|v| serde_json::json!(v)).collect()),
    )]);

    let input = ferrinx_common::InferenceInput { inputs };
    let output = state.engine.infer(&model.id, &model.file_path, input).await?;

    let output_config = model_config.outputs.first();
    let result = if let Some(out_cfg) = output_config {
        if !out_cfg.postprocess.is_empty() {
            let output_name = out_cfg.name.clone();
            let output_data = output.outputs.get(&output_name)
                .ok_or_else(|| ApiError::BadRequest(format!("Output {} not found", output_name)))?;

            let tensor_data: Vec<f32> = serde_json::from_value(output_data.clone())
                .map_err(|e| ApiError::BadRequest(format!("Invalid output format: {}", e)))?;

            let tensor = ndarray::ArrayD::from_shape_vec(
                ndarray::IxDyn(&[tensor_data.len()]),
                tensor_data
            ).map_err(|e| ApiError::BadRequest(format!("Tensor creation failed: {}", e)))?;

            let labels = model_config.get_labels().cloned();

            let post_pipeline = ferrinx_core::PostprocessPipeline::new(
                out_cfg.postprocess.clone(),
                labels,
            );

            post_pipeline
                .run(ferrinx_core::TransformData::TensorF32(tensor))
                .map_err(|e| ApiError::BadRequest(format!("Postprocessing failed: {}", e)))?
        } else {
            serde_json::to_value(&output.outputs).map_err(|e| ApiError::BadRequest(format!("JSON error: {}", e)))?
        }
    } else {
        serde_json::to_value(&output.outputs).map_err(|e| ApiError::BadRequest(format!("JSON error: {}", e)))?
    };

    let api_key_id = api_key.id;
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = db.api_keys.update_last_used(&api_key_id).await;
    });

    Ok(Json(ApiResponse::success(ImageInferResponse {
        result,
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

    let redis = state.redis.as_ref().ok_or(ApiError::RedisUnavailable)?;

    let model_id = Uuid::parse_str(&req.model_id)?;
    let model = state
        .db
        .models
        .find_by_id(&model_id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    if !model.is_valid() {
        return Err(ApiError::ModelNotValid);
    }

    let best_worker = redis.get_best_worker_for_model(&model.id).await
        .map_err(|e| ApiError::RedisError(e.to_string()))?;

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

    let worker_id = best_worker.ok_or(ApiError::NoWorkerAvailable)?;
    redis.push_task_to_worker(&worker_id, &task).await
        .map_err(|e| ApiError::RedisError(e.to_string()))?;

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
