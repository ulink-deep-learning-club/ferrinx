use axum::{
    extract::{Path, State},
    Extension, Json,
};
use chrono::{Duration, Utc};
use rand::RngExt;
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, ApiKeyDetail, CreateApiKeyRequest, CreateApiKeyResponse, UpdateApiKeyRequest},
    error::{ApiError, Result},
    routes::AppState,
};

fn generate_api_key() -> String {
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    hex::encode(random_bytes)
}

pub async fn create(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiResponse<CreateApiKeyResponse>>> {
    let existing_keys = state.db.api_keys.find_by_user(&api_key.user_id).await?;

    if existing_keys.len() >= state.config.auth.max_keys_per_user {
        return Err(ApiError::ApiKeyLimitExceeded);
    }

    let key_id = Uuid::new_v4();
    let raw_key = generate_api_key();
    let key_hash = ferrinx_common::hash_key(&raw_key);

    let expires_at = req.expires_in_hours.map(|hours| {
        Utc::now() + Duration::hours(hours as i64)
    });

    let permissions = req.permissions.unwrap_or_else(ferrinx_common::Permissions::user_default);

    let api_key_record = ferrinx_common::ApiKeyRecord {
        id: key_id,
        user_id: api_key.user_id,
        key_hash,
        name: req.name.clone(),
        permissions,
        is_active: true,
        is_temporary: expires_at.is_some(),
        last_used_at: None,
        expires_at,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.api_keys.save(&api_key_record).await?;

    Ok(Json(ApiResponse::success(CreateApiKeyResponse {
        id: key_id.to_string(),
        key: raw_key,
        name: req.name,
    })))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
) -> Result<Json<ApiResponse<Vec<ApiKeyDetail>>>> {
    let keys = state.db.api_keys.find_by_user(&api_key.user_id).await?;

    Ok(Json(ApiResponse::success(
        keys.into_iter().map(ApiKeyDetail::from).collect(),
    )))
}

pub async fn get(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ApiKeyDetail>>> {
    let id = Uuid::parse_str(&id)?;
    let key = state
        .db
        .api_keys
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::BadRequest("API key not found".to_string()))?;

    if key.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    Ok(Json(ApiResponse::success(ApiKeyDetail::from(key))))
}

pub async fn revoke(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>> {
    let id = Uuid::parse_str(&id)?;
    let key = state
        .db
        .api_keys
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::BadRequest("API key not found".to_string()))?;

    if key.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    state.db.api_keys.delete(&id).await?;

    Ok(Json(ApiResponse::success(())))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiResponse<ApiKeyDetail>>> {
    let id = Uuid::parse_str(&id)?;
    let mut key = state
        .db
        .api_keys
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::BadRequest("API key not found".to_string()))?;

    if key.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    if let Some(name) = req.name {
        key.name = name;
    }

    if let Some(permissions) = req.permissions {
        key.permissions = permissions;
    }

    key.updated_at = Utc::now();

    state.db.api_keys.save(&key).await?;

    Ok(Json(ApiResponse::success(ApiKeyDetail::from(key))))
}
