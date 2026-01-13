use chrono::{NaiveDate, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use tokio::time::{interval, Duration};

use crate::entities::{rebalances, prelude::*};
use crate::services::coingecko::CoinGeckoService;
use crate::services::exchange_api::ExchangeApiService;
use crate::services::rebalancing::{RebalancingService, RebalanceReason};

pub async fn start_rebalance_sync_job(
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
    exchange_api: ExchangeApiService,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every day

        let rebalancing_service = RebalancingService::new(
            db.clone(),
            coingecko,
            Some(exchange_api), // Pass exchange_api for scheduled rebalances
        );

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled rebalancing check");

            if let Err(e) = check_and_rebalance(&db, &rebalancing_service).await {
                tracing::error!("Failed to check and rebalance: {}", e);
            }
        }
    });
}

async fn check_and_rebalance(
    db: &DatabaseConnection,
    rebalancing_service: &RebalancingService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get all indexes
    let indexes = IndexMetadata::find().all(db).await?;

    let current_time = Utc::now().timestamp();

    for index in indexes {
        // CRITICAL: Skip manual indexes (skip_backfill = true)
        // These indexes are manually managed and should NOT have automatic rebalancing
        if index.skip_backfill {
            tracing::debug!(
                "Skipping index {} - manual index (skip_backfill=true)",
                index.index_id
            );
            continue;
        }

        // Check if index has rebalancing configured
        let rebalance_period = match index.rebalance_period {
            Some(period) => period,
            None => continue, // Skip indexes without rebalance period
        };

        // Check if index has initial_date configured
        let initial_date = match index.initial_date {
            Some(date) => date,
            None => {
                tracing::warn!("Index {} has no initial_date, skipping", index.index_id);
                continue;
            }
        };

        // Calculate expected number of rebalances from initial_date to today
        let today = Utc::now().date_naive();
        let expected_rebalances = calculate_expected_rebalances(
            initial_date,
            rebalance_period,
            today,
        );

        // Count actual rebalances in database
        let actual_rebalances = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .count(db)
            .await? as usize;

        // Check if backfill is incomplete
        if actual_rebalances < expected_rebalances {
            tracing::warn!(
                "Index {} has incomplete backfill: {} of {} rebalances. Starting backfill...",
                index.index_id,
                actual_rebalances,
                expected_rebalances
            );
            
            match rebalancing_service.backfill_historical_rebalances(index.index_id).await {
                Ok(_) => {
                    tracing::info!(
                        "Successfully completed backfill for index {} ({} rebalances)",
                        index.index_id,
                        expected_rebalances
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to backfill index {}: {}", index.index_id, e);
                }
            }
            
            // Continue to next index after backfill
            continue;
        }

        // Get last rebalance
        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index.index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(db)
            .await?;

        let last_rebalance = match last_rebalance {
            Some(rb) => rb,
            None => {
                tracing::warn!("No last rebalance found for index {} despite count check", index.index_id);
                continue;
            }
        };

        // CONDITION 1: Check if rebalance period is met
        let time_since_last = current_time - last_rebalance.timestamp;
        let period_seconds = (rebalance_period as i64) * 86400;
        let period_met = time_since_last >= period_seconds;

        // CONDITION 2: Check if any constituent is delisted
        let delisting_detected = check_for_delistings(
            db,
            rebalancing_service,
            &last_rebalance,
            index.index_id,
        )
        .await?;

        // TRIGGER REBALANCE IF: period met OR delisting detected
        let needs_rebalance = period_met || delisting_detected;

        if needs_rebalance {
            let reason = if delisting_detected {
                tracing::warn!(
                    "Delisting detected for index {} - triggering immediate rebalance",
                    index.index_id
                );
                RebalanceReason::Delisting("constituent_delisted".to_string())
            } else {
                tracing::info!("Index {} needs periodic rebalancing", index.index_id);
                RebalanceReason::Periodic
            };

            let current_date = Utc::now().date_naive();
            
            match rebalancing_service
                .perform_rebalance_for_date(index.index_id, current_date, reason)
                .await
            {
                Ok(_) => tracing::info!("Successfully rebalanced index {}", index.index_id),
                Err(e) => {
                    tracing::error!("Failed to rebalance index {}: {}", index.index_id, e);
                    tracing::error!("Skipping this rebalance cycle due to error. Will retry later.");
                }
            }

            // Add delay before next index
            tokio::time::sleep(Duration::from_millis(5000)).await;
        } else {
            tracing::debug!(
                "Index {} does not need rebalancing (time since last: {}s / {}s, delisting: {})",
                index.index_id,
                time_since_last,
                period_seconds,
                delisting_detected
            );
        }
    }

    Ok(())
}

/// Check if any constituent from last rebalance is delisted (not tradeable anymore)
async fn check_for_delistings(
    db: &DatabaseConnection,
    rebalancing_service: &RebalancingService,
    last_rebalance: &rebalances::Model,
    index_id: i32,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Get exchange API (only available in live mode)
    let exchange_api = match &rebalancing_service.exchange_api() {
        Some(api) => api,
        None => {
            // No exchange API available (shouldn't happen in scheduled mode, but handle gracefully)
            tracing::debug!("Exchange API not available - skipping delisting check for index {}", index_id);
            return Ok(false);
        }
    };

    // Parse constituents from last rebalance
    let constituents: Vec<crate::services::rebalancing::CoinRebalanceInfo> =
        serde_json::from_value(last_rebalance.coins.clone())?;

    if constituents.is_empty() {
        return Ok(false);
    }

    tracing::debug!(
        "Checking {} constituents for delistings in index {}",
        constituents.len(),
        index_id
    );

    // Check each constituent
    for constituent in &constituents {
        // Priority order: Binance USDC > USDT > Bitget USDC > USDT
        let exchanges_to_check = [
            ("binance", "usdc"),
            ("binance", "usdt"),
            ("bitget", "usdc"),
            ("bitget", "usdt"),
        ];

        let mut found_tradeable = false;

        for (exchange, pair) in exchanges_to_check {
            let is_tradeable = exchange_api
                .is_pair_tradeable(exchange, &constituent.symbol, pair)
                .await?;

            if is_tradeable {
                found_tradeable = true;
                break;
            }
        }

        if !found_tradeable {
            tracing::warn!(
                "🚨 DELISTING DETECTED: {} ({}) is no longer tradeable on any exchange for index {}",
                constituent.symbol,
                constituent.coin_id,
                index_id
            );
            return Ok(true); // Found at least one delisting
        }
    }

    tracing::debug!("✅ All constituents still tradeable for index {}", index_id);
    Ok(false)
}

/// Calculate expected number of rebalances from initial_date to current_date
fn calculate_expected_rebalances(
    initial_date: NaiveDate,
    period_days: i32,
    current_date: NaiveDate,
) -> usize {
    if current_date < initial_date {
        return 0;
    }

    let total_days = (current_date - initial_date).num_days();
    let num_periods = (total_days / period_days as i64) as usize;
    
    // +1 for the initial rebalance
    num_periods + 1
}