use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::State,
    http::Request,
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{error::ApiError, routes::AppState};

pub struct RateLimiter {
    limits: Arc<RwLock<HashMap<String, RateLimitEntry>>>,
    #[allow(dead_code)]
    default_limit: u32,
    #[allow(dead_code)]
    cleanup_interval: Duration,
}

struct RateLimitEntry {
    count: AtomicU64,
    reset_at: Instant,
}

impl RateLimiter {
    pub fn new(default_limit: u32, cleanup_interval_secs: u64) -> Self {
        Self {
            limits: Arc::new(RwLock::new(HashMap::new())),
            default_limit,
            cleanup_interval: Duration::from_secs(cleanup_interval_secs),
        }
    }

    pub async fn check(&self, key: &str, limit: u32) -> Result<bool, ApiError> {
        let now = Instant::now();
        let minute_ago = now - Duration::from_secs(60);

        {
            let mut limits = self.limits.write().await;
            limits.retain(|_, entry| entry.reset_at > minute_ago);
        }

        let limits = self.limits.read().await;
        if let Some(entry) = limits.get(key) {
            let count = entry.count.load(Ordering::Relaxed);
            if count >= limit as u64 {
                return Ok(false);
            }
        }
        drop(limits);

        let mut limits = self.limits.write().await;
        let entry = limits.entry(key.to_string()).or_insert(RateLimitEntry {
            count: AtomicU64::new(0),
            reset_at: now + Duration::from_secs(60),
        });

        entry.count.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }
}

pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if !state.config.rate_limit.enabled {
        return Ok(next.run(req).await);
    }

    let api_key_info = req
        .extensions()
        .get::<ferrinx_common::ApiKeyInfo>()
        .cloned();

    let key = if let Some(info) = api_key_info {
        format!("rate_limit:{}", info.id)
    } else {
        let ip = req
            .headers()
            .get("X-Real-IP")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown");
        format!("rate_limit:ip:{}", ip)
    };

    let limit = get_rate_limit(req.uri().path(), &state.config.rate_limit);

    let allowed = state.rate_limiter.check(&key, limit).await?;

    if !allowed {
        return Err(ApiError::RateLimitExceeded);
    }

    Ok(next.run(req).await)
}

fn get_rate_limit(path: &str, config: &ferrinx_common::RateLimitConfig) -> u32 {
    if path.starts_with("/api/v1/inference/sync") {
        config.sync_inference_rpm
    } else if path.starts_with("/api/v1/inference") {
        config.async_inference_rpm
    } else {
        config.default_rpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(5, 60);

        for i in 0..5 {
            let result = limiter.check("test_key", 5).await.unwrap();
            assert!(result, "Request {} should be allowed", i + 1);
        }

        let result = limiter.check("test_key", 5).await.unwrap();
        assert!(!result, "Request 6 should be denied");
    }
}
