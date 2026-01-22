//! ITP Price Downsampler Job
//!
//! Runs daily to aggregate and clean up old price history data.
//! Supports graceful shutdown via SIGTERM/SIGINT signals.

use sea_orm::DatabaseConnection;
use std::env;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{error, info};

use crate::services::itp_price_downsampler::ItpPriceDownsampler;

/// Default downsampling interval in seconds (24 hours)
const DEFAULT_DOWNSAMPLE_INTERVAL_SECS: u64 = 86400;

/// Environment variable for downsampling interval
const ENV_DOWNSAMPLE_INTERVAL: &str = "ITP_DOWNSAMPLE_INTERVAL_SECS";

/// Environment variable for dry run mode
const ENV_DRY_RUN: &str = "ITP_DOWNSAMPLE_DRY_RUN";

/// Start the ITP price downsampler job
///
/// Spawns a background task that:
/// 1. Runs daily (configurable)
/// 2. Aggregates 5-min data older than 7 days to hourly
/// 3. Aggregates hourly data older than 30 days to daily
/// 4. Cleans up aggregated raw data
///
/// # Arguments
///
/// * `db` - Database connection
///
/// # Environment Variables
///
/// * `ITP_DOWNSAMPLE_INTERVAL_SECS` - Interval in seconds (default: 86400 = 24 hours)
/// * `ITP_DOWNSAMPLE_DRY_RUN` - Set to "true" for logging only mode
pub async fn start_itp_price_downsampler_job(db: DatabaseConnection) {
    tokio::spawn(async move {
        let downsample_interval_secs: u64 = env::var(ENV_DOWNSAMPLE_INTERVAL)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DOWNSAMPLE_INTERVAL_SECS);

        let dry_run = env::var(ENV_DRY_RUN)
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        info!(
            downsample_interval_secs = downsample_interval_secs,
            dry_run = dry_run,
            "Initializing ITP price downsampler job"
        );

        let downsampler = ItpPriceDownsampler::new(db);

        info!("ITP price downsampler job started successfully");

        // Run downsampling loop with graceful shutdown support
        let mut interval = interval(TokioDuration::from_secs(downsample_interval_secs));

        loop {
            tokio::select! {
                // Handle shutdown signal gracefully
                _ = tokio::signal::ctrl_c() => {
                    info!("Shutdown signal received, stopping ITP price downsampler job gracefully");
                    break;
                }
                // Normal interval tick
                _ = interval.tick() => {
                    info!("Starting ITP price downsampling");

                    if dry_run {
                        info!("DRY RUN: Skipping actual downsampling");
                        continue;
                    }

                    match downsampler.run_downsampling().await {
                        Ok(stats) => {
                            info!(
                                hourly_aggregated = stats.hourly_aggregated,
                                daily_aggregated = stats.daily_aggregated,
                                five_min_deleted = stats.five_min_deleted,
                                hourly_deleted = stats.hourly_deleted,
                                "ITP price downsampling completed"
                            );
                        }
                        Err(e) => {
                            error!(error = %e, "ITP price downsampling failed");
                            // Continue - next interval will retry
                        }
                    }
                }
            }
        }

        info!("ITP price downsampler job stopped");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_interval() {
        assert_eq!(DEFAULT_DOWNSAMPLE_INTERVAL_SECS, 86400);
    }

    #[test]
    fn test_env_var_names() {
        assert_eq!(ENV_DOWNSAMPLE_INTERVAL, "ITP_DOWNSAMPLE_INTERVAL_SECS");
        assert_eq!(ENV_DRY_RUN, "ITP_DOWNSAMPLE_DRY_RUN");
    }
}
