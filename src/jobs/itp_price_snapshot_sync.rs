//! ITP Price Snapshot Sync Job
//!
//! Periodically snapshots ITP prices from the Orbit chain
//! and stores time-series data in itp_price_history table.
//! Supports graceful shutdown via SIGTERM/SIGINT signals.

use sea_orm::DatabaseConnection;
use std::env;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{error, info, warn};

use crate::services::itp_price_snapshot::ItpPriceSnapshotService;

/// Default snapshot interval in seconds (5 minutes)
const DEFAULT_SNAPSHOT_INTERVAL_SECS: u64 = 300;

/// Environment variable for Orbit RPC URL
const ENV_ORBIT_RPC_URL: &str = "ORBIT_RPC_URL";

/// Environment variable for Castle contract address
const ENV_CASTLE_ADDRESS: &str = "CASTLE_ADDRESS";

/// Environment variable for snapshot interval
const ENV_SNAPSHOT_INTERVAL: &str = "ITP_PRICE_SNAPSHOT_INTERVAL_SECS";

/// Environment variable for dry run mode
const ENV_DRY_RUN: &str = "ITP_PRICE_SNAPSHOT_DRY_RUN";

/// Start the ITP price snapshot sync job
///
/// Spawns a background task that:
/// 1. Connects to Orbit RPC
/// 2. Polls active ITP prices at configured interval (default: 5 minutes)
/// 3. Stores snapshots in itp_price_history table
///
/// # Arguments
///
/// * `db` - Database connection
///
/// # Environment Variables
///
/// * `ORBIT_RPC_URL` - Orbit chain RPC URL (required)
/// * `CASTLE_ADDRESS` - Castle contract address (required)
/// * `ITP_PRICE_SNAPSHOT_INTERVAL_SECS` - Interval in seconds (default: 300)
/// * `ITP_PRICE_SNAPSHOT_DRY_RUN` - Set to "true" for logging only mode
pub async fn start_itp_price_snapshot_job(db: DatabaseConnection) {
    tokio::spawn(async move {
        // Get configuration from environment
        let rpc_url = match env::var(ENV_ORBIT_RPC_URL) {
            Ok(url) => url,
            Err(_) => {
                warn!(
                    "ORBIT_RPC_URL not set - ITP price snapshot job disabled. \
                     Set ORBIT_RPC_URL to enable."
                );
                return;
            }
        };

        let castle_address = match env::var(ENV_CASTLE_ADDRESS) {
            Ok(addr) => addr,
            Err(_) => {
                warn!(
                    "CASTLE_ADDRESS not set - ITP price snapshot job disabled. \
                     Set CASTLE_ADDRESS to enable."
                );
                return;
            }
        };

        let snapshot_interval_secs: u64 = env::var(ENV_SNAPSHOT_INTERVAL)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SNAPSHOT_INTERVAL_SECS);

        let dry_run = env::var(ENV_DRY_RUN)
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        info!(
            rpc_url = %rpc_url,
            castle_address = %castle_address,
            snapshot_interval_secs = snapshot_interval_secs,
            dry_run = dry_run,
            "Initializing ITP price snapshot job"
        );

        // Initialize snapshot service
        let snapshot_service =
            match ItpPriceSnapshotService::new(db.clone(), &rpc_url, &castle_address).await {
                Ok(service) => service,
                Err(e) => {
                    error!(error = %e, "Failed to initialize ItpPriceSnapshotService");
                    return;
                }
            };

        info!("ITP price snapshot job started successfully");

        // Run snapshot loop with graceful shutdown support
        let mut interval = interval(TokioDuration::from_secs(snapshot_interval_secs));

        loop {
            tokio::select! {
                // Handle shutdown signal gracefully
                _ = tokio::signal::ctrl_c() => {
                    info!("Shutdown signal received, stopping ITP price snapshot job gracefully");
                    break;
                }
                // Normal interval tick
                _ = interval.tick() => {
                    info!("Starting ITP price snapshot sync");

                    if dry_run {
                        info!("DRY RUN: Skipping actual snapshot");
                        continue;
                    }

                    match snapshot_service.snapshot_all_itp_prices().await {
                        Ok(count) => {
                            info!(count = count, "ITP price snapshot completed");
                        }
                        Err(e) => {
                            error!(error = %e, "ITP price snapshot failed");
                            // Continue - next interval will retry
                        }
                    }
                }
            }
        }

        info!("ITP price snapshot job stopped");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_interval() {
        assert_eq!(DEFAULT_SNAPSHOT_INTERVAL_SECS, 300);
    }

    #[test]
    fn test_env_var_names() {
        assert_eq!(ENV_ORBIT_RPC_URL, "ORBIT_RPC_URL");
        assert_eq!(ENV_CASTLE_ADDRESS, "CASTLE_ADDRESS");
        assert_eq!(ENV_SNAPSHOT_INTERVAL, "ITP_PRICE_SNAPSHOT_INTERVAL_SECS");
    }
}
