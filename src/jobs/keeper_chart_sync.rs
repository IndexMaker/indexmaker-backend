//! Keeper Chart Sync Job
//!
//! Periodically polls Orbit VAULT contract for keeper claimable data
//! and stores time-series data for chart rendering.

use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use serde_json::json;
use std::env;
use tokio::time::{interval, Duration as TokioDuration};

use crate::entities::keeper_claimable_data;
use crate::services::orbit_keeper::{OrbitKeeperService, KeeperClaimableResult};

/// Default polling interval in seconds (5 minutes)
const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;

/// Environment variable for Orbit RPC URL
const ENV_ORBIT_RPC_URL: &str = "ORBIT_RPC_URL";

/// Environment variable for keeper addresses (comma-separated)
const ENV_KEEPER_ADDRESSES: &str = "KEEPER_ADDRESSES";

/// Environment variable for polling interval
const ENV_POLL_INTERVAL: &str = "KEEPER_POLL_INTERVAL_SECS";

/// Environment variable for dry run mode (logging only, no DB persistence)
const ENV_DRY_RUN: &str = "KEEPER_DRY_RUN";

/// Start the keeper chart sync job
///
/// Spawns a background task that:
/// 1. Connects to Orbit RPC
/// 2. Polls keeper claimable data at configured interval
/// 3. Stores results in database
///
/// # Arguments
///
/// * `db` - Database connection
///
/// # Environment Variables
///
/// * `ORBIT_RPC_URL` - Orbit chain RPC URL (required)
/// * `KEEPER_ADDRESSES` - Comma-separated list of keeper addresses (required)
/// * `KEEPER_POLL_INTERVAL_SECS` - Polling interval in seconds (default: 300)
/// * `KEEPER_DRY_RUN` - Set to "true" for logging only mode
pub async fn start_keeper_chart_sync_job(db: DatabaseConnection) {
    tokio::spawn(async move {
        // Get configuration from environment
        let rpc_url = match env::var(ENV_ORBIT_RPC_URL) {
            Ok(url) => url,
            Err(_) => {
                tracing::warn!(
                    "ORBIT_RPC_URL not set - keeper chart sync job disabled. \
                     Set ORBIT_RPC_URL to enable."
                );
                return;
            }
        };

        let keeper_addresses: Vec<String> = match env::var(ENV_KEEPER_ADDRESSES) {
            Ok(addrs) => addrs
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Err(_) => {
                tracing::warn!(
                    "KEEPER_ADDRESSES not set - keeper chart sync job disabled. \
                     Set KEEPER_ADDRESSES to comma-separated list of keeper addresses."
                );
                return;
            }
        };

        if keeper_addresses.is_empty() {
            tracing::warn!("No keeper addresses configured - keeper chart sync job disabled");
            return;
        }

        let poll_interval_secs: u64 = env::var(ENV_POLL_INTERVAL)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECS);

        let dry_run = env::var(ENV_DRY_RUN)
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        tracing::info!(
            rpc_url = %rpc_url,
            keeper_count = keeper_addresses.len(),
            poll_interval_secs = poll_interval_secs,
            dry_run = dry_run,
            "Initializing keeper chart sync job"
        );

        // Initialize Orbit service
        let orbit_service = match OrbitKeeperService::new(&rpc_url).await {
            Ok(service) => service,
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize OrbitKeeperService");
                return;
            }
        };

        tracing::info!("Keeper chart sync job started successfully");

        // Run sync loop
        let mut interval = interval(TokioDuration::from_secs(poll_interval_secs));

        loop {
            interval.tick().await;

            tracing::info!(
                keeper_count = keeper_addresses.len(),
                "Starting keeper claimable data sync"
            );

            let mut success_count = 0;
            let mut error_count = 0;

            // Process each keeper
            for keeper_address in &keeper_addresses {
                match sync_keeper_data(&db, &orbit_service, keeper_address, dry_run).await {
                    Ok(_) => {
                        success_count += 1;
                    }
                    Err(e) => {
                        error_count += 1;
                        tracing::error!(
                            keeper_address = %keeper_address,
                            error = %e,
                            "Failed to sync keeper data"
                        );
                        // Continue with other keepers - one failure shouldn't stop all
                    }
                }
            }

            tracing::info!(
                success_count = success_count,
                error_count = error_count,
                total = keeper_addresses.len(),
                "Keeper claimable data sync complete"
            );
        }
    });
}

/// Sync data for a single keeper
async fn sync_keeper_data(
    db: &DatabaseConnection,
    orbit_service: &OrbitKeeperService,
    keeper_address: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let result = orbit_service.get_claimable_data(keeper_address).await?;

    let recorded_at = Utc::now().naive_utc();

    tracing::info!(
        keeper_address = %keeper_address,
        acquisition_1 = result.acquisition_value_1,
        acquisition_2 = result.acquisition_value_2,
        disposal_1 = result.disposal_value_1,
        disposal_2 = result.disposal_value_2,
        recorded_at = %recorded_at,
        dry_run = dry_run,
        "Fetched keeper claimable data"
    );

    if dry_run {
        tracing::debug!(
            keeper_address = %keeper_address,
            "DRY RUN: Skipping database persistence"
        );
        return Ok(());
    }

    // Store in database
    store_keeper_data(db, &result, recorded_at).await?;

    Ok(())
}

/// Store keeper claimable data in database
async fn store_keeper_data(
    db: &DatabaseConnection,
    result: &KeeperClaimableResult,
    recorded_at: chrono::NaiveDateTime,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Convert u128 to Decimal for storage
    let acq_1 = Decimal::from(result.acquisition_value_1);
    let acq_2 = Decimal::from(result.acquisition_value_2);
    let disp_1 = Decimal::from(result.disposal_value_1);
    let disp_2 = Decimal::from(result.disposal_value_2);

    // Store raw values for debugging
    let raw_response = json!({
        "acquisition": [result.acquisition_value_1.to_string(), result.acquisition_value_2.to_string()],
        "disposal": [result.disposal_value_1.to_string(), result.disposal_value_2.to_string()],
    });

    let new_record = keeper_claimable_data::ActiveModel {
        keeper_address: Set(result.keeper_address.clone()),
        recorded_at: Set(recorded_at),
        acquisition_value_1: Set(acq_1),
        acquisition_value_2: Set(acq_2),
        disposal_value_1: Set(disp_1),
        disposal_value_2: Set(disp_2),
        raw_response: Set(Some(raw_response)),
        created_at: Set(Some(recorded_at)),
    };

    new_record.insert(db).await?;

    tracing::debug!(
        keeper_address = %result.keeper_address,
        recorded_at = %recorded_at,
        "Stored keeper claimable data"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_poll_interval() {
        assert_eq!(DEFAULT_POLL_INTERVAL_SECS, 300);
    }

    #[test]
    fn test_env_var_names() {
        assert_eq!(ENV_ORBIT_RPC_URL, "ORBIT_RPC_URL");
        assert_eq!(ENV_KEEPER_ADDRESSES, "KEEPER_ADDRESSES");
        assert_eq!(ENV_POLL_INTERVAL, "KEEPER_POLL_INTERVAL_SECS");
        assert_eq!(ENV_DRY_RUN, "KEEPER_DRY_RUN");
    }

    #[test]
    fn test_u128_to_decimal_conversion() {
        let value: u128 = 1_000_000_000_000_000_000; // 1e18
        let decimal = Decimal::from(value);
        assert!(decimal > Decimal::ZERO);
    }
}
