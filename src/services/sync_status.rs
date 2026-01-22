//! Sync status service for tracking last successful sync times
//!
//! This service prevents redundant API calls on restart by tracking when
//! each sync job last ran successfully.

#![allow(dead_code)]

use chrono::{Duration, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entities::sync_status::{self, Entity as SyncStatus};

/// Job names for tracking sync status
pub mod jobs {
    pub const ALL_COINGECKO_COINS: &str = "all_coingecko_coins_sync";
    pub const COINS_HISTORICAL_PRICES: &str = "coins_historical_prices_sync";
    pub const COINS_LOGO_SYNC: &str = "coins_logo_sync";
    pub const CATEGORY_SYNC: &str = "category_sync";
    pub const CATEGORY_MEMBERSHIP: &str = "category_membership_sync";
    pub const ANNOUNCEMENT_SCRAPER: &str = "announcement_scraper";
    pub const INDEX_DAILY_PRICES: &str = "index_daily_prices_sync";
    pub const REBALANCE_SYNC: &str = "rebalance_sync";
    pub const BITGET_HISTORICAL_PRICES: &str = "bitget_historical_prices_sync";
}

/// Default minimum intervals between syncs (in seconds)
pub mod intervals {
    pub const ALL_COINGECKO_COINS: i32 = 21600;      // 6 hours
    pub const COINS_HISTORICAL_PRICES: i32 = 21600;  // 6 hours
    pub const COINS_LOGO_SYNC: i32 = 86400;          // 24 hours (logos rarely change)
    pub const CATEGORY_SYNC: i32 = 21600;            // 6 hours
    pub const CATEGORY_MEMBERSHIP: i32 = 21600;      // 6 hours
    pub const ANNOUNCEMENT_SCRAPER: i32 = 3600;      // 1 hour
    pub const INDEX_DAILY_PRICES: i32 = 3600;        // 1 hour
    pub const REBALANCE_SYNC: i32 = 3600;            // 1 hour
    pub const BITGET_HISTORICAL_PRICES: i32 = 86400; // 24 hours (daily update)
}

/// Check if a sync job should run based on last successful sync time
///
/// Returns true if:
/// - No record exists for this job (first run)
/// - Last successful sync was more than min_interval_secs ago
pub async fn should_sync(
    db: &DatabaseConnection,
    job_name: &str,
    _default_interval_secs: i32,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let status = SyncStatus::find()
        .filter(sync_status::Column::JobName.eq(job_name))
        .one(db)
        .await?;

    match status {
        None => {
            // First run - create record and return true
            tracing::info!("[{}] First run detected, will sync", job_name);
            Ok(true)
        }
        Some(record) => {
            let min_interval = record.min_interval_secs;

            match record.last_success_at {
                None => {
                    // Never succeeded - should sync
                    tracing::info!("[{}] No previous successful sync, will sync", job_name);
                    Ok(true)
                }
                Some(last_success) => {
                    let now = Utc::now().naive_utc();
                    let elapsed = now.signed_duration_since(last_success);
                    let interval = Duration::seconds(min_interval as i64);

                    if elapsed >= interval {
                        tracing::info!(
                            "[{}] Last sync was {}s ago (min: {}s), will sync",
                            job_name,
                            elapsed.num_seconds(),
                            min_interval
                        );
                        Ok(true)
                    } else {
                        let remaining = (interval - elapsed).num_seconds();
                        tracing::info!(
                            "[{}] Skipping sync - last sync was {}s ago, next sync in {}s",
                            job_name,
                            elapsed.num_seconds(),
                            remaining
                        );
                        Ok(false)
                    }
                }
            }
        }
    }
}

/// Record a successful sync
pub async fn record_success(
    db: &DatabaseConnection,
    job_name: &str,
    default_interval_secs: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let now = Utc::now().naive_utc();

    let existing = SyncStatus::find()
        .filter(sync_status::Column::JobName.eq(job_name))
        .one(db)
        .await?;

    match existing {
        Some(record) => {
            let mut active_model: sync_status::ActiveModel = record.into();
            active_model.last_success_at = Set(Some(now));
            active_model.last_attempt_at = Set(Some(now));
            active_model.last_error = Set(None);
            active_model.success_count = Set(active_model.success_count.unwrap() + 1);
            active_model.update(db).await?;
        }
        None => {
            let new_record = sync_status::ActiveModel {
                job_name: Set(job_name.to_string()),
                last_success_at: Set(Some(now)),
                last_attempt_at: Set(Some(now)),
                last_error: Set(None),
                success_count: Set(1),
                error_count: Set(0),
                min_interval_secs: Set(default_interval_secs),
                ..Default::default()
            };
            new_record.insert(db).await?;
        }
    }

    tracing::debug!("[{}] Recorded successful sync", job_name);
    Ok(())
}

/// Record a failed sync attempt
pub async fn record_failure(
    db: &DatabaseConnection,
    job_name: &str,
    error: &str,
    default_interval_secs: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let now = Utc::now().naive_utc();

    let existing = SyncStatus::find()
        .filter(sync_status::Column::JobName.eq(job_name))
        .one(db)
        .await?;

    match existing {
        Some(record) => {
            let mut active_model: sync_status::ActiveModel = record.into();
            active_model.last_attempt_at = Set(Some(now));
            active_model.last_error = Set(Some(error.to_string()));
            active_model.error_count = Set(active_model.error_count.unwrap() + 1);
            active_model.update(db).await?;
        }
        None => {
            let new_record = sync_status::ActiveModel {
                job_name: Set(job_name.to_string()),
                last_success_at: Set(None),
                last_attempt_at: Set(Some(now)),
                last_error: Set(Some(error.to_string())),
                success_count: Set(0),
                error_count: Set(1),
                min_interval_secs: Set(default_interval_secs),
                ..Default::default()
            };
            new_record.insert(db).await?;
        }
    }

    tracing::debug!("[{}] Recorded failed sync: {}", job_name, error);
    Ok(())
}

/// Update the minimum interval for a job
pub async fn set_min_interval(
    db: &DatabaseConnection,
    job_name: &str,
    interval_secs: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let existing = SyncStatus::find()
        .filter(sync_status::Column::JobName.eq(job_name))
        .one(db)
        .await?;

    match existing {
        Some(record) => {
            let mut active_model: sync_status::ActiveModel = record.into();
            active_model.min_interval_secs = Set(interval_secs);
            active_model.update(db).await?;
        }
        None => {
            let new_record = sync_status::ActiveModel {
                job_name: Set(job_name.to_string()),
                min_interval_secs: Set(interval_secs),
                ..Default::default()
            };
            new_record.insert(db).await?;
        }
    }

    tracing::info!("[{}] Set min interval to {}s", job_name, interval_secs);
    Ok(())
}
