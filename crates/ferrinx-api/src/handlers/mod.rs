pub mod admin;
pub mod api_key;
pub mod auth;
pub mod inference;
pub mod model;

use axum::{extract::State, Json};
use crate::{
    dto::{ApiResponse, HealthResponse, ReadyResponse},
    routes::AppState,
};

pub async fn health(
    State(state): State<AppState>,
) -> Json<ApiResponse<HealthResponse>> {
    let uptime_secs = state.start_time.elapsed().as_secs();
    Json(ApiResponse::success(HealthResponse {
        status: "ok".to_string(),
        version: state.config.server.api_version.clone(),
        uptime_secs,
    }))
}

pub async fn ready(
    State(state): State<AppState>,
) -> Json<ApiResponse<ReadyResponse>> {
    let database = state.db.health_check().await.is_ok();
    let redis = state.redis.is_some();
    let engine = state.engine.concurrency_status().available_permits > 0;

    Json(ApiResponse::success(ReadyResponse {
        database,
        redis,
        engine,
    }))
}

pub async fn metrics(
    State(state): State<AppState>,
) -> Json<ApiResponse<serde_json::Value>> {
    let cache_status = state.engine.cache_status().await;
    let concurrency_status = state.engine.concurrency_status();

    Json(ApiResponse::success(serde_json::json!({
        "cache": {
            "loaded_models": cache_status.loaded_models,
            "max_size": cache_status.max_size,
        },
        "concurrency": {
            "available_permits": concurrency_status.available_permits,
            "total_permits": concurrency_status.total_permits,
        }
    })))
}
