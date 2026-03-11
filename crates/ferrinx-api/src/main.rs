use std::sync::Arc;

use ferrinx_api::{
    middleware::rate_limit::RateLimiter,
    routes::{create_router, AppState},
};
use ferrinx_common::Config;
use ferrinx_core::{InferenceEngine, LocalStorage, ModelLoader};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_file("config.toml").unwrap_or_else(|_| {
        info!("Using default development configuration");
        Config::default_dev()
    });

    init_logging(&config);

    if let Err(errors) = config.validate() {
        for error in errors {
            error!("Configuration error: {}", error);
        }
        return Err("Invalid configuration".into());
    }

    info!("Initializing database...");
    let db = Arc::new(ferrinx_db::DbContext::new(&config.database).await?);

    if config.database.run_migrations {
        info!("Database migrations completed");
    }

    let redis = if config.redis.url.is_empty() {
        info!("Redis URL not configured, running without Redis");
        None
    } else {
        info!("Initializing Redis client...");
        let redis_config = ferrinx_common::RedisPoolConfig {
            url: config.redis.url.clone(),
            pool_size: config.redis.pool_size as usize,
            connection_timeout: std::time::Duration::from_secs(5),
            api_key_cache_ttl: config.redis.api_key_cache_ttl,
            result_cache_ttl: config.redis.result_cache_ttl,
            task_timeout_ms: 300000,
        };
        match ferrinx_common::RedisClient::new(redis_config).await {
            Ok(client) => {
                if let Err(e) = client.initialize_consumer_groups().await {
                    error!("Failed to initialize Redis consumer groups: {}", e);
                }
                info!("Redis client initialized successfully");
                Some(Arc::new(client))
            }
            Err(e) => {
                error!("Failed to initialize Redis client: {}", e);
                None
            }
        }
    };

    info!("Initializing inference engine...");
    let engine = Arc::new(InferenceEngine::new(&config.onnx)?);

    info!("Initializing storage...");
    let storage: Arc<dyn ferrinx_core::ModelStorage> = match &config.storage.backend {
        ferrinx_common::StorageBackend::Local => {
            let path = config.storage.path.as_deref().unwrap_or("./models");
            Arc::new(LocalStorage::new(path)?)
        }
        ferrinx_common::StorageBackend::S3 => {
            return Err("S3 storage not yet implemented".into());
        }
    };

    let loader = Arc::new(ModelLoader::new(storage.clone()));

    let rate_limiter = Arc::new(RateLimiter::new(
        config.rate_limit.default_rpm,
        config.rate_limit.cleanup_interval_secs,
    ));

    let cancel_token = CancellationToken::new();

    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        redis,
        engine,
        loader,
        storage,
        rate_limiter,
        cancel_token: cancel_token.clone(),
    };

    let app = create_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel_token))
        .await?;

    info!("Server shutdown complete");
    Ok(())
}

fn init_logging(config: &Config) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.logging.level));

    match config.logging.format {
        ferrinx_common::LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
        ferrinx_common::LogFormat::Text => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
        }
    }
}

async fn shutdown_signal(cancel_token: CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = cancel_token.cancelled() => {},
    }

    info!("Shutdown signal received");
}
