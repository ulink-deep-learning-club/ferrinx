use axum::{
    body::Body,
    extract::State,
    http::{Method, Request},
    middleware::Next,
    response::Response,
};
use ferrinx_common::{hash_key, ApiKeyInfo};

use crate::{error::ApiError, routes::AppState};

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let path = req.uri().path();
    if is_public_path(path) {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(ApiError::MissingApiKey)?;

    let api_key = auth_header
        .strip_prefix("Bearer ")
        .ok_or(ApiError::InvalidApiKeyFormat)?;

    let api_key_info = validate_api_key(api_key, &state).await?;

    if !check_permission(&api_key_info, path, req.method()) {
        return Err(ApiError::PermissionDenied);
    }

    req.extensions_mut().insert(api_key_info);

    Ok(next.run(req).await)
}

async fn validate_api_key(key: &str, state: &AppState) -> Result<ApiKeyInfo, ApiError> {
    let key_hash = hash_key(key);

    if let Some(ref redis) = state.redis {
        if let Ok(Some(cached_info)) = redis.get_api_key(&key_hash).await {
            if cached_info.is_valid() {
                return Ok(cached_info);
            }
        }
    }

    if let Some(record) = state.db.api_keys.find_by_hash(&key_hash).await? {
        let info = ApiKeyInfo::from(record);

        if !info.is_valid() {
            return Err(ApiError::InvalidApiKey);
        }

        if let Some(ref redis) = state.redis {
            let _ = redis.set_api_key(&key_hash, &info).await;
        }

        return Ok(info);
    }

    Err(ApiError::InvalidApiKey)
}

fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/health"
            | "/api/v1/ready"
            | "/api/v1/bootstrap"
            | "/api/v1/auth/login"
            | "/api/v1/metrics"
    )
}

fn check_permission(api_key: &ApiKeyInfo, path: &str, method: &Method) -> bool {
    if api_key.permissions.admin {
        return true;
    }

    if path.starts_with("/api/v1/admin") {
        return false;
    }

    if path.starts_with("/api/v1/models") && *method == Method::DELETE {
        return api_key.permissions.can_delete_models();
    }

    if path.starts_with("/api/v1/inference") {
        return api_key.permissions.can_execute_inference();
    }

    true
}
