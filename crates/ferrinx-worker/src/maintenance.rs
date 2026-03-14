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
