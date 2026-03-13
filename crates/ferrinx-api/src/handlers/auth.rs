use axum::{extract::State, Extension, Json};
use chrono::{Duration, Utc};
use rand::RngExt;
use tracing::warn;
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, BootstrapResponse, LoginRequest, LoginResponse},
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

    if !ferrinx_common::verify_password(&req.password, &user.password_hash)
        .map_err(|_| ApiError::InvalidCredentials)?
    {
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

pub async fn bootstrap(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<BootstrapResponse>>> {
    if state.db.users.exists().await? {
        return Err(ApiError::BadRequest(
            "System already initialized".to_string(),
        ));
    }

    // Generate a secure random password for bootstrap admin
    let admin_password = ferrinx_common::generate_secure_password(16);
    let password_hash = ferrinx_common::hash_password(&admin_password)
        .map_err(|_| ApiError::InternalError)?;

    let admin_id = Uuid::new_v4();
    let admin_username = "admin".to_string();
    let admin_user = ferrinx_common::User {
        id: admin_id,
        username: admin_username.clone(),
        password_hash,
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

    // Log warning about bootstrap being used
    warn!(
        user_id = %admin_id,
        username = %admin_username,
        "SECURITY WARNING: Bootstrap endpoint was used to create admin user. Ensure this is intentional and change the password immediately."
    );

    Ok(Json(ApiResponse::success(BootstrapResponse {
        user_id: admin_id.to_string(),
        username: admin_username,
        password: admin_password,
        api_key: raw_key,
    })))
}
