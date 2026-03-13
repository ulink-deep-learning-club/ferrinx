use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    dto::{ApiResponse, CreateUserRequest, UpdateUserRequest, UserDetail},
    error::{ApiError, Result},
    routes::AppState,
};

#[derive(Debug, Deserialize)]
pub struct UserFilterQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn create_user(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserDetail>>> {
    if !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    if state
        .db
        .users
        .find_by_username(&req.username)
        .await?
        .is_some()
    {
        return Err(ApiError::UserAlreadyExists);
    }

    let user_id = Uuid::new_v4();
    let password_hash = ferrinx_common::hash_password(&req.password)
        .map_err(|_| ApiError::InternalError)?;

    let role = match req.role.as_deref() {
        Some("admin") => ferrinx_common::UserRole::Admin,
        _ => ferrinx_common::UserRole::User,
    };

    let user = ferrinx_common::User {
        id: user_id,
        username: req.username,
        password_hash,
        role,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    state.db.users.save(&user).await?;

    Ok(Json(ApiResponse::success(UserDetail::from(user))))
}

pub async fn list_users(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Query(filter): Query<UserFilterQuery>,
) -> Result<Json<ApiResponse<Vec<UserDetail>>>> {
    if !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    let users = state.db.users.list(filter.limit, filter.offset).await?;

    Ok(Json(ApiResponse::success(
        users.into_iter().map(UserDetail::from).collect(),
    )))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>> {
    if !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    let id = Uuid::parse_str(&id)?;

    let user = state
        .db
        .users
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::UserNotFound)?;

    // Prevent deleting admin users
    if user.role == ferrinx_common::UserRole::Admin {
        // Check if this is the last admin
        let all_users = state.db.users.list(None, None).await?;
        let admin_count = all_users.iter().filter(|u| u.role == ferrinx_common::UserRole::Admin).count();
        if admin_count <= 1 {
            return Err(ApiError::BadRequest("Cannot delete the last admin user".to_string()));
        }
    }

    // Delete user's API keys first
    state.db.api_keys.delete_by_user(&id).await?;

    // Delete user's tasks
    state.db.tasks.delete_by_user(&id).await?;

    // Delete user
    state.db.users.delete(&id).await?;

    Ok(Json(ApiResponse::success(())))
}

pub async fn update_user(
    State(state): State<AppState>,
    Extension(api_key): Extension<ferrinx_common::ApiKeyInfo>,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<ApiResponse<UserDetail>>> {
    if !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }

    let id = Uuid::parse_str(&id)?;

    let user = state
        .db
        .users
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::UserNotFound)?;

    let mut updates = ferrinx_common::UserUpdates::default();

    if let Some(username) = req.username {
        if username != user.username {
            if state.db.users.find_by_username(&username).await?.is_some() {
                return Err(ApiError::UserAlreadyExists);
            }
            updates.username = Some(username);
        }
    }

    if let Some(password) = req.password {
        updates.password_hash = Some(
            ferrinx_common::hash_password(&password)
                .map_err(|_| ApiError::InternalError)?
        );
    }

    if let Some(role_str) = req.role {
        let role = match role_str.as_str() {
            "admin" => ferrinx_common::UserRole::Admin,
            _ => ferrinx_common::UserRole::User,
        };
        updates.role = Some(role);
    }

    if let Some(is_active) = req.is_active {
        updates.is_active = Some(is_active);
    }

    state.db.users.update(&id, &updates).await?;

    let updated_user = state
        .db
        .users
        .find_by_id(&id)
        .await?
        .ok_or(ApiError::UserNotFound)?;

    Ok(Json(ApiResponse::success(UserDetail::from(updated_user))))
}
