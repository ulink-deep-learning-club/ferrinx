use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tracing::{error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;

mod consumer;
mod error;
mod maintenance;
mod model_reporter;
mod processor;
mod redis;

use consumer::TaskConsumer;
use error::{Result, WorkerError};
use maintenance::MaintenanceRunner;
use model_reporter::ModelReporter;
use processor::TaskProcessor;
use redis::RedisClient;

struct WorkerContext {
    config: ferrinx_common::Config,
    db: Arc<ferrinx_db::DbContext>,
    redis: Arc<dyn RedisClient>,
    engine: Arc<ferrinx_core::InferenceEngine>,
    storage: Arc<dyn ferrinx_core::ModelStorage>,
    cached_models: model_reporter::CachedModelsRef,
}

impl WorkerContext {
    async fn from_config(config: ferrinx_common::Config) -> Result<Self> {
        let db = Arc::new(ferrinx_db::DbContext::new(&config.database).await?);

        let redis = redis::create_redis_client(&config.redis.url)?;

        let cached_models: model_reporter::CachedModelsRef = 
            Arc::new(std::sync::RwLock::new(std::collections::HashSet::new()));

        let cached_models_clone = cached_models.clone();
        let on_load = Some(std::sync::Arc::new(move |model_id: uuid::Uuid| {
            if let Ok(mut set) = cached_models_clone.write() {
                set.insert(model_id);
            }
        }) as ferrinx_core::CacheLoadCallback);

        let cached_models_clone = cached_models.clone();
        let on_evict = Some(std::sync::Arc::new(move |model_id: uuid::Uuid| {
            if let Ok(mut set) = cached_models_clone.write() {
                set.remove(&model_id);
            }
        }) as ferrinx_core::CacheEvictCallback);

        let engine = Arc::new(
            ferrinx_core::InferenceEngine::new(&config.onnx)?
                .with_callbacks(on_evict, on_load)
        );

        let path = config.storage.path.as_deref().unwrap_or("./models");
        let storage: Arc<dyn ferrinx_core::ModelStorage> = Arc::new(ferrinx_core::LocalStorage::new(path)?);

        Ok(Self {
            config,
            db,
            redis,
            engine,
            storage,
            cached_models,
        })
    }
}

fn generate_consumer_name() -> String {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let pid = std::process::id();
    format!("{}-{}", hostname, pid)
}

fn init_logging(config: &ferrinx_common::LoggingConfig) -> Result<()> {
    let level = config.level.parse().unwrap_or(tracing::Level::INFO);

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false);

    match config.format {
        ferrinx_common::LogFormat::Json => {
            subscriber.json().init();
        }
        ferrinx_common::LogFormat::Text => {
            subscriber.init();
        }
    }

    Ok(())
}

async fn run_worker(
    ctx: Arc<WorkerContext>,
    shutdown: Arc<tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let consumer_name = if ctx.config.worker.consumer_name.is_empty() {
        generate_consumer_name()
    } else {
        ctx.config.worker.consumer_name.clone()
    };

    info!("Starting worker: {}", consumer_name);

    let streams = vec![
        ferrinx_common::constants::REDIS_STREAM_KEY_HIGH.to_string(),
        ferrinx_common::constants::REDIS_STREAM_KEY_NORMAL.to_string(),
        ferrinx_common::constants::REDIS_STREAM_KEY_LOW.to_string(),
    ];

    let consumer = Arc::new(TaskConsumer::new(
        ctx.redis.clone(),
        consumer_name.clone(),
        ctx.config.redis.consumer_group.clone(),
        streams,
    ).with_claim_idle_ms(ctx.config.worker.claim_idle_ms));

    let processor = Arc::new(TaskProcessor::new(
        ctx.db.clone(),
        ctx.redis.clone(),
        ctx.engine.clone(),
        ctx.config.worker.max_retries,
        ctx.config.worker.retry_delay_ms,
    ));

    let model_reporter = Arc::new(
        ModelReporter::new(
            consumer_name.clone(),
            ctx.redis.clone(),
            ctx.storage.clone(),
            ctx.db.clone(),
            ferrinx_common::constants::WORKER_STATUS_REPORT_INTERVAL_SECS,
        )
        .with_cached_models(ctx.cached_models.clone())
    );

    let reporter_token = shutdown.child_token();
    tokio::spawn(async move {
        model_reporter.run(reporter_token).await;
    });

    let current_tasks = Arc::new(AtomicUsize::new(0));
    let poll_interval = Duration::from_millis(ctx.config.worker.poll_interval_ms);

    let maintenance = Arc::new(MaintenanceRunner::new(
        ctx.db.clone(),
        ctx.config.cleanup.completed_task_retention_days,
        ctx.config.cleanup.failed_task_retention_days,
        ctx.config.cleanup.cancelled_task_retention_days,
        ctx.config.cleanup.cleanup_batch_size,
    ));

    let mut maintenance_interval =
        tokio::time::interval(Duration::from_secs(ctx.config.cleanup.cleanup_interval_hours * 3600));

    let mut task_recovery_interval =
        tokio::time::interval(Duration::from_secs(ctx.config.worker.task_recovery_interval_secs));

    let mut health_check_interval =
        tokio::time::interval(Duration::from_secs(ctx.config.worker.health_check_interval_secs));

    let mut consecutive_health_failures = 0u32;
    const MAX_HEALTH_FAILURES: u32 = 3;

    info!("Worker {} started, polling for tasks...", consumer_name);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Shutdown signal received");
                break;
            }

            _ = maintenance_interval.tick() => {
                if ctx.config.cleanup.enabled {
                    info!("Running maintenance tasks...");
                    match maintenance.run_all().await {
                        Ok(stats) => {
                            info!(
                                "Maintenance completed: {} tasks deleted, {} temp keys deleted",
                                stats.tasks_deleted, stats.keys_deleted
                            );
                        }
                        Err(e) => {
                            error!("Maintenance task failed: {}", e);
                        }
                    }
                }
            }

            _ = task_recovery_interval.tick() => {
                match consumer.claim_pending_tasks().await {
                    Ok(tasks) => {
                        if !tasks.is_empty() {
                            for task_message in tasks {
                                let processor = processor.clone();
                                let consumer = consumer.clone();
                                let current_tasks = current_tasks.clone();

                                current_tasks.fetch_add(1, Ordering::Relaxed);

                                tokio::spawn(async move {
                                    let result = processor.process(task_message.clone()).await;

                                    match result {
                                        Ok(()) => {
                                            if let Err(e) = consumer.ack_task(&task_message.stream, &task_message.entry_id).await {
                                                error!("Failed to ACK recovered task: {}", e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Recovered task processing failed: {}", e);
                                        }
                                    }

                                    current_tasks.fetch_sub(1, Ordering::Relaxed);
                                });
                            }
                        }
                    }
                    Err(e) => {
                        error!("Task recovery failed: {}", e);
                    }
                }
            }

            _ = health_check_interval.tick() => {
                match consumer.health_check().await {
                    Ok(()) => {
                        if consecutive_health_failures > 0 {
                            info!("Redis health check recovered after {} failures", consecutive_health_failures);
                        }
                        consecutive_health_failures = 0;
                    }
                    Err(e) => {
                        consecutive_health_failures += 1;
                        error!("Redis health check failed (attempt {}/{}): {}", consecutive_health_failures, MAX_HEALTH_FAILURES, e);

                        if consecutive_health_failures >= MAX_HEALTH_FAILURES {
                            error!("Redis health check failed {} times consecutively, triggering shutdown", MAX_HEALTH_FAILURES);
                            shutdown.cancel();
                        }
                    }
                }
            }

            result = consumer.poll_task() => {
                match result {
                    Ok(Some(task_message)) => {
                        let processor = processor.clone();
                        let consumer = consumer.clone();
                        let current_tasks = current_tasks.clone();
                        let timeout_secs = 300u64;

                        current_tasks.fetch_add(1, Ordering::Relaxed);

                        tokio::spawn(async move {
                            let result = tokio::time::timeout(
                                Duration::from_secs(timeout_secs),
                                processor.process(task_message.clone()),
                            ).await;

                            match result {
                                Ok(Ok(())) => {
                                    if let Err(e) = consumer.ack_task(&task_message.stream, &task_message.entry_id).await {
                                        error!("Failed to ACK task: {}", e);
                                    }
                                }
                                Ok(Err(e)) => {
                                    error!("Task processing failed: {}", e);
                                }
                                Err(_) => {
                                    error!("{}", WorkerError::TaskTimeout);
                                }
                            }

                            current_tasks.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Ok(None) => {
                        tokio::time::sleep(poll_interval).await;
                    }
                    Err(e) => {
                        error!("Failed to poll task: {}", e);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }

    info!("Waiting for {} active tasks to complete...", current_tasks.load(Ordering::Relaxed));

    let shutdown_timeout = Duration::from_secs(ctx.config.server.graceful_shutdown_timeout);
    let start = Instant::now();

    while current_tasks.load(Ordering::Relaxed) > 0 && start.elapsed() < shutdown_timeout {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let remaining = current_tasks.load(Ordering::Relaxed);
    if remaining > 0 {
        warn!("{} tasks still running after shutdown timeout", remaining);
    } else {
        info!("All tasks completed, shutting down");
    }

    Err(WorkerError::Shutdown)
}

fn setup_shutdown_handler() -> Arc<tokio_util::sync::CancellationToken> {
    let token = Arc::new(tokio_util::sync::CancellationToken::new());
    let token_clone = token.clone();

    tokio::spawn(async move {
        let mut sigterm = signal_hook_tokio::Signals::new([signal_hook::consts::SIGTERM])
            .expect("Failed to setup SIGTERM handler");
        let mut sigint = signal_hook_tokio::Signals::new([signal_hook::consts::SIGINT])
            .expect("Failed to setup SIGINT handler");

        tokio::select! {
            _ = sigterm.next() => {
                info!("Received SIGTERM");
            }
            _ = sigint.next() => {
                info!("Received SIGINT");
            }
        }

        token_clone.cancel();
    });

    token
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config.toml");

    let config = if std::path::Path::new(config_path).exists() {
        ferrinx_common::Config::from_file(config_path)?
    } else {
        warn!("Config file not found, using default development config");
        ferrinx_common::Config::default_dev()
    };

    if let Err(errors) = config.validate() {
        for error in errors {
            error!("Configuration error: {}", error);
        }
        std::process::exit(1);
    }

    init_logging(&config.logging)?;

    info!("Ferrinx Worker starting...");
    info!("Config: {:?}", config.worker);

    let ctx = Arc::new(WorkerContext::from_config(config).await?);

    let shutdown = setup_shutdown_handler();

    match run_worker(ctx, shutdown).await {
        Err(WorkerError::Shutdown) => {
            info!("Worker shutdown completed gracefully");
        }
        Err(e) => {
            error!("Worker error: {}", e);
            std::process::exit(1);
        }
        Ok(()) => {}
    }

    info!("Ferrinx Worker stopped");
    Ok(())
}
