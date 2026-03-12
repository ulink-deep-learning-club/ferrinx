use std::sync::Arc;

use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tokio_util::sync::CancellationToken;

use crate::{handlers, middleware as mw};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ferrinx_common::Config>,
    pub db: Arc<ferrinx_db::DbContext>,
    pub redis: Option<Arc<ferrinx_common::RedisClient>>,
    pub engine: Arc<ferrinx_core::InferenceEngine>,
    pub loader: Arc<ferrinx_core::ModelLoader>,
    pub storage: Arc<dyn ferrinx_core::ModelStorage>,
    pub rate_limiter: Arc<crate::middleware::rate_limit::RateLimiter>,
    pub cancel_token: CancellationToken,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(handlers::health))
        .route("/api/v1/ready", get(handlers::ready))
        .route("/api/v1/metrics", get(handlers::metrics))
        .route("/api/v1/bootstrap", post(handlers::auth::bootstrap))
        .route("/api/v1/auth/login", post(handlers::auth::login))
        .route("/api/v1/auth/logout", post(handlers::auth::logout))
        .nest("/api/v1/admin", admin_routes())
        .nest("/api/v1/api-keys", api_key_routes())
        .nest("/api/v1/models", model_routes())
        .route(
            "/api/v1/inference/sync",
            post(handlers::inference::sync_infer),
        )
        .route(
            "/api/v1/inference/image",
            post(handlers::inference::image_infer),
        )
        .route("/api/v1/inference", post(handlers::inference::async_infer))
        .route("/api/v1/inference/{id}", get(handlers::inference::get_task))
        .route(
            "/api/v1/inference/{id}",
            delete(handlers::inference::cancel_task),
        )
        .route("/api/v1/inference", get(handlers::inference::list_tasks))
        .layer(middleware::from_fn(mw::logging_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            mw::rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            mw::auth_middleware,
        ))
        .with_state(state)
}

fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/users", post(handlers::admin::create_user))
        .route("/users", get(handlers::admin::list_users))
        .route("/users/{id}", delete(handlers::admin::delete_user))
        .route("/users/{id}", put(handlers::admin::update_user))
}

fn api_key_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(handlers::api_key::create))
        .route("/", get(handlers::api_key::list))
        .route("/{id}", get(handlers::api_key::get))
        .route("/{id}", delete(handlers::api_key::revoke))
        .route("/{id}", put(handlers::api_key::update))
}

fn model_routes() -> Router<AppState> {
    Router::new()
        .route("/upload", post(handlers::model::upload))
        .route("/register", post(handlers::model::register))
        .route("/", get(handlers::model::list))
        .route("/{id}", get(handlers::model::get))
        .route("/{id}", delete(handlers::model::delete))
        .route("/{id}", put(handlers::model::update))
        .route(
            "/{name}/{version}",
            get(handlers::model::get_by_name_version),
        )
        .route(
            "/{name}/{version}",
            delete(handlers::model::delete_by_name_version),
        )
}
