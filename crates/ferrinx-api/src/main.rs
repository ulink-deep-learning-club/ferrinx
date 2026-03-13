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

    println!("{}", format_config(&config));

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
    let path = config.storage.path.as_deref().unwrap_or("./models");
    let storage: Arc<dyn ferrinx_core::ModelStorage> = Arc::new(LocalStorage::new(path)?);

    let loader = Arc::new(ModelLoader::new(storage.clone()));

    let rate_limiter = Arc::new(RateLimiter::new(
        config.rate_limit.default_rpm,
        config.rate_limit.cleanup_interval_secs,
    ));

    let cancel_token = CancellationToken::new();
    let start_time = std::time::Instant::now();

    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        redis,
        engine,
        loader,
        storage,
        rate_limiter,
        cancel_token: cancel_token.clone(),
        start_time,
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

fn format_config(config: &Config) -> String {
    let mut output = String::from("Loaded configuration:\n");

    output.push_str(&format!("  Server:\n"));
    output.push_str(&format!("    host: {}\n", config.server.host));
    output.push_str(&format!("    port: {}\n", config.server.port));

    output.push_str(&format!("  Database:\n"));
    output.push_str(&format!("    backend: {:?}\n", config.database.backend));
    output.push_str(&format!("    url: {}\n", mask_secrets("url", &config.database.url)));

    output.push_str(&format!("  Storage:\n"));
    output.push_str(&format!("    backend: {:?}\n", config.storage.backend));
    output.push_str(&format!("    path: {}\n", config.storage.path.as_deref().unwrap_or("default")));

    output.push_str(&format!("  ONNX:\n"));
    output.push_str(&format!("    execution_provider: {:?}\n", config.onnx.execution_provider));

    output.push_str(&format!("  Auth:\n"));
    output.push_str(&format!("    api_key_secret: {}\n", mask_secrets("secret", &config.auth.api_key_secret)));

    output.push_str(&format!("  Redis:\n"));
    output.push_str(&format!("    url: {}\n",
        if config.redis.url.is_empty() { "(not configured)".to_string() }
        else { mask_secrets("url", &config.redis.url) }
    ));

    output.trim_end().to_string()
}

fn mask_secrets(field_name: &str, value: &str) -> String {
    if field_name.contains("secret") {
        "*".repeat(8)
    } else if field_name == "url" && value.contains('@') {
        value.split('@').last().map(|host| format!("***@{}", host)).unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    }
}
