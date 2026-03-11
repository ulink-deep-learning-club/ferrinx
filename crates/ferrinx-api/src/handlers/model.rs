use axum::{
    extract::{Multipart, Path, Query, State},
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, ModelDetail},
    error::{ApiError, Result},
    routes::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ModelFilterQuery {
    pub name: Option<String>,
    pub is_valid: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterModelRequest {
    pub name: String,
    pub version: String,
    pub file_path: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateModelRequest {
    pub name: Option<String>,
    pub version: Option<String>,
}

pub async fn upload(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<ModelDetail>>> {
    if !api_key.permissions.can_write_models() {
        return Err(ApiError::PermissionDenied);
    }

    let mut model_name = String::new();
    let mut model_version = String::new();
    let mut model_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ApiError::BadRequest(format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "name" => {
                model_name = field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read name: {}", e))
                })?;
            }
            "version" => {
                model_version = field.text().await.map_err(|e| {
                    ApiError::BadRequest(format!("Failed to read version: {}", e))
                })?;
            }
            "file" => {
                model_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("Failed to read file: {}", e)))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    if model_name.is_empty() || model_version.is_empty() {
        return Err(ApiError::BadRequest("Missing name or version".to_string()));
    }

    let model_data = model_data.ok_or_else(|| ApiError::BadRequest("No file uploaded".to_string()))?;

    if state
        .db
        .models
        .exists(&model_name, &model_version)
        .await?
    {
        return Err(ApiError::BadRequest(format!(
            "Model {}:{} already exists",
            model_name, model_version
        )));
    }

    let model_id = Uuid::new_v4();
    let model_id_str = model_id.to_string();
    let file_path = state.storage.save(&model_id_str, &model_data).await?;

    match state.loader.validate_model(&model_data).await {
        Ok(metadata) => {
            let model = ferrinx_common::ModelInfo {
                id: model_id,
                name: model_name.clone(),
                version: model_version.clone(),
                file_path: file_path.clone(),
                file_size: Some(model_data.len() as i64),
                storage_backend: "local".to_string(),
                input_shapes: Some(serde_json::to_value(metadata.inputs)?),
                output_shapes: Some(serde_json::to_value(metadata.outputs)?),
                metadata: None,
                is_valid: true,
                validation_error: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            state.db.models.save(&model).await?;

            return Ok(Json(ApiResponse::success(ModelDetail::from(model))));
        }
        Err(e) => {
            let model = ferrinx_common::ModelInfo {
                id: model_id,
                name: model_name,
                version: model_version,
                file_path,
                file_size: Some(model_data.len() as i64),
                storage_backend: "local".to_string(),
                input_shapes: None,
                output_shapes: None,
                metadata: None,
                is_valid: false,
                validation_error: Some(e.to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            state.db.models.save(&model).await?;

            return Ok(Json(ApiResponse::success(ModelDetail::from(model))));
        }
    }
}

pub async fn register(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Json(req): Json<RegisterModelRequest>,
) -> Result<Json<ApiResponse<ModelDetail>>> {
    if !api_key.permissions.can_write_models() {
        return Err(ApiError::PermissionDenied);
    }

    if state.db.models.exists(&req.name, &req.version).await? {
        return Err(ApiError::BadRequest(format!(
            "Model {}:{} already exists",
            req.name, req.version
        )));
    }

    let model_id = Uuid::new_v4();
    let model_data = state.storage.load(&req.file_path).await?;

    let is_valid;
    let validation_error;
    let input_shapes;
    let output_shapes;

    match state.loader.validate_model(&model_data).await {
        Ok(metadata) => {
            is_valid = true;
            validation_error = None;
            input_shapes = Some(serde_json::to_value(metadata.inputs)?);
            output_shapes = Some(serde_json::to_value(metadata.outputs)?);
        }
        Err(e) => {
            is_valid = false;
            validation_error = Some(e.to_string());
            input_shapes = None;
            output_shapes = None;
        }
    }

    let model = ferrinx_common::ModelInfo {
        id: model_id,
        name: req.name,
        version: req.version,
        file_path: req.file_path,
        file_size: Some(model_data.len() as i64),
        storage_backend: "local".to_string(),
        input_shapes,
        output_shapes,
        metadata: req.metadata,
        is_valid,
        validation_error,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.models.save(&model).await?;

    Ok(Json(ApiResponse::success(ModelDetail::from(model))))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Query(filter): Query<ModelFilterQuery>,
) -> Result<Json<ApiResponse<Vec<ModelDetail>>>> {
    if !api_key.permissions.can_read_models() {
        return Err(ApiError::PermissionDenied);
    }

    let filter = ferrinx_common::ModelFilter {
        name: filter.name,
        is_valid: filter.is_valid,
        limit: filter.limit,
        offset: filter.offset,
    };

    let models = state.db.models.list(&filter).await?;

    Ok(Json(ApiResponse::success(
        models.into_iter().map(ModelDetail::from).collect(),
    )))
}

pub async fn get(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ModelDetail>>> {
    if !api_key.permissions.can_read_models() {
        return Err(ApiError::PermissionDenied);
    }

    let id = Uuid::parse_str(&id)?;
    let model = state
        .db
        .models
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    Ok(Json(ApiResponse::success(ModelDetail::from(model))))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>> {
    if !api_key.permissions.can_delete_models() {
        return Err(ApiError::PermissionDenied);
    }

    let id = Uuid::parse_str(&id)?;
    let model = state
        .db
        .models
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    state.storage.delete(&model.file_path).await?;
    state.db.models.delete(&id).await?;

    Ok(Json(ApiResponse::success(())))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
    Json(req): Json<UpdateModelRequest>,
) -> Result<Json<ApiResponse<ModelDetail>>> {
    if !api_key.permissions.can_write_models() {
        return Err(ApiError::PermissionDenied);
    }

    let id = Uuid::parse_str(&id)?;
    let mut model = state
        .db
        .models
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::ModelNotFound)?;

    if let Some(name) = req.name {
        if name != model.name {
            if state.db.models.exists(&name, &model.version).await? {
                return Err(ApiError::BadRequest(format!(
                    "Model {}:{} already exists",
                    name, model.version
                )));
            }
            model.name = name;
        }
    }

    if let Some(version) = req.version {
        if version != model.version {
            if state.db.models.exists(&model.name, &version).await? {
                return Err(ApiError::BadRequest(format!(
                    "Model {}:{} already exists",
                    model.name, version
                )));
            }
            model.version = version;
        }
    }

    model.updated_at = Utc::now();
    state.db.models.save(&model).await?;

    Ok(Json(ApiResponse::success(ModelDetail::from(model))))
}
