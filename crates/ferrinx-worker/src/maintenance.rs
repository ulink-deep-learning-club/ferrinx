use std::sync::Arc;

use tracing::info;

use ferrinx_db::DbContext;

use crate::error::Result;

pub struct MaintenanceRunner {
    db: Arc<DbContext>,
    completed_retention_days: u32,
    failed_retention_days: u32,
    cancelled_retention_days: u32,
    batch_size: usize,
}

impl MaintenanceRunner {
    pub fn new(
        db: Arc<DbContext>,
        completed_retention_days: u32,
        failed_retention_days: u32,
        cancelled_retention_days: u32,
        batch_size: usize,
    ) -> Self {
        Self {
            db,
            completed_retention_days,
            failed_retention_days,
            cancelled_retention_days,
            batch_size,
        }
    }

    pub async fn cleanup_expired_tasks(&self) -> Result<u64> {
        let mut total_deleted = 0u64;

        for (status, days) in [
            (
                ferrinx_common::TaskStatus::Completed,
                self.completed_retention_days,
            ),
            (
                ferrinx_common::TaskStatus::Failed,
                self.failed_retention_days,
            ),
            (
                ferrinx_common::TaskStatus::Cancelled,
                self.cancelled_retention_days,
            ),
        ] {
            let deleted = self.db.tasks.cleanup_expired(days, self.batch_size).await?;
            if deleted > 0 {
                info!("Cleaned up {} expired {:?} tasks", deleted, status);
            }
            total_deleted += deleted;
        }

        if total_deleted > 0 {
            info!("Total cleaned up {} expired tasks", total_deleted);
        }

        Ok(total_deleted)
    }

    pub async fn cleanup_expired_temp_keys(&self) -> Result<u64> {
        let deleted = self.db.api_keys.cleanup_expired_temp_keys().await?;

        if deleted > 0 {
            info!("Cleaned up {} expired temporary API keys", deleted);
        }

        Ok(deleted)
    }

    pub async fn run_all(&self) -> Result<MaintenanceStats> {
        let tasks_deleted = self.cleanup_expired_tasks().await?;
        let keys_deleted = self.cleanup_expired_temp_keys().await?;

        Ok(MaintenanceStats {
            tasks_deleted,
            keys_deleted,
        })
    }
}

#[derive(Debug, Default)]
pub struct MaintenanceStats {
    pub tasks_deleted: u64,
    pub keys_deleted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maintenance_runner_new() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = ferrinx_common::DatabaseConfig {
                backend: ferrinx_common::DatabaseBackend::Sqlite,
                url: ":memory:".to_string(),
                max_connections: 1,
                run_migrations: true,
            };
            let db = Arc::new(DbContext::new(&config).await.unwrap());

            let runner = MaintenanceRunner::new(db, 30, 90, 7, 100);

            assert_eq!(runner.completed_retention_days, 30);
            assert_eq!(runner.failed_retention_days, 90);
            assert_eq!(runner.cancelled_retention_days, 7);
            assert_eq!(runner.batch_size, 100);
        });
    }

    #[tokio::test]
    async fn test_cleanup_expired_tasks_empty_db() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());

        let runner = MaintenanceRunner::new(db, 30, 90, 7, 100);

        let deleted = runner.cleanup_expired_tasks().await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_cleanup_expired_temp_keys_empty_db() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());

        let runner = MaintenanceRunner::new(db, 30, 90, 7, 100);

        let deleted = runner.cleanup_expired_temp_keys().await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_run_all_empty_db() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());

        let runner = MaintenanceRunner::new(db, 30, 90, 7, 100);

        let stats = runner.run_all().await.unwrap();
        assert_eq!(stats.tasks_deleted, 0);
        assert_eq!(stats.keys_deleted, 0);
    }

    #[test]
    fn test_maintenance_stats_default() {
        let stats = MaintenanceStats::default();
        assert_eq!(stats.tasks_deleted, 0);
        assert_eq!(stats.keys_deleted, 0);
    }

    #[test]
    fn test_maintenance_stats_debug() {
        let stats = MaintenanceStats {
            tasks_deleted: 10,
            keys_deleted: 5,
        };

        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("tasks_deleted: 10"));
        assert!(debug_str.contains("keys_deleted: 5"));
    }

    #[tokio::test]
    async fn test_cleanup_with_different_retention_days() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());

        let runner = MaintenanceRunner::new(db, 7, 14, 3, 50);

        assert_eq!(runner.completed_retention_days, 7);
        assert_eq!(runner.failed_retention_days, 14);
        assert_eq!(runner.cancelled_retention_days, 3);
        assert_eq!(runner.batch_size, 50);
    }

    #[tokio::test]
    async fn test_cleanup_with_small_batch_size() {
        let config = ferrinx_common::DatabaseConfig {
            backend: ferrinx_common::DatabaseBackend::Sqlite,
            url: ":memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        };
        let db = Arc::new(DbContext::new(&config).await.unwrap());

        let runner = MaintenanceRunner::new(db, 30, 90, 7, 10);

        let deleted = runner.cleanup_expired_tasks().await.unwrap();
        assert_eq!(deleted, 0);
    }
}
