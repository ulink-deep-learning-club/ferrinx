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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    fn create_test_api_key(admin: bool, can_delete: bool, can_execute: bool) -> ApiKeyInfo {
        let mut perms = ferrinx_common::Permissions::default();
        perms.admin = admin;
        if can_delete {
            perms.models.push("delete".to_string());
        }
        if can_execute {
            perms.inference.push("execute".to_string());
        }
        ApiKeyInfo {
            id: uuid::Uuid::nil(),
            user_id: uuid::Uuid::nil(),
            name: "test".to_string(),
            permissions: perms,
            is_active: true,
            is_temporary: false,
            expires_at: None,
        }
    }

    #[test]
    fn test_is_public_path() {
        assert!(is_public_path("/api/v1/health"));
        assert!(is_public_path("/api/v1/ready"));
        assert!(is_public_path("/api/v1/bootstrap"));
        assert!(is_public_path("/api/v1/auth/login"));
        assert!(is_public_path("/api/v1/metrics"));
        assert!(!is_public_path("/api/v1/models"));
        assert!(!is_public_path("/api/v1/inference"));
        assert!(!is_public_path("/api/v1/admin/users"));
    }

    #[test]
    fn test_check_permission_admin() {
        let api_key = create_test_api_key(true, false, false);
        assert!(check_permission(&api_key, "/api/v1/models", &Method::GET));
        assert!(check_permission(&api_key, "/api/v1/models/123", &Method::DELETE));
        assert!(check_permission(&api_key, "/api/v1/inference", &Method::POST));
        assert!(check_permission(&api_key, "/api/v1/admin/users", &Method::GET));
    }

    #[test]
    fn test_check_permission_non_admin_admin_path() {
        let api_key = create_test_api_key(false, true, true);
        assert!(!check_permission(&api_key, "/api/v1/admin/users", &Method::GET));
    }

    #[test]
    fn test_check_permission_delete_model() {
        let api_key_with_delete = create_test_api_key(false, true, false);
        let api_key_without_delete = create_test_api_key(false, false, false);
        
        assert!(check_permission(&api_key_with_delete, "/api/v1/models/123", &Method::DELETE));
        assert!(!check_permission(&api_key_without_delete, "/api/v1/models/123", &Method::DELETE));
    }

    #[test]
    fn test_check_permission_inference() {
        let api_key_with_inference = create_test_api_key(false, false, true);
        let api_key_without_inference = create_test_api_key(false, false, false);
        
        assert!(check_permission(&api_key_with_inference, "/api/v1/inference", &Method::POST));
        assert!(!check_permission(&api_key_without_inference, "/api/v1/inference", &Method::POST));
    }

    #[test]
    fn test_check_permission_read_models() {
        let api_key = create_test_api_key(false, false, false);
        assert!(check_permission(&api_key, "/api/v1/models", &Method::GET));
        assert!(check_permission(&api_key, "/api/v1/models/123", &Method::GET));
    }

    #[test]
    fn test_check_permission_write_models() {
        let api_key = create_test_api_key(false, false, false);
        assert!(check_permission(&api_key, "/api/v1/models", &Method::POST));
        assert!(check_permission(&api_key, "/api/v1/models/123", &Method::PUT));
    }
}
