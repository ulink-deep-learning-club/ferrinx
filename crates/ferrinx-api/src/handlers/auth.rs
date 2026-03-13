use axum::{extract::State, Extension, Json};
use chrono::{Duration, Utc};
use rand::RngExt;
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, LoginRequest, LoginResponse},
    error::{ApiError, Result},
    routes::AppState,
};

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>> {
    let user = state
        .db
        .users
        .find_by_username(&req.username)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;

    if !user.is_active {
        return Err(ApiError::InvalidCredentials);
    }

    let password_hash = hash_password(&req.password);
    if user.password_hash != password_hash {
        return Err(ApiError::InvalidCredentials);
    }

    let key_id = Uuid::new_v4();
    let raw_key = generate_session_key();
    let key_hash = ferrinx_common::hash_key(&raw_key);

    let expires_at = Utc::now() + Duration::hours(state.config.auth.temp_key_ttl_hours as i64);

    let api_key = ferrinx_common::ApiKeyRecord {
        id: key_id,
        user_id: user.id,
        key_hash,
        name: format!("session-{}", Utc::now().format("%Y%m%d-%H%M%S")),
        permissions: if user.role == ferrinx_common::UserRole::Admin {
            ferrinx_common::Permissions::admin_default()
        } else {
            ferrinx_common::Permissions::user_default()
        },
        is_active: true,
        is_temporary: true,
        last_used_at: None,
        expires_at: Some(expires_at),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.api_keys.save(&api_key).await?;

    Ok(Json(ApiResponse::success(LoginResponse {
        api_key: raw_key,
        user_id: user.id.to_string(),
        expires_at: Some(expires_at.to_rfc3339()),
    })))
}

pub async fn logout(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
) -> Result<Json<ApiResponse<()>>> {
    if api_key.is_temporary {
        state.db.api_keys.delete(&api_key.id).await?;
    }

    Ok(Json(ApiResponse::success(())))
}

fn generate_session_key() -> String {
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    hex::encode(random_bytes)
}

fn hash_password(password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub async fn bootstrap(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<LoginResponse>>> {
    if state.db.users.exists().await? {
        return Err(ApiError::BadRequest(
            "System already initialized".to_string(),
        ));
    }

    let admin_id = Uuid::new_v4();
    let admin_user = ferrinx_common::User {
        id: admin_id,
        username: "admin".to_string(),
        password_hash: hash_password("admin"),
        role: ferrinx_common::UserRole::Admin,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.users.save(&admin_user).await?;

    let key_id = Uuid::new_v4();
    let raw_key = generate_session_key();
    let key_hash = ferrinx_common::hash_key(&raw_key);

    let api_key = ferrinx_common::ApiKeyRecord {
        id: key_id,
        user_id: admin_id,
        key_hash,
        name: "bootstrap-admin-key".to_string(),
        permissions: ferrinx_common::Permissions::admin_default(),
        is_active: true,
        is_temporary: false,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.api_keys.save(&api_key).await?;

    Ok(Json(ApiResponse::success(LoginResponse {
        api_key: raw_key,
        user_id: admin_id.to_string(),
        expires_at: None,
    })))
}
