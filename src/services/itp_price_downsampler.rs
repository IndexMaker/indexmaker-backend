//! ITP Price Downsampler Service
//!
//! Aggregates and cleans up old price history data:
//! - 5-min data older than 7 days → aggregate to hourly
//! - Hourly data older than 30 days → aggregate to daily
//! - Delete aggregated raw data

use chrono::{Duration, Timelike, Utc};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    Order,
};
use std::collections::HashMap;
use tracing::{info, warn};

use crate::entities::{itp_price_history, prelude::ItpPriceHistory};

/// Error types for downsampler
#[derive(Debug)]
pub enum DownsamplerError {
    DatabaseError(String),
}

impl std::fmt::Display for DownsamplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownsamplerError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for DownsamplerError {}

/// ITP Price Downsampler Service
pub struct ItpPriceDownsampler {
    db: DatabaseConnection,
}

impl ItpPriceDownsampler {
    /// Create a new downsampler
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Run the full downsampling process
    ///
    /// 1. Aggregate 5-min data older than 7 days to hourly
    /// 2. Aggregate hourly data older than 30 days to daily
    /// 3. Delete old raw data that has been aggregated
    pub async fn run_downsampling(&self) -> Result<DownsampleStats, DownsamplerError> {
        let now = Utc::now();
        info!(timestamp = %now, "Starting price history downsampling");

        let mut stats = DownsampleStats::default();

        // Step 1: Aggregate 5-min to hourly (data older than 7 days)
        let hourly_cutoff = now - Duration::days(7);
        match self.aggregate_to_hourly(hourly_cutoff).await {
            Ok((aggregated, deleted)) => {
                stats.hourly_aggregated = aggregated;
                stats.five_min_deleted = deleted;
                info!(
                    aggregated = aggregated,
                    deleted = deleted,
                    "5-min to hourly aggregation complete"
                );
            }
            Err(e) => {
                warn!(error = %e, "5-min to hourly aggregation failed");
            }
        }

        // Step 2: Aggregate hourly to daily (data older than 30 days)
        let daily_cutoff = now - Duration::days(30);
        match self.aggregate_to_daily(daily_cutoff).await {
            Ok((aggregated, deleted)) => {
                stats.daily_aggregated = aggregated;
                stats.hourly_deleted = deleted;
                info!(
                    aggregated = aggregated,
                    deleted = deleted,
                    "Hourly to daily aggregation complete"
                );
            }
            Err(e) => {
                warn!(error = %e, "Hourly to daily aggregation failed");
            }
        }

        info!(
            hourly_aggregated = stats.hourly_aggregated,
            daily_aggregated = stats.daily_aggregated,
            five_min_deleted = stats.five_min_deleted,
            hourly_deleted = stats.hourly_deleted,
            "Downsampling complete"
        );

        Ok(stats)
    }

    /// Aggregate 5-min data to hourly
    ///
    /// Returns (records_created, records_deleted)
    async fn aggregate_to_hourly(
        &self,
        cutoff: chrono::DateTime<Utc>,
    ) -> Result<(usize, usize), DownsamplerError> {
        // Find all 5-min records older than cutoff
        let old_records = ItpPriceHistory::find()
            .filter(itp_price_history::Column::Granularity.eq("5min"))
            .filter(itp_price_history::Column::Timestamp.lt(cutoff.fixed_offset()))
            .order_by(itp_price_history::Column::Timestamp, Order::Asc)
            .all(&self.db)
            .await
            .map_err(|e| DownsamplerError::DatabaseError(format!("Query failed: {}", e)))?;

        if old_records.is_empty() {
            return Ok((0, 0));
        }

        // Group by itp_id and hour
        let mut hourly_groups: HashMap<(String, String), Vec<itp_price_history::Model>> =
            HashMap::new();

        for record in &old_records {
            // Truncate timestamp to hour
            let hour_key = record
                .timestamp
                .format("%Y-%m-%d %H:00:00")
                .to_string();
            let key = (record.itp_id.clone(), hour_key);
            hourly_groups.entry(key).or_default().push(record.clone());
        }

        // Create hourly aggregates
        let mut created = 0;
        for ((itp_id, _hour), records) in hourly_groups {
            if records.is_empty() {
                continue;
            }

            // Calculate aggregates
            let avg_price = self.average_price(&records);
            let total_volume = self.sum_volume(&records);
            // Truncate timestamp to hour boundary (e.g., 10:03:45 -> 10:00:00)
            let first_ts = records[0].timestamp;
            let truncated_ts = first_ts
                .with_minute(0)
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .unwrap_or(first_ts);
            let timestamp = truncated_ts;

            // Check if hourly record already exists
            let existing = ItpPriceHistory::find()
                .filter(itp_price_history::Column::ItpId.eq(&itp_id))
                .filter(itp_price_history::Column::Timestamp.eq(timestamp))
                .filter(itp_price_history::Column::Granularity.eq("hourly"))
                .one(&self.db)
                .await
                .map_err(|e| DownsamplerError::DatabaseError(format!("Query failed: {}", e)))?;

            if existing.is_none() {
                let hourly_record = itp_price_history::ActiveModel {
                    itp_id: Set(itp_id),
                    price: Set(avg_price),
                    volume: Set(total_volume),
                    timestamp: Set(timestamp),
                    granularity: Set("hourly".to_string()),
                    ..Default::default()
                };

                hourly_record.insert(&self.db).await.map_err(|e| {
                    DownsamplerError::DatabaseError(format!("Insert failed: {}", e))
                })?;

                created += 1;
            }
        }

        // Delete old 5-min records in batch (much faster than one-by-one)
        let record_ids: Vec<i64> = old_records.iter().map(|r| r.id).collect();
        let deleted = record_ids.len();

        if !record_ids.is_empty() {
            itp_price_history::Entity::delete_many()
                .filter(itp_price_history::Column::Id.is_in(record_ids))
                .exec(&self.db)
                .await
                .map_err(|e| DownsamplerError::DatabaseError(format!("Batch delete failed: {}", e)))?;
        }

        Ok((created, deleted))
    }

    /// Aggregate hourly data to daily
    ///
    /// Returns (records_created, records_deleted)
    async fn aggregate_to_daily(
        &self,
        cutoff: chrono::DateTime<Utc>,
    ) -> Result<(usize, usize), DownsamplerError> {
        // Find all hourly records older than cutoff
        let old_records = ItpPriceHistory::find()
            .filter(itp_price_history::Column::Granularity.eq("hourly"))
            .filter(itp_price_history::Column::Timestamp.lt(cutoff.fixed_offset()))
            .order_by(itp_price_history::Column::Timestamp, Order::Asc)
            .all(&self.db)
            .await
            .map_err(|e| DownsamplerError::DatabaseError(format!("Query failed: {}", e)))?;

        if old_records.is_empty() {
            return Ok((0, 0));
        }

        // Group by itp_id and day
        let mut daily_groups: HashMap<(String, String), Vec<itp_price_history::Model>> =
            HashMap::new();

        for record in &old_records {
            let day_key = record.timestamp.format("%Y-%m-%d").to_string();
            let key = (record.itp_id.clone(), day_key);
            daily_groups.entry(key).or_default().push(record.clone());
        }

        // Create daily aggregates
        let mut created = 0;
        for ((itp_id, _day), records) in daily_groups {
            if records.is_empty() {
                continue;
            }

            let avg_price = self.average_price(&records);
            let total_volume = self.sum_volume(&records);
            // Truncate timestamp to day boundary (e.g., 10:30:00 -> 00:00:00)
            let first_ts = records[0].timestamp;
            let truncated_ts = first_ts
                .with_hour(0)
                .and_then(|t| t.with_minute(0))
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .unwrap_or(first_ts);
            let timestamp = truncated_ts;

            // Check if daily record already exists
            let existing = ItpPriceHistory::find()
                .filter(itp_price_history::Column::ItpId.eq(&itp_id))
                .filter(itp_price_history::Column::Timestamp.eq(timestamp))
                .filter(itp_price_history::Column::Granularity.eq("daily"))
                .one(&self.db)
                .await
                .map_err(|e| DownsamplerError::DatabaseError(format!("Query failed: {}", e)))?;

            if existing.is_none() {
                let daily_record = itp_price_history::ActiveModel {
                    itp_id: Set(itp_id),
                    price: Set(avg_price),
                    volume: Set(total_volume),
                    timestamp: Set(timestamp),
                    granularity: Set("daily".to_string()),
                    ..Default::default()
                };

                daily_record.insert(&self.db).await.map_err(|e| {
                    DownsamplerError::DatabaseError(format!("Insert failed: {}", e))
                })?;

                created += 1;
            }
        }

        // Delete old hourly records in batch (much faster than one-by-one)
        let record_ids: Vec<i64> = old_records.iter().map(|r| r.id).collect();
        let deleted = record_ids.len();

        if !record_ids.is_empty() {
            itp_price_history::Entity::delete_many()
                .filter(itp_price_history::Column::Id.is_in(record_ids))
                .exec(&self.db)
                .await
                .map_err(|e| DownsamplerError::DatabaseError(format!("Batch delete failed: {}", e)))?;
        }

        Ok((created, deleted))
    }

    /// Calculate average price from records
    fn average_price(&self, records: &[itp_price_history::Model]) -> Decimal {
        if records.is_empty() {
            return Decimal::ZERO;
        }

        let sum: Decimal = records.iter().map(|r| r.price).sum();
        sum / Decimal::from(records.len())
    }

    /// Sum volumes from records
    fn sum_volume(&self, records: &[itp_price_history::Model]) -> Option<Decimal> {
        let volumes: Vec<Decimal> = records.iter().filter_map(|r| r.volume).collect();

        if volumes.is_empty() {
            None
        } else {
            Some(volumes.iter().sum())
        }
    }
}

/// Statistics from downsampling run
#[derive(Debug, Default)]
pub struct DownsampleStats {
    pub hourly_aggregated: usize,
    pub daily_aggregated: usize,
    pub five_min_deleted: usize,
    pub hourly_deleted: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downsample_stats_default() {
        let stats = DownsampleStats::default();
        assert_eq!(stats.hourly_aggregated, 0);
        assert_eq!(stats.daily_aggregated, 0);
        assert_eq!(stats.five_min_deleted, 0);
        assert_eq!(stats.hourly_deleted, 0);
    }

    #[test]
    fn test_error_display() {
        let err = DownsamplerError::DatabaseError("test".to_string());
        assert!(err.to_string().contains("Database error"));
    }
}
